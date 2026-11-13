//! The archive layer on files: sealing a working file, reading the archive back by range, what an interrupted seal or a damaged file looks like, unsealing, and what the open file guards.
//!
//! A *working file* here is a file with a zeroed userblock and a valid HDF5 superblock (version 0 or 3) followed by filler: the archive layer treats the HDF5 member as bytes, and the IO layer's tests seal real HDF5 files.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use build_log_archive::{
    pack, seal, unseal, Detached, FileArchive, FileState, Manifest, MemberSource, ReadAt,
    SealOptions, Sniff, DEFAULT_PRIMARY, MACHINE_CONFIG_MEMBER, MIMETYPE, USERBLOCK,
};
use build_log_model::ErrorKind;

// ---------------------------------------------------------------- helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "build-log-archive-tests-{}-{}-{label}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("a temporary folder");
        TempDir(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Deterministic filler: the same seed gives the same bytes.
fn filler(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

const SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n'];

/// The bytes of a working file: a zeroed userblock, then an HDF5 superblock of `version` (0 or 3) whose end-of-file address is the length of the file, then filler.
fn working_bytes(version: u8, hdf5_len: usize, seed: u64) -> Vec<u8> {
    assert!(hdf5_len >= 128);
    let total = USERBLOCK as usize + hdf5_len;
    let mut bytes = vec![0u8; USERBLOCK as usize];
    let mut sb = filler(seed, hdf5_len);
    sb[..8].copy_from_slice(&SIGNATURE);
    let put = |sb: &mut Vec<u8>, at: usize, value: u64| {
        sb[at..at + 8].copy_from_slice(&value.to_le_bytes())
    };
    match version {
        0 => {
            sb[8] = 0; // superblock version
            sb[13] = 8; // size of offsets
            sb[14] = 8; // size of lengths
            put(&mut sb, 24, USERBLOCK); // base address
            put(&mut sb, 32, u64::MAX); // free space
            put(&mut sb, 40, total as u64); // end of file
        }
        3 => {
            sb[8] = 3;
            sb[9] = 8;
            sb[10] = 8;
            put(&mut sb, 12, USERBLOCK); // base address
            put(&mut sb, 20, u64::MAX); // extension
            put(&mut sb, 28, total as u64); // end of file
        }
        other => panic!("no superblock of version {other} here"),
    }
    bytes.extend_from_slice(&sb);
    bytes
}

fn working_file(
    dir: &TempDir,
    name: &str,
    version: u8,
    hdf5_len: usize,
    seed: u64,
) -> (PathBuf, Vec<u8>) {
    let bytes = working_bytes(version, hdf5_len, seed);
    let path = dir.path(name);
    fs::write(&path, &bytes).expect("a working file");
    (path, bytes)
}

fn full_options(dir: &TempDir) -> (SealOptions, Vec<u8>, Vec<u8>, Vec<u8>) {
    let mcf = filler(11, 10_000);
    let log = b"2026-11-13 capture started\n".repeat(50);
    let image = filler(12, 200_000);
    let image_path = dir.path("frame.png");
    fs::write(&image_path, &image).expect("an image");
    let options = SealOptions::new()
        .with_machine_config(MemberSource::Bytes(mcf.clone()))
        .with_member("logs/capture.log", MemberSource::Bytes(log.clone()))
        .with_member("attachments/frame.png", MemberSource::File(image_path))
        .with_schema_version(18);
    (options, mcf, log, image)
}

fn error_kind<T>(result: Result<T, build_log_model::Error>) -> ErrorKind {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.kind().clone(),
    }
}

// ---------------------------------------------------------------- sealing and reading back

#[test]
fn a_sealed_working_file_reads_back_every_member_byte_for_byte() {
    for version in [0u8, 3] {
        let dir = TempDir::new("roundtrip");
        let (path, bytes) = working_file(&dir, "build.lbl", version, 300_000, 1);
        let (options, mcf, log, image) = full_options(&dir);
        let report = seal(&path, &options).expect("the working file seals");
        assert_eq!(report.hdf5_len, 300_000);
        assert_eq!(report.discarded_bytes, 0);

        let archive = FileArchive::open_path(&path).expect("the archive opens");
        let names: Vec<&str> = archive.members().iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "mimetype",
                "build.h5",
                "machine_config.h5",
                "logs/capture.log",
                "attachments/frame.png",
                "lbl-manifest.json"
            ]
        );
        let read = |name: &str| {
            archive
                .read_member(archive.find(name).unwrap(), 1 << 30)
                .unwrap()
        };
        assert_eq!(read("mimetype"), MIMETYPE.as_bytes());
        assert_eq!(read("machine_config.h5"), mcf);
        assert_eq!(read("logs/capture.log"), log);
        assert_eq!(read("attachments/frame.png"), image);
        // the HDF5 member is the file's own bytes after the userblock, untouched by the seal
        assert_eq!(read("build.h5"), bytes[USERBLOCK as usize..]);
        for member in archive.members() {
            archive
                .verify(member)
                .unwrap_or_else(|e| panic!("{}: {e}", member.name));
            assert!(member.stored, "{}", member.name);
        }
        assert_eq!(archive.reader().len().unwrap(), report.archive_len);
    }
}

#[test]
fn the_hdf5_member_starts_at_the_userblock_and_every_other_member_at_a_multiple_of_4096() {
    let dir = TempDir::new("alignment");
    let (path, _) = working_file(&dir, "build.lbl", 3, 100_000, 2);
    let (options, ..) = full_options(&dir);
    seal(&path, &options).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(archive.primary().name, DEFAULT_PRIMARY);
    assert_eq!(archive.primary().data_offset, USERBLOCK);
    assert_eq!(
        archive.primary_userblock(),
        Some(USERBLOCK),
        "stock HDF5 finds the superblock there by itself"
    );
    for member in archive.members() {
        if member.name != "mimetype" {
            assert_eq!(member.data_offset % 4096, 0, "{}", member.name);
        }
    }
    // `mimetype` is first, so a tool can tell the type from the first bytes: its text is at byte 38
    let head = fs::read(&path).unwrap();
    assert_eq!(&head[38..38 + MIMETYPE.len()], MIMETYPE.as_bytes());
}

#[test]
fn the_manifest_names_the_members_and_the_version_hint() {
    let dir = TempDir::new("manifest");
    let (path, _) = working_file(&dir, "build.lbl", 3, 50_000, 3);
    let (options, ..) = full_options(&dir);
    seal(&path, &options.with_created_by("a test")).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    let manifest: &Manifest = archive.manifest();
    assert_eq!(manifest.primary.as_deref(), Some("build.h5"));
    assert_eq!(
        manifest.machine_config.as_deref(),
        Some(MACHINE_CONFIG_MEMBER)
    );
    assert_eq!(manifest.schema_version, Some(18));
    assert_eq!(
        manifest.attachments,
        ["logs/capture.log", "attachments/frame.png"]
    );
    assert_eq!(manifest.created_by.as_deref(), Some("a test"));
    assert!(manifest
        .created_utc
        .as_deref()
        .is_some_and(|t| t.ends_with('Z') && t.len() == 20));
    assert_eq!(
        archive.machine_config().map(|m| m.name.as_str()),
        Some("machine_config.h5")
    );
}

#[test]
fn sealing_with_the_zip64_fields_on_every_member_reads_back_the_same() {
    let dir = TempDir::new("zip64");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 80_000, 4);
    let (options, mcf, ..) = full_options(&dir);
    seal(&path, &options.with_zip64(true)).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(archive.primary().data_offset, USERBLOCK);
    assert_eq!(
        archive.read_member(archive.primary(), 1 << 30).unwrap(),
        bytes[USERBLOCK as usize..]
    );
    assert_eq!(
        archive
            .read_member(archive.machine_config().unwrap(), 1 << 30)
            .unwrap(),
        mcf
    );
    for member in archive.members() {
        archive.verify(member).unwrap();
    }
}

#[test]
fn an_archive_with_only_the_hdf5_member_is_an_archive() {
    let dir = TempDir::new("minimal");
    let (path, _) = working_file(&dir, "build.lbl", 3, 20_000, 5);
    seal(&path, &SealOptions::new()).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(
        archive.members().len(),
        3,
        "mimetype, the HDF5 member and the manifest"
    );
    assert!(archive.machine_config().is_none());
}

#[test]
fn a_range_read_stops_at_the_end_of_the_member_and_refuses_to_start_past_it() {
    let dir = TempDir::new("ranges");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 30_000, 6);
    seal(&path, &SealOptions::new()).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    let primary = archive.primary().clone();
    let mut buf = vec![0u8; 100];
    assert_eq!(archive.read_member_at(&primary, 10, &mut buf).unwrap(), 100);
    assert_eq!(
        buf,
        bytes[USERBLOCK as usize + 10..USERBLOCK as usize + 110]
    );
    // asking for more than is left gives what is left, and at the end, nothing
    assert_eq!(
        archive
            .read_member_at(&primary, primary.size - 40, &mut buf)
            .unwrap(),
        40
    );
    assert_eq!(
        archive
            .read_member_at(&primary, primary.size, &mut buf)
            .unwrap(),
        0
    );
    let kind = error_kind(archive.read_member_at(&primary, primary.size + 1, &mut buf));
    assert!(matches!(kind, ErrorKind::InvalidIndex { .. }), "{kind:?}");
    // a member is not read whole past the budget the caller set
    let kind = error_kind(archive.read_member(&primary, 1000));
    assert!(
        matches!(
            kind,
            ErrorKind::BudgetExceeded {
                limit_bytes: 1000,
                ..
            }
        ),
        "{kind:?}"
    );
}

#[test]
fn many_threads_reading_ranges_of_one_archive_do_not_disturb_each_other() {
    let dir = TempDir::new("threads");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 400_000, 7);
    seal(&path, &SealOptions::new()).unwrap();
    let archive = Arc::new(FileArchive::open_path(&path).unwrap());
    let expected = Arc::new(bytes[USERBLOCK as usize..].to_vec());
    let threads: Vec<_> = (0..8u64)
        .map(|t| {
            let (archive, expected) = (Arc::clone(&archive), Arc::clone(&expected));
            std::thread::spawn(move || {
                let primary = archive.primary().clone();
                for i in 0..200u64 {
                    let at = (t * 7919 + i * 104_729) % (primary.size - 512);
                    let mut buf = vec![0u8; 512];
                    assert_eq!(archive.read_member_at(&primary, at, &mut buf).unwrap(), 512);
                    assert_eq!(buf, expected[at as usize..at as usize + 512]);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a reader thread");
    }
}

// ---------------------------------------------------------------- working files, interrupted seals, files that are not archives

#[test]
fn a_working_file_is_reported_unsealed_and_other_files_are_not_archives() {
    let dir = TempDir::new("unsealed");
    let (working, _) = working_file(&dir, "working.lbl", 3, 20_000, 8);
    let kind = error_kind(FileArchive::open_path(&working));
    assert!(matches!(kind, ErrorKind::Unsealed { .. }), "{kind:?}");
    assert_eq!(
        Sniff::of(&fs::File::open(&working).unwrap()).unwrap(),
        Sniff::WorkingFile
    );

    let bare = dir.path("bare.h5");
    let mut hdf5 = SIGNATURE.to_vec();
    hdf5.extend_from_slice(&filler(9, 5_000));
    fs::write(&bare, &hdf5).unwrap();
    assert_eq!(
        Sniff::of(&fs::File::open(&bare).unwrap()).unwrap(),
        Sniff::Hdf5
    );
    assert!(matches!(
        error_kind(FileArchive::open_path(&bare)),
        ErrorKind::Corrupt { .. }
    ));

    let junk = dir.path("junk.lbl");
    fs::write(&junk, filler(10, 9_000)).unwrap();
    assert_eq!(
        Sniff::of(&fs::File::open(&junk).unwrap()).unwrap(),
        Sniff::Unknown
    );
    assert!(matches!(
        error_kind(FileArchive::open_path(&junk)),
        ErrorKind::Corrupt { .. }
    ));

    let missing = dir.path("missing.lbl");
    assert!(matches!(
        error_kind(FileArchive::open_path(&missing)),
        ErrorKind::IoError { .. }
    ));
}

#[test]
fn a_seal_cut_off_before_the_directory_leaves_a_working_file_that_seals_again() {
    let dir = TempDir::new("interrupted");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 60_000, 11);
    let (options, mcf, ..) = full_options(&dir);
    let report = seal(&path, &options).unwrap();
    // the interruption: everything is written but the directory and the end records, which are written last
    let manifest = report.members.last().unwrap();
    let cut = manifest.data_end();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(cut).unwrap();
    drop(file);
    let kind = error_kind(FileArchive::open_path(&path));
    assert!(
        matches!(kind, ErrorKind::Unsealed { .. }),
        "an archive with no directory is not an archive: {kind:?}"
    );

    let again = seal(&path, &options).expect("sealing again");
    assert!(
        again.discarded_bytes > 0,
        "the members of the first attempt were cut off"
    );
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(
        archive.read_member(archive.primary(), 1 << 30).unwrap(),
        bytes[USERBLOCK as usize..]
    );
    assert_eq!(
        archive
            .read_member(archive.machine_config().unwrap(), 1 << 30)
            .unwrap(),
        mcf
    );
}

#[test]
fn sealing_cuts_off_what_follows_the_hdf5_data() {
    let dir = TempDir::new("trailing");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 40_000, 12);
    let mut with_junk = bytes.clone();
    with_junk.extend_from_slice(&filler(13, 777));
    fs::write(&path, &with_junk).unwrap();
    let report = seal(&path, &SealOptions::new()).unwrap();
    assert_eq!(report.discarded_bytes, 777);
    assert_eq!(report.hdf5_len, 40_000);
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(archive.primary().size, 40_000);
}

#[test]
fn a_file_that_is_not_a_complete_working_file_is_not_sealed() {
    let dir = TempDir::new("refused");
    // no HDF5 file at byte 4096
    let junk = dir.path("junk.lbl");
    fs::write(&junk, vec![0u8; 10_000]).unwrap();
    assert!(matches!(
        error_kind(seal(&junk, &SealOptions::new())),
        ErrorKind::Corrupt { .. }
    ));
    // an HDF5 file whose superblock says it is longer than the file: it was cut short
    let (short, bytes) = working_file(&dir, "short.lbl", 3, 20_000, 14);
    fs::write(&short, &bytes[..bytes.len() - 100]).unwrap();
    assert!(matches!(
        error_kind(seal(&short, &SealOptions::new())),
        ErrorKind::Corrupt { .. }
    ));
    // a base address that is not the userblock's end
    let (shifted, mut bytes) = working_file(&dir, "shifted.lbl", 3, 20_000, 15);
    bytes[USERBLOCK as usize + 12..USERBLOCK as usize + 20].copy_from_slice(&0u64.to_le_bytes());
    fs::write(&shifted, &bytes).unwrap();
    assert!(matches!(
        error_kind(seal(&shifted, &SealOptions::new())),
        ErrorKind::Corrupt { .. }
    ));
    // a sealed archive is not sealed again
    let (sealed, _) = working_file(&dir, "sealed.lbl", 3, 20_000, 16);
    seal(&sealed, &SealOptions::new()).unwrap();
    assert!(matches!(
        error_kind(seal(&sealed, &SealOptions::new())),
        ErrorKind::ValidationError { .. }
    ));
}

#[test]
fn member_names_that_could_escape_or_collide_are_refused_before_anything_is_written() {
    let dir = TempDir::new("names");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 20_000, 17);
    let bad = [
        SealOptions::new().with_member("../evil", MemberSource::Bytes(vec![1])),
        SealOptions::new().with_member("/absolute", MemberSource::Bytes(vec![1])),
        SealOptions::new().with_member("back\\slash", MemberSource::Bytes(vec![1])),
        SealOptions::new().with_member("", MemberSource::Bytes(vec![1])),
        SealOptions::new()
            .with_member("a", MemberSource::Bytes(vec![1]))
            .with_member("a", MemberSource::Bytes(vec![2])),
        SealOptions::new().with_member("mimetype", MemberSource::Bytes(vec![1])),
        SealOptions::new().with_member("lbl-manifest.json", MemberSource::Bytes(vec![1])),
        SealOptions::new().with_member("build.h5", MemberSource::Bytes(vec![1])),
        SealOptions::new()
            .with_machine_config(MemberSource::Bytes(vec![1]))
            .with_member("machine_config.h5", MemberSource::Bytes(vec![2])),
    ];
    for options in bad {
        assert!(
            matches!(
                error_kind(seal(&path, &options)),
                ErrorKind::ValidationError { .. }
            ),
            "{options:?}"
        );
    }
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "a refused seal writes nothing"
    );
}

#[test]
fn a_member_file_that_changes_size_while_it_is_copied_in_is_refused() {
    let dir = TempDir::new("missing-source");
    let (path, _) = working_file(&dir, "build.lbl", 3, 20_000, 18);
    let options =
        SealOptions::new().with_member("a.bin", MemberSource::File(dir.path("not-there.bin")));
    assert!(matches!(
        error_kind(seal(&path, &options)),
        ErrorKind::IoError { .. }
    ));
    // and the file is still a working file, so a corrected seal works
    assert!(matches!(
        error_kind(FileArchive::open_path(&path)),
        ErrorKind::Unsealed { .. }
    ));
    seal(&path, &SealOptions::new()).unwrap();
}

// ---------------------------------------------------------------- damage

fn flip(path: &Path, at: u64) {
    let mut bytes = fs::read(path).unwrap();
    bytes[at as usize] ^= 0xFF;
    fs::write(path, bytes).unwrap();
}

#[test]
fn a_damaged_member_fails_its_own_check_and_no_other() {
    let dir = TempDir::new("damage");
    let (path, _) = working_file(&dir, "build.lbl", 3, 100_000, 19);
    let (options, ..) = full_options(&dir);
    seal(&path, &options).unwrap();
    let (mcf_at, hdf5_at) = {
        let a = FileArchive::open_path(&path).unwrap();
        (
            a.machine_config().unwrap().data_offset + 5,
            a.primary().data_offset + 5_000,
        )
    };
    flip(&path, mcf_at);
    let archive =
        FileArchive::open_path(&path).expect("the directory is intact, so the archive opens");
    for member in archive.members() {
        let result = archive.verify(member);
        if member.name == "machine_config.h5" {
            assert!(matches!(error_kind(result), ErrorKind::Corrupt { .. }));
        } else {
            result.unwrap_or_else(|e| panic!("{}: {e}", member.name));
        }
    }
    drop(archive);
    flip(&path, hdf5_at);
    let archive = FileArchive::open_path(&path).unwrap();
    assert!(matches!(
        error_kind(archive.verify(archive.primary())),
        ErrorKind::Corrupt { .. }
    ));
}

#[test]
fn a_damaged_directory_is_corrupt_and_a_zip_that_is_not_an_lbl_is_refused() {
    let dir = TempDir::new("directory");
    let (path, _) = working_file(&dir, "build.lbl", 3, 30_000, 20);
    let report = seal(&path, &SealOptions::new()).unwrap();
    let original = fs::read(&path).unwrap();

    // the signature of the first entry of the central directory
    let manifest = report.members.last().unwrap();
    let directory_at = manifest.data_end() as usize;
    let mut damaged = original.clone();
    damaged[directory_at] ^= 0xFF;
    fs::write(&path, &damaged).unwrap();
    assert!(matches!(
        error_kind(FileArchive::open_path(&path)),
        ErrorKind::Corrupt { .. }
    ));

    // `mimetype` that says another type
    let mut other = original.clone();
    other[38..38 + MIMETYPE.len()].copy_from_slice(b"application/vnd.xyz+zip");
    fs::write(&path, &other).unwrap();
    let kind = error_kind(FileArchive::open_path(&path));
    assert!(
        matches!(&kind, ErrorKind::Corrupt { detail } if detail.contains("not an .lbl")),
        "{kind:?}"
    );

    // a manifest of a newer format of the archive
    let json = String::from_utf8_lossy(
        &original[manifest.data_offset as usize..manifest.data_end() as usize],
    )
    .into_owned();
    assert!(json.contains("\"lbl_container_version\": 1"));
    let mut newer = original.clone();
    let at = manifest.data_offset as usize
        + json.find("\"lbl_container_version\": 1").unwrap()
        + "\"lbl_container_version\": ".len();
    newer[at] = b'2';
    fs::write(&path, &newer).unwrap();
    let kind = error_kind(FileArchive::open_path(&path));
    assert!(
        matches!(kind, ErrorKind::UnsupportedInVersion { .. }),
        "{kind:?}"
    );

    // a compressed HDF5 member cannot be read by range
    let mut compressed = original.clone();
    let entries: Vec<usize> = (0..compressed.len() - 4)
        .filter(|&i| i >= directory_at && compressed[i..i + 4] == [0x50, 0x4B, 0x01, 0x02])
        .collect();
    compressed[entries[1] + 10] = 8; // the method of the second entry, the HDF5 member
    fs::write(&path, &compressed).unwrap();
    let kind = error_kind(FileArchive::open_path(&path));
    assert!(
        matches!(kind, ErrorKind::UnsupportedInVersion { .. }),
        "{kind:?}"
    );

    // and the untouched archive still opens
    fs::write(&path, &original).unwrap();
    FileArchive::open_path(&path).unwrap();
}

// ---------------------------------------------------------------- unsealing

#[test]
fn unsealing_cuts_the_file_back_to_a_working_file_and_sealing_again_restores_every_other_member() {
    let dir = TempDir::new("unseal");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 120_000, 21);
    let (options, mcf, log, image) = full_options(&dir);
    seal(&path, &options).unwrap();

    let detached = unseal(&path, 1 << 30).expect("unsealing");
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        USERBLOCK + 120_000,
        "only the HDF5 data is left"
    );
    assert!(
        detached.sidecar().is_file(),
        "the other members are safe in the side file"
    );
    assert_eq!(
        detached.member_names(),
        [
            "machine_config.h5",
            "logs/capture.log",
            "attachments/frame.png"
        ]
    );
    assert!(matches!(
        error_kind(FileArchive::open_path(&path)),
        ErrorKind::Unsealed { .. }
    ));

    // a change to the HDF5 data, as a read-write HDF5 pass would make (here: a byte in the middle and a longer file is not needed)
    let mut changed = fs::read(&path).unwrap();
    changed[USERBLOCK as usize + 50_000] ^= 0x55;
    fs::write(&path, &changed).unwrap();

    let report = seal(&path, &detached.to_options()).expect("sealing again");
    detached.finish().unwrap();
    assert!(
        !dir.path("build.lbl.detached.zip").exists(),
        "the side file is removed once the archive is whole again"
    );
    let archive = FileArchive::open_path(&path).unwrap();
    let read = |name: &str| {
        archive
            .read_member(archive.find(name).unwrap(), 1 << 30)
            .unwrap()
    };
    assert_eq!(read("build.h5"), changed[USERBLOCK as usize..]);
    assert_ne!(
        read("build.h5"),
        bytes[USERBLOCK as usize..],
        "the new HDF5 member is the changed one"
    );
    assert_eq!(read("machine_config.h5"), mcf);
    assert_eq!(read("logs/capture.log"), log);
    assert_eq!(read("attachments/frame.png"), image);
    assert_eq!(archive.manifest().schema_version, Some(18));
    for member in archive.members() {
        archive.verify(member).unwrap();
    }
    assert_eq!(report.members.len(), 6);
}

#[test]
fn an_interrupted_unseal_loses_nothing_because_the_side_file_comes_first() {
    let dir = TempDir::new("unseal-crash");
    let (path, _) = working_file(&dir, "build.lbl", 3, 50_000, 22);
    let (options, mcf, log, _) = full_options(&dir);
    seal(&path, &options).unwrap();
    let detached = unseal(&path, 1 << 30).unwrap();
    drop(detached); // the process "dies" here: nothing calls `finish`, nothing seals again

    let recovered = Detached::load(&path)
        .expect("the side file reads")
        .expect("there is one");
    assert_eq!(
        recovered.member_names(),
        [
            "machine_config.h5",
            "logs/capture.log",
            "attachments/frame.png"
        ]
    );
    seal(&path, &recovered.to_options()).unwrap();
    recovered.finish().unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(
        archive
            .read_member(archive.machine_config().unwrap(), 1 << 30)
            .unwrap(),
        mcf
    );
    assert_eq!(
        archive
            .read_member(archive.find("logs/capture.log").unwrap(), 1 << 30)
            .unwrap(),
        log
    );
    assert!(
        Detached::load(&path).unwrap().is_none(),
        "no side file is left"
    );
}

#[test]
fn a_second_unseal_while_a_side_file_exists_is_refused() {
    let dir = TempDir::new("unseal-twice");
    let (path, _) = working_file(&dir, "build.lbl", 3, 30_000, 23);
    let (options, ..) = full_options(&dir);
    seal(&path, &options).unwrap();
    let _detached = unseal(&path, 1 << 30).unwrap();
    seal(&path, &SealOptions::new()).unwrap(); // sealed again WITHOUT the other members: the side file is still the only copy
    let kind = error_kind(unseal(&path, 1 << 30));
    assert!(
        matches!(&kind, ErrorKind::ValidationError { .. }),
        "{kind:?}"
    );
}

#[test]
fn an_unseal_that_would_read_more_than_the_limit_changes_nothing() {
    let dir = TempDir::new("unseal-budget");
    let (path, _) = working_file(&dir, "build.lbl", 3, 30_000, 24);
    let (options, ..) = full_options(&dir);
    seal(&path, &options).unwrap();
    let before = fs::read(&path).unwrap();
    let kind = error_kind(unseal(&path, 1_000));
    assert!(
        matches!(
            kind,
            ErrorKind::BudgetExceeded {
                limit_bytes: 1_000,
                ..
            }
        ),
        "{kind:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!dir.path("build.lbl.detached.zip").exists());
}

// ---------------------------------------------------------------- what the open file guards

#[test]
fn an_open_archive_reports_unchanged_and_then_superseded_when_a_new_version_takes_its_place() {
    let dir = TempDir::new("superseded");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 30_000, 25);
    seal(&path, &SealOptions::new()).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(archive.check(), FileState::Unchanged);

    let (next, _) = working_file(&dir, "next.lbl", 3, 31_000, 26);
    seal(&next, &SealOptions::new()).unwrap();
    fs::rename(&next, &path)
        .expect("a new version replaces the file, even while a reader has it open");
    assert_eq!(archive.check(), FileState::Superseded);
    // the open archive still reads the version it opened, consistently
    assert_eq!(
        archive.read_member(archive.primary(), 1 << 30).unwrap(),
        bytes[USERBLOCK as usize..]
    );
    for member in archive.members() {
        archive.verify(member).unwrap();
    }
}

#[test]
fn an_open_archive_reports_removed_when_its_file_is_deleted_and_still_reads() {
    let dir = TempDir::new("removed");
    let (path, bytes) = working_file(&dir, "build.lbl", 3, 30_000, 27);
    seal(&path, &SealOptions::new()).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    fs::remove_file(&path).expect("a file can be deleted while a reader has it open");
    assert_eq!(archive.check(), FileState::Removed);
    assert_eq!(
        archive.read_member(archive.primary(), 1 << 30).unwrap(),
        bytes[USERBLOCK as usize..]
    );
}

#[cfg(windows)]
#[test]
fn on_windows_an_open_archive_keeps_writers_out_and_a_writer_keeps_the_archive_from_opening() {
    let dir = TempDir::new("sharing");
    let (path, _) = working_file(&dir, "build.lbl", 3, 30_000, 28);
    seal(&path, &SealOptions::new()).unwrap();
    {
        let _archive = FileArchive::open_path(&path).unwrap();
        // nobody can change the file under it
        assert!(
            fs::OpenOptions::new().write(true).open(&path).is_err(),
            "a writer is refused while the archive is open"
        );
        // a seal or an unseal, which write, are refused too, with an error that says to try again
        let sealing = error_kind(unseal(&path, 1 << 30));
        assert!(
            matches!(
                sealing,
                ErrorKind::IoError {
                    transient: true,
                    ..
                }
            ),
            "{sealing:?}"
        );
        // readers are not excluded
        FileArchive::open_path(&path).expect("a second reader");
    }
    let writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let kind = error_kind(FileArchive::open_path(&path));
    assert!(
        matches!(
            &kind,
            ErrorKind::IoError {
                transient: true,
                ..
            }
        ),
        "{kind:?}"
    );
    drop(writer);
    FileArchive::open_path(&path).expect("it opens once the writer is gone");
}

#[cfg(unix)]
#[test]
fn on_unix_a_change_in_place_under_an_open_archive_is_reported_modified() {
    use std::io::{Seek, SeekFrom, Write};
    let dir = TempDir::new("modified");
    let (path, _) = working_file(&dir, "build.lbl", 3, 30_000, 29);
    seal(&path, &SealOptions::new()).unwrap();
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(archive.check(), FileState::Unchanged);
    let mut writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"more").unwrap();
    writer.sync_all().unwrap();
    assert_eq!(archive.check(), FileState::Modified);
}

// ---------------------------------------------------------------- packing a bare HDF5 file

/// A bare HDF5 file as HDF5 writes it without a userblock: the superblock at byte 0, naming base address 0 and an end equal to its length.
fn bare_hdf5_bytes(hdf5_len: usize, seed: u64) -> Vec<u8> {
    let mut bytes = filler(seed, hdf5_len);
    bytes[..8].copy_from_slice(&SIGNATURE);
    bytes[8] = 3;
    bytes[9] = 8;
    bytes[10] = 8;
    bytes[12..20].copy_from_slice(&0u64.to_le_bytes());
    bytes[20..28].copy_from_slice(&u64::MAX.to_le_bytes());
    bytes[28..36].copy_from_slice(&(hdf5_len as u64).to_le_bytes());
    bytes
}

#[test]
fn a_bare_hdf5_file_is_packed_into_an_archive_with_its_bytes_unchanged() {
    let dir = TempDir::new("pack");
    let source = dir.path("old.h5");
    let bare = bare_hdf5_bytes(90_000, 30);
    fs::write(&source, &bare).unwrap();
    let (options, mcf, ..) = full_options(&dir);
    let dest = dir.path("new.lbl");
    let report = pack(&source, &dest, &options).expect("packing");
    assert_eq!(
        report.hdf5_len, 90_000,
        "the length is the superblock's end minus its base, whichever base it names"
    );
    let archive = FileArchive::open_path(&dest).unwrap();
    assert_eq!(archive.primary().data_offset, USERBLOCK);
    assert_eq!(
        archive.read_member(archive.primary(), 1 << 30).unwrap(),
        bare,
        "the HDF5 bytes are copied as they are"
    );
    assert_eq!(
        archive
            .read_member(archive.machine_config().unwrap(), 1 << 30)
            .unwrap(),
        mcf
    );
    for member in archive.members() {
        archive.verify(member).unwrap();
    }
    assert_eq!(
        fs::read(&source).unwrap(),
        bare,
        "the source is not touched"
    );
}

#[test]
fn packing_refuses_what_it_cannot_pack_and_leaves_no_half_made_archive() {
    let dir = TempDir::new("pack-refused");
    let dest = dir.path("new.lbl");
    // not HDF5
    let junk = dir.path("junk.h5");
    fs::write(&junk, filler(31, 10_000)).unwrap();
    assert!(matches!(
        error_kind(pack(&junk, &dest, &SealOptions::new())),
        ErrorKind::ValidationError { .. }
    ));
    assert!(!dest.exists());
    // HDF5 whose superblock says it is longer than the file: the copy is made, the seal refuses, and the copy is removed
    let mut cut = bare_hdf5_bytes(50_000, 32);
    cut.truncate(40_000);
    let short = dir.path("short.h5");
    fs::write(&short, &cut).unwrap();
    assert!(matches!(
        error_kind(pack(&short, &dest, &SealOptions::new())),
        ErrorKind::Corrupt { .. }
    ));
    assert!(!dest.exists(), "a failed pack leaves nothing behind");
    // a destination that exists is not overwritten
    let good = dir.path("good.h5");
    fs::write(&good, bare_hdf5_bytes(20_000, 33)).unwrap();
    fs::write(&dest, b"mine").unwrap();
    assert!(matches!(
        error_kind(pack(&good, &dest, &SealOptions::new())),
        ErrorKind::IoError { .. }
    ));
    assert_eq!(fs::read(&dest).unwrap(), b"mine");
}
