//! The library's miniatures, for a consumer's own tests (IP-26 phase 26L).
//!
//! * Every named miniature is a file in the folder that `testing::miniature_dir` finds, and the names are the testkit's (the library's own tests use the testkit, so the two lists
//!   cannot drift apart without this test failing).
//! * A consumer can open every miniature but the V18 draft through the facade, by the path that `testing::miniature_path` gives.

use build_log::testing::{miniature, miniature_dir, miniature_path};

#[test]
fn every_named_miniature_is_in_the_folder() {
    assert!(
        miniature_dir().is_dir(),
        "{} is not a folder",
        miniature_dir().display()
    );
    for name in miniature::ALL {
        let path = miniature_path(name);
        assert!(path.is_file(), "{} is not a file", path.display());
    }
}

#[test]
fn the_names_are_the_testkits_and_all_lists_them_once() {
    use build_log_testkit::miniature as kit;
    let mut ours = miniature::ALL.to_vec();
    let mut theirs = vec![
        kit::V16,
        kit::V17_MARCH,
        kit::V17_MOCK,
        kit::V17_OCT15,
        kit::V17_TRIGGERED_2026_10_19,
        kit::V17_2026_08_18_IMAGES,
        kit::V17_SPECIAL_LAYERS,
        kit::V18,
    ];
    ours.sort_unstable();
    theirs.sort_unstable();
    assert_eq!(
        ours, theirs,
        "the facade's miniatures and the testkit's differ"
    );
    ours.dedup();
    assert_eq!(ours.len(), 8, "a name is listed twice");
    // the folder is the testkit's too
    assert_eq!(miniature_dir(), build_log_testkit::testdata_dir());
}

#[test]
fn a_consumer_opens_every_miniature_but_the_draft_by_its_path() {
    for name in miniature::ALL {
        if name == miniature::V18 {
            continue;
        }
        let opened = build_log::open(miniature_path(name))
            .unwrap_or_else(|error| panic!("{name} does not open: {error}"));
        assert!(
            opened.source().overview().is_ok(),
            "{name}: the overview is not read"
        );
    }
}
