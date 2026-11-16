//! An end-to-end check of the `.lbl` archive on this machine: V17 and V16 still open as bare files, a V18 build is created, sealed and read, the adapter's dispatch
//! sees the file version inside the archive, and a benchmark compares an HDF5 file read inside an archive with the same file read bare.
//!
//! `cargo run --release -p build-log-io --example lbl_check -- [bench-GiB]` (default 1; 0 skips the benchmark). Nothing is written outside the system temp folder, which is removed at the end.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use build_log_io::adapters::mock;
use build_log_io::container::{
    create_working_file, pack, seal, FileArchive, MemberSource, SealOptions, USERBLOCK,
};
use build_log_io::dispatch::{self, production, Registry};
use build_log_model::{ContainerKind, ErrorKind};
use build_log_testkit::{miniature, miniature_path};
use hdf5::File;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> Res<()> {
    let gib: f64 = std::env::args()
        .nth(1)
        .map(|a| a.parse())
        .transpose()?
        .unwrap_or(1.0);
    let dir = std::env::temp_dir().join(format!("lbl_check_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let result = run(&dir, gib);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run(dir: &Path, gib: f64) -> Res<()> {
    let mut failures = 0;
    let mut check = |what: &str, ok: bool| {
        println!("  [{}] {what}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failures += 1;
        }
    };

    println!("\n== 1. V17 and V16 files are still supported (bare files, production registry)");
    for name in [
        miniature::V16,
        miniature::V17_MARCH,
        miniature::V17_MOCK,
        miniature::V17_OCT15,
        miniature::V17_TRIGGERED_2026_10_19,
        miniature::V17_2026_08_18_IMAGES,
        miniature::V17_SPECIAL_LAYERS,
    ] {
        let path = miniature_path(name);
        let t = Instant::now();
        match dispatch::open(production(), &path) {
            Ok(opened) => {
                let build = opened.source().build()?;
                let layers = opened.source().layers()?.len();
                println!(
                    "     {name:<34} version {:<3} adapter {:<4} {:?}  {layers} layers, {} lasers  ({:.1?})",
                    opened.key().as_str(),
                    opened.adapter(),
                    opened.container(),
                    build.lasers().len(),
                    t.elapsed()
                );
                check(
                    &format!("{name} opens as a bare file"),
                    opened.container() == ContainerKind::BareHdf5 && opened.archive().is_none(),
                );
            }
            Err(e) => check(&format!("{name} opens ({e})"), false),
        }
    }
    let v17 = miniature_path(miniature::V17_MARCH);
    let wrong = dir.join("v17_in_an_archive.lbl");
    pack(&v17, &wrong, &SealOptions::new())?;
    let refused = dispatch::open(production(), &wrong).err();
    check(
        "a V17 build inside an .lbl archive is refused (WrongContainer)",
        matches!(
            refused.as_ref().map(|e| e.kind()),
            Some(ErrorKind::WrongContainer { .. })
        ),
    );
    let renamed = dir.join("v17_named_lbl.lbl");
    std::fs::copy(&v17, &renamed)?;
    check(
        "a bare V17 file named .lbl is refused (WrongContainer)",
        matches!(
            dispatch::open(production(), &renamed)
                .err()
                .as_ref()
                .map(|e| e.kind()),
            Some(ErrorKind::WrongContainer { .. })
        ),
    );

    println!("\n== 2. A real V18 miniature, packed into an .lbl archive and read from inside it");
    let v18 = miniature_path(miniature::V18);
    let lbl = dir.join("v18.lbl");
    let t = Instant::now();
    let report = pack(
        &v18,
        &lbl,
        &SealOptions::new()
            .with_machine_config(MemberSource::Bytes(b"machine configuration".repeat(500)))
            .with_member("logs/run.log", MemberSource::Bytes(b"line\n".repeat(1000)))
            .with_member(
                "images/layer_1.png",
                MemberSource::Bytes(vec![0x89; 70_000]),
            )
            .with_schema_version(18),
    )?;
    println!(
        "     packed {} bytes of HDF5 behind a {USERBLOCK}-byte userblock in {:.1?}; the archive is {} bytes",
        std::fs::metadata(&v18)?.len(),
        t.elapsed(),
        std::fs::metadata(&lbl)?.len()
    );
    let _ = report;
    let archive = FileArchive::open_path(&lbl)?;
    for m in archive.members() {
        println!(
            "       member {:<22} at {:>9}  {:>9} bytes  stored={}",
            m.name, m.data_offset, m.size, m.stored
        );
    }
    check(
        "the HDF5 member starts at the userblock (4096)",
        archive.primary().data_offset == USERBLOCK,
    );
    let inside = archive.read_member(archive.primary(), u64::MAX)?;
    check(
        "the HDF5 member is byte-for-byte the file it came from",
        inside == std::fs::read(&v18)?,
    );
    check(
        "every member passes its CRC-32",
        archive.members().iter().all(|m| archive.verify(m).is_ok()),
    );
    check(
        "the machine configuration member is found without extraction",
        archive.machine_config().is_some(),
    );
    drop(archive);
    let packed = File::open(&lbl)?;
    let version: i32 = packed.group("Build")?.attr("File_Version")?.read_scalar()?;
    let build_group = packed.group("Build")?;
    let key = dispatch::peek_marker(&build_group, &["File_Version"])?;
    println!("     stock HDF5 opened the .lbl itself: Build@File_Version = {version}, marker key {key:?}");
    check(
        "the file version is read from inside the archive",
        version == 18,
    );
    let channel = "Build/Lasers/Laser_1/Monitoring_Data/Layer_1/Actual_X";
    let a: Vec<i32> = packed.dataset(channel)?.read_raw()?;
    let b: Vec<i32> = File::open(&v18)?.dataset(channel)?.read_raw()?;
    check(
        &format!(
            "a data channel reads the same inside and outside the archive ({} samples)",
            a.len()
        ),
        !a.is_empty() && a == b,
    );
    drop(packed);
    match dispatch::open(production(), &lbl) {
        Ok(o) => println!("     production registry opened it as version {}", o.key().as_str()),
        Err(e) => println!(
            "     production registry: {e}\n       (expected until the V18 adapter registers its marker and `Archive`: the container and the marker work, the layout mapping is IP-05)"
        ),
    }

    println!("\n== 3. Creating and reading a V18-style build (a mock adapter set that declares `Archive`)");
    let key = "9018";
    let registry = Registry::new().with(mock::adapter_set_in(key, ContainerKind::Archive)?)?;
    let created = dir.join("created.lbl");
    let t = Instant::now();
    {
        let file = create_working_file(&created)?;
        let mount = file.create_group(&mock::mount_for(key))?;
        mock::write_mock_file_for(&mount, key, "BUILD-18-DEMO", 3)?;
        let samples = file
            .new_dataset::<i32>()
            .chunk(10_000)
            .shape(0..)
            .create("Samples")?;
        for block in 0..20usize {
            samples.resize((block + 1) * 10_000)?;
            let values: Vec<i32> =
                ((block * 10_000) as i32..((block + 1) * 10_000) as i32).collect();
            samples.write_slice(&values, (block * 10_000)..((block + 1) * 10_000))?;
        }
    }
    println!(
        "     working file written in place at its final path ({:.1?})",
        t.elapsed()
    );
    let before = dispatch::open(&registry, &created).err();
    check(
        "before sealing it is `Unsealed`, not a damaged archive",
        matches!(
            before.as_ref().map(|e| e.kind()),
            Some(ErrorKind::Unsealed { .. })
        ),
    );
    let t = Instant::now();
    seal(
        &created,
        &SealOptions::new()
            .with_machine_config(MemberSource::Bytes(b"mcf".repeat(2000)))
            .with_member(
                "logs/capture.log",
                MemberSource::Bytes(b"started\n".repeat(50)),
            )
            .with_schema_version(18),
    )?;
    println!("     sealed in {:.1?}", t.elapsed());
    let opened = dispatch::open(&registry, &created)?;
    let source = opened.source();
    println!(
        "     opened through the registry: version {}, adapter {}, container {:?}, {} layers, members {:?}",
        opened.key().as_str(),
        opened.adapter(),
        opened.container(),
        source.layers()?.len(),
        opened.archive().map(|a| a.members().iter().map(|m| m.name.clone()).collect::<Vec<_>>())
    );
    check(
        "it opens as an archive",
        opened.container() == ContainerKind::Archive,
    );
    check(
        "the adapter reports the file version",
        opened.key().as_str() == key,
    );
    check(
        "the archive is unchanged and watched",
        opened.check_file().is_some(),
    );
    drop(opened);
    let back = File::open(&created)?;
    let samples: Vec<i32> = back.dataset("Samples")?.read_raw()?;
    check(
        "all 200000 samples written in place read back",
        samples == (0..200_000).collect::<Vec<i32>>(),
    );
    drop(back);
    let bare_registry =
        Registry::new().with(mock::adapter_set_in(key, ContainerKind::BareHdf5)?)?;
    check(
        "the same version registered as bare refuses the archive (WrongContainer)",
        matches!(
            dispatch::open(&bare_registry, &created)
                .err()
                .as_ref()
                .map(|e| e.kind()),
            Some(ErrorKind::WrongContainer { .. })
        ),
    );

    if gib > 0.0 {
        benchmark(dir, gib)?;
    }

    println!(
        "\n{}",
        if failures == 0 {
            "ALL CHECKS PASSED".to_string()
        } else {
            format!("{failures} CHECK(S) FAILED")
        }
    );
    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------ benchmark

const CHUNK: usize = 262_144; // 1 MiB of f32
const SLICE: usize = 65_536; // 256 KiB of f32, one random read

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn write_data(file: &File, total: usize) -> Res<Duration> {
    let t = Instant::now();
    let ds = file
        .new_dataset::<f32>()
        .chunk(CHUNK)
        .shape(total)
        .create("Samples")?;
    let step = 64 * CHUNK;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut buf = vec![0f32; step];
    let mut at = 0;
    while at < total {
        let n = step.min(total - at);
        for v in &mut buf[..n] {
            *v = (rng.next() >> 40) as f32;
        }
        ds.write_slice(&buf[..n], at..at + n)?;
        at += n;
    }
    Ok(t.elapsed())
}

fn mib_s(bytes: usize, d: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / d.as_secs_f64()
}

fn timed<T>(f: impl FnOnce() -> Res<T>) -> Res<(T, Duration)> {
    let t = Instant::now();
    let v = f()?;
    Ok((v, t.elapsed()))
}

fn read_all(path: &Path, total: usize) -> Res<Duration> {
    let ds = File::open(path)?.dataset("Samples")?;
    let step = 64 * CHUNK;
    let t = Instant::now();
    let mut at = 0;
    let mut sum = 0f64;
    while at < total {
        let n = step.min(total - at);
        let v: Vec<f32> = ds.read_slice_1d(at..at + n)?.to_vec();
        sum += v[0] as f64;
        at += n;
    }
    std::hint::black_box(sum);
    Ok(t.elapsed())
}

fn read_random(path: &Path, total: usize, count: usize) -> Res<Duration> {
    let ds = File::open(path)?.dataset("Samples")?;
    let mut rng = Rng(42);
    let t = Instant::now();
    let mut sum = 0f64;
    for _ in 0..count {
        let at = (rng.next() as usize) % (total - SLICE);
        let v = ds.read_slice_1d::<f32, _>(at..at + SLICE)?;
        sum += v[0] as f64;
    }
    std::hint::black_box(sum);
    Ok(t.elapsed())
}

fn best(runs: usize, mut f: impl FnMut() -> Res<Duration>) -> Res<Duration> {
    let mut best = Duration::MAX;
    for _ in 0..runs {
        best = best.min(f()?);
    }
    Ok(best)
}

fn benchmark(dir: &Path, gib: f64) -> Res<()> {
    let total = ((gib * 1024.0 * 1024.0 * 1024.0) as usize / 4 / CHUNK).max(1) * CHUNK;
    let bytes = total * 4;
    println!(
        "\n== 4. Benchmark: {:.2} GiB of f32, chunked 1 MiB, uncompressed; the same data as a bare file and inside an .lbl archive",
        bytes as f64 / (1u64 << 30) as f64
    );
    println!("     (best of 3; the files were just written, so the OS cache is warm: this measures the layers above the disk, not the disk)");
    let bare: PathBuf = dir.join("bench_bare.h5");
    let arch: PathBuf = dir.join("bench.lbl");
    let (w_bare, w_work) = if std::env::var_os("LBL_BENCH_SWAP").is_some() {
        let work = write_data(&create_working_file(&arch)?, total)?;
        (write_data(&File::create(&bare)?, total)?, work)
    } else {
        (
            write_data(&File::create(&bare)?, total)?,
            write_data(&create_working_file(&arch)?, total)?,
        )
    };
    let (_, seal_t) = timed(|| {
        Ok(seal(
            &arch,
            &SealOptions::new()
                .with_machine_config(MemberSource::Bytes(vec![7; 1 << 20]))
                .with_member("logs/run.log", MemberSource::Bytes(vec![b'x'; 1 << 20])),
        )?)
    })?;
    println!(
        "  write   bare file               {:>8.0} MiB/s  ({:.1?})",
        mib_s(bytes, w_bare),
        w_bare
    );
    println!("  write   working file (archive)  {:>8.0} MiB/s  ({:.1?})   <- same HDF5 writer, userblock only", mib_s(bytes, w_work), w_work);
    println!("  seal    CRC-32 pass + directory {:>8.0} MiB/s  ({:.1?})   <- one extra read of the data, once", mib_s(bytes, seal_t), seal_t);
    println!(
        "  size    bare {} bytes, archive {} bytes (+{} bytes)",
        std::fs::metadata(&bare)?.len(),
        std::fs::metadata(&arch)?.len(),
        std::fs::metadata(&arch)?.len() as i64 - std::fs::metadata(&bare)?.len() as i64
    );

    let open_bare = best(3, || {
        Ok(timed(|| Ok(File::open(&bare)?.dataset("Samples")?))?.1)
    })?;
    let open_arch = best(3, || {
        Ok(timed(|| Ok(File::open(&arch)?.dataset("Samples")?))?.1)
    })?;
    let dir_only = best(3, || Ok(timed(|| Ok(FileArchive::open_path(&arch)?))?.1))?;
    println!("  open    HDF5, bare              {:>8.2?}", open_bare);
    println!("  open    HDF5, inside archive    {:>8.2?}", open_arch);
    println!(
        "  open    archive directory only  {:>8.2?}   (cost is the same for 1 MB or 1 TB)",
        dir_only
    );

    let seq_bare = best(3, || read_all(&bare, total))?;
    let seq_arch = best(3, || read_all(&arch, total))?;
    println!(
        "  read    sequential, bare        {:>8.0} MiB/s",
        mib_s(bytes, seq_bare)
    );
    println!(
        "  read    sequential, archive     {:>8.0} MiB/s  ({:+.1}%)",
        mib_s(bytes, seq_arch),
        (seq_bare.as_secs_f64() / seq_arch.as_secs_f64() - 1.0) * 100.0
    );

    let count = 1000;
    let slice_bytes = count * SLICE * 4;
    let rnd_bare = best(3, || read_random(&bare, total, count))?;
    let rnd_arch = best(3, || read_random(&arch, total, count))?;
    println!(
        "  read    {count} random 256 KiB slices, bare     {:>8.0} MiB/s  ({:.2?} each)",
        mib_s(slice_bytes, rnd_bare),
        rnd_bare / count as u32
    );
    println!(
        "  read    {count} random 256 KiB slices, archive  {:>8.0} MiB/s  ({:.2?} each, {:+.1}%)",
        mib_s(slice_bytes, rnd_arch),
        rnd_arch / count as u32,
        (rnd_bare.as_secs_f64() / rnd_arch.as_secs_f64() - 1.0) * 100.0
    );

    let archive = FileArchive::open_path(&arch)?;
    let log = archive.find("logs/run.log").expect("the log member");
    let mut buf = vec![0u8; 64 * 1024];
    let t = Instant::now();
    for i in 0..1000u64 {
        archive.read_member_at(log, (i * 997) % (log.size - 65_536), &mut buf)?;
    }
    println!(
        "  read    1000 ranges of 64 KiB from a non-HDF5 member  {:.2?} each",
        t.elapsed() / 1000
    );

    let ((), verify_t) = timed(|| Ok(archive.verify(archive.primary())?))?;
    println!("  verify  CRC-32 of the whole HDF5 member {:>8.0} MiB/s  ({:.1?})   <- on demand, never per read", mib_s(bytes, verify_t), verify_t);
    Ok(())
}
