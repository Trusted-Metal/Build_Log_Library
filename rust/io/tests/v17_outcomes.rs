//! How a V17 build ended, what was done to it after capture, and whether each laser fired (IP-13 phase 13J, task T13.12; decisions J9, J14, D13-21 and D13-24).
//!
//! * `Build_Status` is the build's outcome; `Opcua_Join_Status` and `Opcua_Join_Version` are provenance. Only the words that tauri writes are named, matched
//!   exactly; any other text is kept as the file spells it, and a file with none says nothing.
//! * A laser's `complete` flag is kept as written in each layer and is **never worked out from its samples**; a value that is not 0 or 1 is a fault in the
//!   laser's group and is reported on that laser.
//! * The reader declares that it returns outcomes.

mod common;

use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildMeta, Direction, Feature, JoinStatus, LaserKey, LayerId, Origin, Outcome, Presence,
};
use build_log_testkit::{miniature, miniature_path};
use common::{laser, layer, monitoring, put_array, put_scalar, put_text, temp_h5, TempH5};

fn build_of(file: &str) -> BuildMeta {
    let opened = open(production(), miniature_path(file)).expect("the file opens");
    (*opened.source().build().expect("the metadata")).clone()
}

/// A file with a mount that has the build id and a first layer, filled further by `write`, read as a V17 build.
fn synthetic(label: &str, write: impl FnOnce(&hdf5::Group, &hdf5::Group)) -> BuildMeta {
    let on_disk: TempH5 = temp_h5(label, |mount| {
        put_text(mount, layout::BUILD_ID, "a build");
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        write(mount, &first);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    v17::read::read_build(&mount).expect("the build reads")
}

fn laser_key(number: u32) -> LaserKey {
    LaserKey::new(format!("{}{number}", layout::LASER_PREFIX), number)
}

fn flag_of(build: &BuildMeta, number: u32, layer: u32) -> Option<bool> {
    build
        .laser(&laser_key(number))
        .expect("the laser")
        .layers
        .get(LayerId(layer))
        .expect("the laser in the layer")
        .complete
}

// ---------------------------------------------------------------- the build's outcome

#[test]
fn build_status_becomes_the_builds_outcome() {
    // the two files that tauri wrote with its OPC UA output
    for file in [
        miniature::V17_TRIGGERED_2026_10_19,
        miniature::V17_2026_08_18_IMAGES,
    ] {
        assert_eq!(build_of(file).quality.outcome, Outcome::Complete, "{file}");
    }
    // the others have no status, and that is not an outcome of any kind
    for file in [
        miniature::V17_MOCK,
        miniature::V17_OCT15,
        miniature::V17_MARCH,
        miniature::V17_SPECIAL_LAYERS,
    ] {
        assert_eq!(
            build_of(file).quality.outcome,
            Outcome::NotRecorded,
            "{file}"
        );
    }
    // the word that tauri writes when it creates the file is named; any other text is kept as written, in whatever case
    for (text, wanted) in [
        (layout::STATUS_IN_PROGRESS, Outcome::InProgress),
        (layout::STATUS_COMPLETE, Outcome::Complete),
        ("Complete", Outcome::Other("Complete".into())),
        (
            "failed: disk full",
            Outcome::Other("failed: disk full".into()),
        ),
        ("", Outcome::Other(String::new())),
    ] {
        let build = synthetic("outcome-words", |mount, _| {
            put_text(mount, layout::BUILD_STATUS, text)
        });
        assert_eq!(build.quality.outcome, wanted, "{text:?}");
    }
}

#[test]
fn the_join_status_and_version_become_provenance() {
    let gate = build_of(miniature::V17_TRIGGERED_2026_10_19);
    assert_eq!(
        gate.provenance.opcua_join_status,
        Some(JoinStatus::Complete)
    );
    assert_eq!(gate.provenance.opcua_join_version.as_deref(), Some("2"));
    // a file with no join says nothing: not a join that failed
    let mock = build_of(miniature::V17_MOCK);
    assert_eq!(mock.provenance.opcua_join_status, None);
    assert_eq!(mock.provenance.opcua_join_version, None);
    // a status that is not the one word is kept as written, and the version is text, whatever it spells
    let build = synthetic("join-words", |mount, _| {
        put_text(mount, layout::OPCUA_JOIN_STATUS, "partial");
        put_text(mount, layout::OPCUA_JOIN_VERSION, "1");
    });
    assert_eq!(
        build.provenance.opcua_join_status,
        Some(JoinStatus::Other("partial".into()))
    );
    assert_eq!(build.provenance.opcua_join_version.as_deref(), Some("1"));
}

#[test]
fn what_is_typed_leaves_extra() {
    let gate = build_of(miniature::V17_TRIGGERED_2026_10_19);
    for name in [
        layout::BUILD_STATUS,
        layout::OPCUA_JOIN_STATUS,
        layout::OPCUA_JOIN_VERSION,
    ] {
        assert!(gate.extra.attribute(name).is_none(), "{name}");
    }
    let entry = gate
        .laser(&laser_key(1))
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap();
    assert!(entry.extra.attribute(layout::COMPLETE).is_none());
}

// ---------------------------------------------------------------- the laser's flag

#[test]
fn the_complete_flag_is_kept_as_written_and_never_derived() {
    // the real files: both lasers fired in the gate file; in the 2026-10-15 file the second laser received nothing and says so
    let gate = build_of(miniature::V17_TRIGGERED_2026_10_19);
    assert_eq!(
        (flag_of(&gate, 1, 1), flag_of(&gate, 2, 1)),
        (Some(true), Some(true))
    );
    let oct15 = build_of(miniature::V17_OCT15);
    assert_eq!(
        (flag_of(&oct15, 1, 1), flag_of(&oct15, 2, 1)),
        (Some(true), Some(false))
    );
    // the March files have no flag, which is not false
    for file in [miniature::V17_MARCH, miniature::V17_SPECIAL_LAYERS] {
        let build = build_of(file);
        for number in [1, 2] {
            let flags: Vec<_> = build
                .laser(&laser_key(number))
                .unwrap()
                .layers
                .iter()
                .map(|(_, entry)| entry.complete)
                .collect();
            assert!(flags.iter().all(Option::is_none), "{file}, laser {number}");
        }
    }
    // made to disagree with its samples: a laser with samples and a 0, one with none and a 1, one with no flag at all, and one that stores it as a scalar
    let build = synthetic("flags-disagree", |_, first| {
        let one = laser(first, 1, 5, None);
        put_array(&one, layout::COMPLETE, &[0u8]);
        let two = laser(first, 2, 0, None);
        put_array(&two, layout::COMPLETE, &[1u8]);
        laser(first, 3, 5, None);
        let four = laser(first, 4, 5, None);
        put_scalar(&four, layout::COMPLETE, 1u8);
    });
    let at = |number| {
        build
            .laser(&laser_key(number))
            .unwrap()
            .layers
            .get(LayerId(1))
            .unwrap()
    };
    assert_eq!(
        (at(1).presence.samples(), at(1).complete),
        (Some(5), Some(false))
    );
    assert_eq!(
        (at(2).presence, at(2).complete),
        (Presence::RecordedEmpty, Some(true))
    );
    assert_eq!((at(3).presence.samples(), at(3).complete), (Some(5), None));
    assert_eq!(
        (at(4).presence.samples(), at(4).complete),
        (Some(5), Some(true))
    );
}

#[test]
fn a_flag_that_is_neither_0_nor_1_is_a_fault_in_that_laser_and_the_build_opens() {
    let build = synthetic("flag-two", |_, first| {
        laser(first, 1, 5, None);
        let bad = laser(first, 2, 5, None);
        put_array(&bad, layout::COMPLETE, &[2u8]);
    });
    // the other laser is untouched
    assert_eq!(flag_of(&build, 1, 1), None);
    let entry = build
        .laser(&laser_key(2))
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap();
    // the laser is reported on itself: not recorded in that layer, with the reason, and its group is kept whole
    assert_eq!(entry.presence, Presence::NotRecorded);
    assert_eq!(entry.complete, None);
    match entry.origins.get("laser") {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, build_log_model::NotReadCause::Fault);
            assert!(reason.contains("only 0 and 1"), "{reason}");
        }
        other => panic!("expected the reason, got {other:?}"),
    }
    let layer_one = build.layers.get(LayerId(1)).unwrap();
    assert!(layer_one
        .extra
        .node(&[format!("{}2", layout::LASER_PREFIX)])
        .is_some());
}

// ---------------------------------------------------------------- the capabilities

#[test]
fn outcomes_are_declared_in_the_capabilities() {
    let set = common::v17_set();
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Outcomes));
    // nothing is declared in the write direction (the exact list of what is read is held by the registration test)
    assert!(set.capabilities().features(Direction::Write).is_empty());
    // a file with no status still reads through an adapter that declares outcomes: the answer is "not recorded", which is an answer
    assert_eq!(
        build_of(miniature::V17_MOCK).quality.outcome,
        Outcome::NotRecorded
    );
}
