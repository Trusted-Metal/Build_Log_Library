//! A build with no `Build_ID` is read, and the id is "not recorded" (IP-26 phase 26J, decision D26-17).
//!
//! The Vision team's rule is that a missing attribute must not break or block the software, and the library follows its own: a value is read as it is recorded, and
//! absent is absent (decision D02-4). `Build_ID` was the one attribute that both adapters refused to read without, so a file with none was not opened at all.
//!
//! * **Absent is `None`**, in V17 and in V16. Nothing else about the build changes.
//! * **An empty text is a value**, kept as it is (`Some("")`): the library does not decide that an empty id is no id.
//! * **A `Build_ID` that is present but is not text is still a fault**, with the attribute's name: it is what the file says, and it is not repaired.
//!
//! The small mounts are made at test time with the binding, and read with each adapter's own reader. The files that these tests write are their own and are
//! removed when the test ends.

mod common;

use build_log_io::adapters::v17;
use build_log_model::{BuildMeta, Error};
use common::{put_scalar, put_text, temp_h5};
use hdf5::Group;

/// A V17 mount with a build id and a layers' group, that `write` changes, read into a build.
fn v17_build(label: &str, write: impl FnOnce(&Group)) -> Result<BuildMeta, Error> {
    let on_disk = temp_h5(label, |mount| {
        put_text(mount, v17::layout::BUILD_ID, "a build");
        mount
            .create_group(v17::layout::MONITORING_DATA)
            .expect("the layers' group");
        write(mount);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    v17::read::read_build(&mount)
}

/// A V16 mount with a build id (and what the build level requires), that `write` changes, read into a build.
fn v16_build(label: &str, write: impl FnOnce(&Group)) -> Result<BuildMeta, Error> {
    common::v16::made(label, write)
}

#[test]
fn a_build_with_no_id_is_read_and_the_id_is_not_recorded_in_both_versions() {
    let remove = |mount: &Group| {
        mount
            .delete_attr(v17::layout::BUILD_ID)
            .expect("the attribute is removed");
    };
    let read_17 = v17_build("build-id-absent-v17", remove).expect("V17 reads a build with no id");
    let read_16 = v16_build("build-id-absent-v16", remove).expect("V16 reads a build with no id");
    assert_eq!(read_17.identity.build_id, None);
    assert_eq!(read_16.identity.build_id, None);

    // nothing else changes: the same mount with its id differs from this one in the id alone
    let with_id = v17_build("build-id-present-v17", |_| {}).expect("V17 reads it");
    let mut expected = with_id.clone();
    expected.identity.build_id = None;
    assert_eq!(read_17, expected);
}

#[test]
fn an_empty_build_id_is_a_value_and_is_kept_as_it_is_in_both_versions() {
    let empty = |mount: &Group| {
        mount
            .delete_attr(v17::layout::BUILD_ID)
            .expect("the attribute is removed");
        put_text(mount, v17::layout::BUILD_ID, "");
    };
    let read_17 = v17_build("build-id-empty-v17", empty).expect("V17 reads an empty id");
    let read_16 = v16_build("build-id-empty-v16", empty).expect("V16 reads an empty id");
    assert_eq!(read_17.identity.build_id.as_deref(), Some(""));
    assert_eq!(read_16.identity.build_id.as_deref(), Some(""));
}

#[test]
fn a_build_id_that_is_not_text_is_still_a_fault_that_names_the_attribute_in_both_versions() {
    let number = |mount: &Group| {
        mount
            .delete_attr(v17::layout::BUILD_ID)
            .expect("the attribute is removed");
        put_scalar(mount, v17::layout::BUILD_ID, 7i32);
    };
    for (version, result) in [
        ("V17", v17_build("build-id-number-v17", number)),
        ("V16", v16_build("build-id-number-v16", number)),
    ] {
        let error = result.expect_err(&format!("{version}: a number is not an id"));
        assert!(
            error.to_string().contains(v17::layout::BUILD_ID),
            "{version}: {error}"
        );
    }
}
