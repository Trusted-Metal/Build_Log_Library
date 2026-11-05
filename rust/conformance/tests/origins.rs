//! The two origins of a laser's stream (IP-07 phase 7A, task T07.1; decisions D07-1 and D07-2), held by the conformance check and shown to fail when they are wrong.
//!
//! * Where the channels of a laser in a layer differ in length, the laser's **stored samples** carry the origin `LongestChannel` (`stored_samples_origin` of the laser's entry for the layer).
//!   Where they agree there is none.
//! * A channel that is **recorded and empty because its dataspace is null** has the origin `NullDataspace` (`origin` of the channel's entry for the layer), on the laser's entry for that layer.
//!   A channel that is not null has none.
//!
//! The eight miniatures hold neither (their expectation files are unchanged), so the check on them is that nothing is said where nothing is expected; the made expectations here say both.

use std::fs;

use build_log_conformance::{check, expectations_dir, synthesise, Expectation};
use build_log_model::{BuildMeta, LaserKey, LayerId, Origin, Origins};
use serde_json::Value;

const STREAM: &str = "LongestChannel";
const NULL: &str = "NullDataspace";

/// The expectation of the mock miniature, changed by `change`, parsed.
fn mock(change: impl FnOnce(&mut Value)) -> Expectation {
    let text = fs::read_to_string(expectations_dir().join("v17_mock.json")).expect("the file");
    let mut root: Value = serde_json::from_str(&text).expect("JSON");
    change(&mut root);
    Expectation::parse(&root.to_string()).expect("an expectation")
}

/// The first laser's first layer and its power: the stream says its channels differ, and the power says it is null.
fn with_both_origins(root: &mut Value) {
    let laser = &mut root["lasers"][0];
    assert_eq!(laser["key"], "Laser_1");
    laser["layers"][0]["stored_samples_origin"] = Value::String(STREAM.to_string());
    let power = laser["channels"]
        .as_array_mut()
        .expect("channels")
        .iter_mut()
        .find(|c| c["source_name"] == "Laser_Power")
        .expect("the power");
    assert_eq!(power["layers"][0]["layer"], 1);
    power["layers"][0]["origin"] = Value::String(NULL.to_string());
}

fn laser_one() -> LaserKey {
    LaserKey::new("Laser_1", 1)
}

/// The origins without the one of `field`.
fn without(origins: &Origins, field: &str) -> Origins {
    let mut kept = Origins::new();
    for (name, origin) in origins.iter() {
        if name != field {
            kept.record(name, origin.clone());
        }
    }
    kept
}

/// Edits the first laser's entry for the first layer in a copy of the build.
fn edited(meta: &BuildMeta, edit: impl FnOnce(&mut build_log_model::LaserLayerMeta)) -> BuildMeta {
    let mut copy = meta.clone();
    let laser = copy.laser_mut(&laser_one()).expect("the laser");
    edit(laser.layers.get_mut(LayerId(1)).expect("the layer"));
    copy
}

#[test]
fn the_origins_the_expectation_states_are_in_the_synthetic_build_and_the_check_holds_them() {
    let expectation = mock(with_both_origins);
    let meta = synthesise(&expectation).expect("a build");
    let entry = meta
        .laser(&laser_one())
        .and_then(|l| l.layers.get(LayerId(1)))
        .expect("the entry");
    assert_eq!(
        entry.origins.get("stored_samples"),
        Some(&Origin::LongestChannel)
    );
    assert_eq!(
        entry.origins.get("channel Laser_Power"),
        Some(&Origin::NullDataspace)
    );
    assert_eq!(check(&meta, &expectation), Vec::<String>::new());
}

#[test]
fn an_origin_that_is_missing_changed_or_not_expected_is_reported_on_its_own_laser_layer_and_channel(
) {
    let expectation = mock(with_both_origins);
    let meta = synthesise(&expectation).expect("a build");

    // each way of getting one wrong is reported once, in its own place
    let cases: Vec<(&str, BuildMeta, &str)> = vec![
        (
            "the stream's origin missing",
            edited(&meta, |l| {
                l.origins = without(&l.origins, "stored_samples");
            }),
            "in layer 1, where its stored samples came from",
        ),
        (
            "the stream's origin in other words",
            edited(&meta, |l| {
                l.origins
                    .record("stored_samples", Origin::Other("the power".to_string()));
            }),
            "in layer 1, where its stored samples came from",
        ),
        (
            "the stream's origin of another kind",
            edited(&meta, |l| {
                l.origins.record("stored_samples", Origin::Inferred);
            }),
            "in layer 1, where its stored samples came from",
        ),
        (
            "the channel's origin missing",
            edited(&meta, |l| {
                l.origins = without(&l.origins, "channel Laser_Power");
            }),
            "where it came from in layer 1",
        ),
        (
            "the channel's origin in other words",
            edited(&meta, |l| {
                l.origins
                    .record("channel Laser_Power", Origin::Other("empty".to_string()));
            }),
            "where it came from in layer 1",
        ),
        (
            "another channel with an origin that is not expected",
            edited(&meta, |l| {
                l.origins.record("channel Actual_X", Origin::NullDataspace);
            }),
            "Actual",
        ),
    ];
    for (what, changed, place) in cases {
        let found = check(&changed, &expectation);
        assert_eq!(found.len(), 1, "{what}: {found:?}");
        assert!(found[0].contains(place), "{what}: {found:?}");
    }
}

#[test]
fn an_origin_that_the_expectation_does_not_state_is_reported() {
    // the unchanged expectation of the mock says nothing, so a build that carries either origin does not conform to it: nothing is said where nothing is expected
    let plain = mock(|_| {});
    let with_origins = synthesise(&mock(with_both_origins)).expect("a build");
    let found = check(&with_origins, &plain);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found
        .iter()
        .any(|l| l.contains("where its stored samples came from")));
    assert!(found
        .iter()
        .any(|l| l.contains("Power") && l.contains("where it came from in layer 1")));
    // and the build made from the unchanged expectation conforms to it, as before
    let meta = synthesise(&plain).expect("a build");
    assert_eq!(check(&meta, &plain), Vec::<String>::new());
}
