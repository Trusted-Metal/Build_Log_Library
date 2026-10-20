//! The test kit's own tests: where the miniatures and the large fixtures are found, how a test skips, and that no Rust code reads the originals.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use build_log_testkit::{
    fixtures_dir_from, large_fixture_in, miniature, miniature_path, repo_root, skip_message,
    skip_unless, testdata_dir, FIXTURES_ENV, MINIATURE_FILE_LIMIT_BYTES,
    MINIATURE_TOTAL_LIMIT_BYTES,
};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh empty folder under the system temporary folder; the caller removes it.
fn scratch_dir(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "build-log-testkit-{}-{}-{label}",
        std::process::id(),
        n
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

#[test]
fn large_fixture_is_none_when_the_variable_is_unset() {
    // Unset (or empty) means the default `<repo>/fixtures`. In a checkout without that folder nothing is found, so the answer is `None`.
    let bare_repo = scratch_dir("bare-repo");
    assert_eq!(
        fixtures_dir_from(None, &bare_repo),
        bare_repo.join("fixtures")
    );
    assert_eq!(
        fixtures_dir_from(Some(""), &bare_repo),
        bare_repo.join("fixtures")
    );
    assert_eq!(
        large_fixture_in(
            &fixtures_dir_from(None, &bare_repo),
            "Build Log Fixtures/V18_fixture.h5"
        ),
        None
    );

    // Set: the folder it names, and a file is found only when it is there.
    let folder = scratch_dir("fixtures");
    assert_eq!(
        fixtures_dir_from(Some(folder.to_str().expect("a UTF-8 path")), &bare_repo),
        folder
    );
    assert_eq!(large_fixture_in(&folder, "extracts/v16_layer_1.h5"), None);
    fs::create_dir_all(folder.join("extracts")).expect("a folder");
    fs::write(folder.join("extracts/v16_layer_1.h5"), b"x").expect("a file");
    assert_eq!(
        large_fixture_in(&folder, "extracts/v16_layer_1.h5"),
        Some(folder.join("extracts/v16_layer_1.h5"))
    );
    assert_eq!(
        large_fixture_in(&folder, "extracts"),
        None,
        "a folder is not a fixture file"
    );
    let _ = fs::remove_dir_all(&bare_repo);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn the_variable_is_named_as_the_python_tools_name_it() {
    assert_eq!(FIXTURES_ENV, "BUILD_LOG_FIXTURES_DIR");
}

#[test]
fn the_budget_constants_are_the_ones_the_owner_approved() {
    // IP-00 decision D00-5: at most 12 MB a file and 50 MB in all
    assert_eq!(
        (MINIATURE_FILE_LIMIT_BYTES, MINIATURE_TOTAL_LIMIT_BYTES),
        (12_000_000, 50_000_000)
    );
}

#[test]
fn miniature_paths_are_absolute_and_inside_testdata() {
    let root = repo_root();
    assert!(
        root.join("Cargo.toml").is_file() && root.join("rust").join("testkit").is_dir(),
        "{} is not the repository root",
        root.display()
    );
    assert_eq!(testdata_dir(), root.join("testdata"));
    let path = miniature_path(miniature::V16);
    assert!(path.is_absolute());
    assert_eq!(path, root.join("testdata").join("v16.h5"));
    assert_eq!(
        miniature_path("anything.h5").parent(),
        Some(testdata_dir().as_path())
    );
}

static REACHED: AtomicBool = AtomicBool::new(false);

fn stops_unless(condition: bool) {
    skip_unless!(condition, "the condition is {condition}");
    REACHED.store(true, Ordering::SeqCst);
}

#[test]
fn skip_unless_stops_the_test_and_names_its_reason() {
    stops_unless(false);
    assert!(
        !REACHED.load(Ordering::SeqCst),
        "the code after a failed skip_unless! must not run"
    );
    stops_unless(true);
    assert!(REACHED.load(Ordering::SeqCst));
    assert_eq!(skip_message("no fixtures"), "SKIPPED: no fixtures");
}

fn rust_sources_and_manifests(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("a readable folder") {
        let path = entry.expect("a folder entry").path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n != "target" && n != ".git")
            {
                rust_sources_and_manifests(&path, found);
            }
        } else if path.extension().is_some_and(|e| e == "rs" || e == "toml") {
            found.push(path);
        }
    }
}

#[test]
fn no_rust_code_names_the_originals_variable() {
    // Only the Python fixture tools of IP-00 read the two untrimmed originals (decision D00-9). The name is built here so that this file does not match itself.
    let name = ["BUILD_LOG", "ORIGINALS", "DIR"].join("_");
    let mut files = Vec::new();
    rust_sources_and_manifests(&repo_root().join("rust"), &mut files);
    assert!(
        files.len() > 8,
        "found only {} Rust files: is the search looking in the right place?",
        files.len()
    );
    let offenders: Vec<_> = files
        .iter()
        .filter(|p| {
            fs::read_to_string(p)
                .map(|text| text.contains(&name))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "no Rust code may read the originals; found the variable in {offenders:?}"
    );
}
