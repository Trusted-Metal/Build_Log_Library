//! The test kit seen from another crate: a miniature is found from here as from anywhere (IP-01 task T01.6).

use build_log_testkit::{miniature, miniature_path, testdata_dir};

#[test]
fn miniature_path_resolves_from_any_crate() {
    // the expected path is worked out here, from this crate's own place in the workspace, and must equal what the kit says
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("two folders below the repository root");
    assert_eq!(testdata_dir(), root.join("testdata"));
    assert_eq!(
        miniature_path(miniature::V16),
        root.join("testdata").join("v16.h5")
    );
    assert_eq!(
        miniature_path(miniature::V17_SPECIAL_LAYERS),
        root.join("testdata").join("special_layers_v17.h5")
    );
}

#[test]
fn the_miniature_names_are_the_files_that_are_committed() {
    // testdata/SHA256SUMS lists the approved files; each constant of the kit must be one of them (skips when the folder is not there)
    let sums = testdata_dir().join("SHA256SUMS");
    build_log_testkit::skip_unless!(sums.is_file(), "{} is not there", sums.display());
    let text = std::fs::read_to_string(&sums).expect("SHA256SUMS can be read");
    for name in [
        miniature::V16,
        miniature::V17_MARCH,
        miniature::V17_MOCK,
        miniature::V17_OCT15,
        miniature::V18,
        miniature::V17_SPECIAL_LAYERS,
    ] {
        assert!(
            text.lines()
                .any(|l| l.trim_end_matches('\r').ends_with(&format!("  {name}"))),
            "{name} is not listed in testdata/SHA256SUMS"
        );
        assert!(
            miniature_path(name).is_file(),
            "{name} is listed but not in testdata/"
        );
    }
}
