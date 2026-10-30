//! The cost of opening a build, and the proof that making it cheaper changes nothing (IP-07 phase 7E, task T07.5; decision D07-8).
//!
//! Vision's target is **at most 1 s cold and 100 ms warm for about 1,000 layers**. The numbers are measured on the local discovery-scale files and recorded in the plan (Appendix B of IP-07);
//! **they are recorded and not gated** (decision D06-15): a timing in CI is flaky, and the files are not in CI.
//!
//! * `the_open_of_the_many_layer_file_is_timed`: the open and the discovery of `v17_many_layers.h5` (2,110 layers) and of `v17_1000_layers.h5` (1,000 layers, made for the purpose), each run
//!   once and again, and what a second ask of the open source costs (`build`, `layers`, `validate`, `availability`). Run it as
//!   `cargo test --release -p build-log-io --test open_cost -- --nocapture`; it skips a file that is not there.
//! * `a_second_ask_of_an_open_source_is_an_arc_clone`: what "warm" means: the same shared snapshot, not a read of the file again.
//! * `trimming_changes_no_metadata`: **the metadata of every V17 file that is here, as one number each, is what it was before the open was made cheaper.** The number is the hash of the
//!   whole `Debug` text of the build (the origins, the extra nodes and every field that an expectation file does not say) and the length of that text. The first run writes the
//!   numbers (`BUILD_LOG_WRITE_GOLDEN=1`) to `tools/open_cost/metadata_golden.txt`, which is local like the tools (decision D00-10); a later run compares. It skips where there is no
//!   such file, so in CI the expectation files (the conformance tests of every V17 file) are what holds the metadata.

mod common;

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use build_log_io::dispatch::{open, production};
use build_log_model::BuildMeta;
use build_log_testkit::{large_fixture, miniature, miniature_path, repo_root, skip_message};

/// A 64-bit FNV-1a hash that is written to as text, so that a build's whole `Debug` text is hashed without being held.
struct Digest {
    hash: u64,
    length: u64,
}

impl Digest {
    fn new() -> Self {
        Digest {
            hash: 0xcbf2_9ce4_8422_2325,
            length: 0,
        }
    }
}

impl fmt::Write for Digest {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            self.hash ^= u64::from(byte);
            self.hash = self.hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.length += text.len() as u64;
        Ok(())
    }
}

/// The metadata of a build as one number and one length.
fn digest_of(build: &BuildMeta) -> String {
    let mut digest = Digest::new();
    write!(digest, "{build:?}").expect("a digest takes any text");
    format!("{:016x}:{}", digest.hash, digest.length)
}

/// The V17 files that are here, by a key that says where each is from: the six miniatures, the local extracts and working files, and the 13 edge-case files.
fn v17_files() -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    for name in [
        miniature::V17_MOCK,
        miniature::V17_OCT15,
        miniature::V17_TRIGGERED_2026_10_19,
        miniature::V17_2026_08_18_IMAGES,
        miniature::V17_MARCH,
        miniature::V17_SPECIAL_LAYERS,
    ] {
        files.push((format!("miniature/{name}"), miniature_path(name)));
    }
    for name in [
        "extracts/v17_march_layer_1.h5",
        "extracts/special_layers_v17.h5",
        "extracts/v17_many_layers.h5",
        "extracts/v17_1000_layers.h5",
        "Build Log Fixtures/V17_triggered_2026_10_19.h5",
        "Build Log Fixtures/V17_2026_08_18_with_images.h5",
        "Build Log Fixtures/V17_fixture_generated.h5",
        "Build Log Fixtures/V17_fixture_trimmed.h5",
        "Build Log Fixtures/3MF_Test_CLI+checked_2026-10-15T18-13-06.h5",
    ] {
        if let Some(path) = large_fixture(name) {
            files.push((format!("local/{name}"), path));
        }
    }
    let edge = repo_root().join("tools").join("edge_cases").join("files");
    if let Ok(entries) = std::fs::read_dir(&edge) {
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "h5"))
            .collect();
        found.sort();
        for path in found {
            let name = path
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .to_string();
            files.push((format!("edge/{name}"), path));
        }
    }
    files
}

/// What opening a file says of its metadata: the digest, or the error (a file that does not open is part of what must not change).
fn metadata_of(path: &Path) -> String {
    match open(production(), path) {
        Ok(opened) => match opened.source().build() {
            Ok(build) => digest_of(&build),
            Err(error) => format!("error: {error}"),
        },
        Err(error) => format!("error: {error}"),
    }
}

fn golden_path() -> PathBuf {
    repo_root()
        .join("tools")
        .join("open_cost")
        .join("metadata_golden.txt")
}

#[test]
fn trimming_changes_no_metadata() {
    let files = v17_files();
    assert!(
        files.len() >= 6,
        "the six miniatures are here: {}",
        files.len()
    );
    // the same file gives the same number twice: the text that is hashed does not depend on anything but the metadata
    let (key, path) = &files[0];
    assert_eq!(metadata_of(path), metadata_of(path), "{key}");
    // the six miniatures are different builds and have different numbers
    let numbers: std::collections::BTreeSet<String> = files
        .iter()
        .take(6)
        .map(|(_, path)| metadata_of(path))
        .collect();
    assert_eq!(numbers.len(), 6);

    let golden = golden_path();
    if std::env::var_os("BUILD_LOG_WRITE_GOLDEN").is_some() {
        let mut text = String::new();
        for (key, path) in &files {
            writeln!(text, "{key} {}", metadata_of(path)).expect("text");
        }
        std::fs::create_dir_all(golden.parent().expect("a folder")).expect("the folder");
        std::fs::write(&golden, text).expect("the golden file is written");
        eprintln!("wrote {} files to {}", files.len(), golden.display());
        return;
    }
    let Ok(recorded) = std::fs::read_to_string(&golden) else {
        eprintln!(
            "{}",
            skip_message(&format!(
                "there is no {} (run once with BUILD_LOG_WRITE_GOLDEN=1 before changing the reader)",
                golden.display()
            ))
        );
        return;
    };
    let mut compared = 0;
    for line in recorded.lines() {
        // a key may have spaces in it (a folder of the local fixtures does), and a number has none
        let Some((key, expected)) = line.rsplit_once(' ') else {
            continue;
        };
        let Some((_, path)) = files.iter().find(|(k, _)| k == key) else {
            eprintln!("{}", skip_message(&format!("{key} is not here")));
            continue;
        };
        assert_eq!(metadata_of(path), expected, "the metadata of {key} changed");
        compared += 1;
    }
    assert!(compared >= 6, "{compared} files compared");
    eprintln!("{compared} files compared, none changed");
}

/// Milliseconds of what `work` does, with the thing it returns.
fn timed<T>(work: impl FnOnce() -> T) -> (f64, T) {
    let start = Instant::now();
    let result = work();
    (start.elapsed().as_secs_f64() * 1_000.0, result)
}

#[test]
fn the_open_of_the_many_layer_file_is_timed() {
    // recorded, not gated: the numbers go into Appendix B of the plan
    for (name, layers) in [
        ("extracts/v17_1000_layers.h5", 1_000usize),
        ("extracts/v17_many_layers.h5", 2_110),
    ] {
        let Some(path) = large_fixture(name) else {
            eprintln!(
                "{}",
                skip_message(&format!("the large fixture {name} is not here"))
            );
            continue;
        };
        // the first open in this process, and a second one of the same file (the library has loaded, and so has the system's cache)
        let (first, opened) = timed(|| open(production(), &path).expect("the file opens"));
        let source = opened.source();
        let (build_first, build) = timed(|| source.build().expect("the metadata"));
        assert_eq!(build.layers.len(), layers, "{name}");
        let (second_open, again) =
            timed(|| open(production(), &path).expect("the file opens again"));
        drop(again);
        // what warm is: the second ask of an open source
        let (build_second, build_again) = timed(|| source.build().expect("the metadata"));
        assert!(
            Arc::ptr_eq(&build, &build_again),
            "{name}: the same snapshot"
        );
        let (layers_ms, ids) = timed(|| source.layers().expect("the layers"));
        assert_eq!(ids.len(), layers);
        let laser = build.lasers().first().expect("a laser").key.clone();
        let (lasers_ms, _) = timed(|| source.lasers_in_layer(ids[0]).expect("the lasers"));
        let (channels_ms, _) = timed(|| source.channels(&laser).expect("the channels"));
        let (validate_first, validation) = timed(|| source.validate().expect("the findings"));
        let (validate_second, _) = timed(|| source.validate().expect("the findings"));
        let (availability_first, availability) =
            timed(|| source.availability().expect("the availability"));
        let (availability_second, _) = timed(|| source.availability().expect("the availability"));
        eprintln!(
            "== {name}: {layers} layers, {} items",
            availability.counts().total()
        );
        eprintln!(
            "   open, the first in the process       {first:9.1} ms  ({:.2} ms a layer)",
            first / layers as f64
        );
        eprintln!(
            "   open, the second                     {second_open:9.1} ms  ({:.2} ms a layer)",
            second_open / layers as f64
        );
        eprintln!("   build(), the first ask               {build_first:9.4} ms");
        eprintln!("   build(), the second ask              {build_second:9.4} ms");
        eprintln!("   layers()                             {layers_ms:9.4} ms");
        eprintln!(
            "   lasers_in_layer(), channels()        {lasers_ms:9.4} ms, {channels_ms:9.4} ms"
        );
        eprintln!("   validate(), first and second ask     {validate_first:9.3} ms, {validate_second:9.4} ms  ({} findings)", validation.findings.len());
        eprintln!("   availability(), first and second ask {availability_first:9.3} ms, {availability_second:9.4} ms");
    }
}

#[test]
fn a_second_ask_of_an_open_source_is_an_arc_clone() {
    // "warm" is the second ask of a source that is open: the same shared snapshot and the same reports, not a read of the file again
    let opened = open(production(), miniature_path(miniature::V17_MOCK)).expect("the file opens");
    let source = opened.source();
    let first = source.build().expect("the metadata");
    let count = Arc::strong_count(&first);
    let second = source.build().expect("the metadata");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        Arc::strong_count(&first),
        count + 1,
        "one more holder of the one snapshot"
    );
    assert_eq!(
        source.layers().expect("the layers"),
        source.layers().expect("the layers")
    );
    assert!(Arc::ptr_eq(
        &source.validate().expect("the findings"),
        &source.validate().expect("the findings")
    ));
    assert!(Arc::ptr_eq(
        &source.availability().expect("the availability"),
        &source.availability().expect("the availability")
    ));
}
