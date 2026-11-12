//! A file with no `Build_ID` opens through the facade, and the id is "not recorded" (IP-26 phase 26J, decision D26-17).
//!
//! Before it, the library refused such a file with a typed error while Vision's own loader opened it. This shows, on a **scratch copy of each miniature that the facade opens**
//! (V16 and the six of V17) with the attribute removed, that:
//!
//! * the file opens, and the id is `None`, which is "not recorded";
//! * **nothing else changes**: the build read from the copy equals the build read from the miniature, except for the id.
//!
//! The miniatures themselves are never opened for writing. Each copy is made in the temporary directory and only that copy is removed when the test ends. The attribute
//! sits on the mount that V16 and V17 share, the group `Built_Part`.

#![cfg(feature = "hdf5")]

use std::fs;
use std::path::PathBuf;

use build_log::{hdf5, open};
use build_log_testkit::{miniature, miniature_path};

/// The miniatures that the facade opens: V16 and the six of V17.
const OPENABLE: [&str; 7] = [
    miniature::V16,
    miniature::V17_MARCH,
    miniature::V17_MOCK,
    miniature::V17_OCT15,
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
    miniature::V17_SPECIAL_LAYERS,
];

/// A copy of a miniature in the temporary directory, removed when the value is dropped. Nothing but the copy is ever written to or removed.
struct ScratchCopy(PathBuf);

impl Drop for ScratchCopy {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn copy_of(miniature_name: &str) -> ScratchCopy {
    let path = std::env::temp_dir().join(format!(
        "build-log-26j-{}-{miniature_name}",
        std::process::id()
    ));
    fs::copy(miniature_path(miniature_name), &path).expect("the miniature is copied");
    ScratchCopy(path)
}

#[test]
fn a_miniature_with_no_build_id_opens_and_only_the_id_differs() {
    for name in OPENABLE {
        let original = open(miniature_path(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let original = original
            .source()
            .build()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            original.identity.build_id.is_some(),
            "{name}: a miniature records its id"
        );

        let copy = copy_of(name);
        {
            let file = hdf5::File::open_rw(&copy.0).expect("the copy opens for writing");
            file.group("Built_Part")
                .expect("the mount")
                .delete_attr("Build_ID")
                .expect("the attribute is removed from the copy");
        }
        let opened = open(&copy.0)
            .unwrap_or_else(|e| panic!("{name}: a file with no Build_ID is refused: {e}"));
        let found = opened
            .source()
            .build()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(found.identity.build_id, None, "{name}");

        let mut expected = (*original).clone();
        expected.identity.build_id = None;
        assert_eq!(*found, expected, "{name}: nothing but the id changes");
    }
}
