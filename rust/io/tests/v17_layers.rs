//! The layers of a V17 build (IP-13 phase 13C, task T13.4; decisions D13-4, D13-5, D13-6 and D13-15).
//!
//! * A layer's id is the number in its group's name; the ids come out in numeric order and may have gaps.
//! * Times are the integers as written, in no unit.
//! * A layer has two thicknesses, the actual and the commanded; each is read on its own, and one that is the placeholder of V17 (`0.0`) is not recorded and keeps
//!   its raw value in its own origin. An attribute the file does not have says nothing.
//! * What else a layer's group holds is kept in the layer's `extra`.
//! * The layers of the gate file, and of the special-layer miniature, conform to their expectation files.
//!
//! The synthetic mounts are made at test time with the binding, and read with the layer reader itself.

mod common;

use build_log_conformance::{check_parts, expectations_dir, Expectation, Part};
use build_log_io::adapters::v17::{layers::read_layers, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    AttrValue, BuildIdentity, BuildMeta, Error, ErrorKind, LayerId, Origin, UnitSource,
};
use build_log_testkit::{miniature, miniature_path};
use common::{put_array, put_scalar, put_text, temp_h5};

fn empty_build() -> BuildMeta {
    BuildMeta::new(BuildIdentity {
        build_id: "a build".into(),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    })
}

/// A mount that holds a layers' group, filled by `write`, read into a build; the result is the build or the error.
fn read_mount(label: &str, write: impl FnOnce(&hdf5::Group)) -> Result<BuildMeta, Error> {
    let on_disk = temp_h5(label, |mount| {
        let monitoring = mount
            .create_group(layout::MONITORING_DATA)
            .expect("the layers' group");
        write(&monitoring);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let mut build = empty_build();
    read_layers(&mount, &mut build).map(|()| build)
}

fn layer(group: &hdf5::Group, id: u32) -> hdf5::Group {
    group
        .create_group(&format!("{}{id}", layout::LAYER_PREFIX))
        .expect("a layer's group")
}

fn expectation(name: &str) -> Expectation {
    Expectation::load(&expectations_dir().join(name)).expect("an expectation file")
}

// ---------------------------------------------------------------- ids

#[test]
fn layer_ids_come_from_the_group_names_and_may_have_gaps() {
    let build = read_mount("v17-layer-ids", |monitoring| {
        // created out of order, with gaps, and with names that are not a layer's
        for id in [2007u32, 2, 10, 1964] {
            layer(monitoring, id);
        }
        monitoring.create_group("Notes").expect("another group");
        monitoring
            .create_group("layer_5")
            .expect("a name in another case");
        monitoring
            .create_group(&format!("{}x", layout::LAYER_PREFIX))
            .expect("a prefix and no digits");
        monitoring
            .create_group(layout::LAYER_PREFIX)
            .expect("a prefix and nothing");
    })
    .expect("the layers are read");
    // in numeric order (10 comes after 2, not after 1), only the layers, and nothing in between
    let ids: Vec<u32> = build.layers.ids().map(|id| id.0).collect();
    assert_eq!(ids, [2, 10, 1964, 2007]);
    assert_eq!(build.layers_present(), 4);
    // declared is what the file says and is not derived from what it holds
    assert_eq!(build.layers_declared, None);

    // two groups that name the same layer are refused, and so are digits that are not a layer id
    let twice = read_mount("v17-layer-twice", |monitoring| {
        layer(monitoring, 7);
        monitoring
            .create_group(&format!("{}007", layout::LAYER_PREFIX))
            .expect("the same layer, spelled with zeros");
    })
    .expect_err("one layer cannot have two groups");
    assert!(
        matches!(twice.kind(), ErrorKind::Corrupt { .. }),
        "{:?}",
        twice.kind()
    );
    let huge = read_mount("v17-layer-huge", |monitoring| {
        monitoring
            .create_group(&format!("{}99999999999", layout::LAYER_PREFIX))
            .expect("a number that is not a layer id");
    })
    .expect_err("an id that does not fit");
    assert!(
        matches!(huge.kind(), ErrorKind::OutOfWidth { .. }),
        "{:?}",
        huge.kind()
    );

    // the real file: layers 1964 to 1967 and 2006 to 2008 of a build that declares 2,110
    let special = open(production(), miniature_path(miniature::V17_SPECIAL_LAYERS))
        .expect("the special-layer miniature opens");
    let build = special.source().build().expect("the metadata");
    let ids: Vec<u32> = build.layers.ids().map(|id| id.0).collect();
    assert_eq!(ids, [1964, 1965, 1966, 1967, 2006, 2007, 2008]);
    assert_eq!(build.layers_declared, Some(2110));
    assert_eq!(build.layers_present(), 7);
}

#[test]
fn a_mount_without_its_layers_group_is_corrupt_and_not_a_panic() {
    let on_disk = temp_h5("v17-no-layers", |_mount| {});
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let mut build = empty_build();
    let error = read_layers(&mount, &mut build).expect_err("a build with no layers' group");
    match error.kind() {
        ErrorKind::Corrupt { detail } => {
            assert!(detail.contains(layout::MONITORING_DATA), "{detail}")
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

// ---------------------------------------------------------------- times

#[test]
fn layer_times_are_the_integers_as_written() {
    let build = read_mount("v17-layer-times", |monitoring| {
        let first = layer(monitoring, 1);
        put_scalar(&first, layout::START_LAYER_TIME, 1_792_440_977_031i64);
        put_scalar(&first, layout::END_LAYER_TIME, 1_792_440_991_421i64);
        // the same number stored as an array of one, and one stored as microseconds' worth: no unit is stated or decided
        let second = layer(monitoring, 2);
        put_array(
            &second,
            layout::START_LAYER_TIME,
            &[1_774_042_974_383_273i64],
        );
        put_scalar(&second, layout::END_LAYER_TIME, 1_774_043_017_296_816i64);
        // a layer that has no times has none
        layer(monitoring, 3);
    })
    .expect("the layers are read");
    let first = build.layers.get(LayerId(1)).unwrap();
    assert_eq!(
        first.start.as_ref().map(|t| t.value),
        Some(1_792_440_977_031)
    );
    assert_eq!(first.end.as_ref().map(|t| t.value), Some(1_792_440_991_421));
    let second = build.layers.get(LayerId(2)).unwrap();
    assert_eq!(
        second.start.as_ref().map(|t| t.value),
        Some(1_774_042_974_383_273)
    );
    for time in [&first.start, &first.end, &second.start, &second.end] {
        let time = time.as_ref().unwrap();
        assert_eq!(
            time.unit_source,
            UnitSource::Unknown,
            "no file says what unit a layer time is in"
        );
        assert!(time.unit.spelling.is_none());
    }
    let third = build.layers.get(LayerId(3)).unwrap();
    assert!(third.start.is_none() && third.end.is_none());

    // a time that does not fit is an error that says so, and not a wrong number
    let too_big = read_mount("v17-layer-time-width", |monitoring| {
        put_scalar(&layer(monitoring, 1), layout::START_LAYER_TIME, u64::MAX);
    })
    .expect_err("a time of 64 unsigned bits");
    assert!(
        matches!(too_big.kind(), ErrorKind::OutOfWidth { .. }),
        "{:?}",
        too_big.kind()
    );
    // and an error in a layer says which layer
    assert_eq!(
        too_big.location().layer,
        Some(LayerId(1)),
        "the error names the layer it happened in"
    );
}

// ---------------------------------------------------------------- the two thicknesses

#[test]
fn a_placeholder_thickness_is_not_recorded_and_keeps_its_raw_value() {
    let build = read_mount("v17-layer-thickness", |monitoring| {
        // both are the placeholder of V17
        let first = layer(monitoring, 1);
        put_array(&first, layout::ACTUAL_LAYER_THICKNESS, &[0.0f32]);
        put_array(&first, layout::COMMANDED_LAYER_THICKNESS, &[0.0f32]);
        // one is real and the other is the placeholder: each is reported on its own
        let second = layer(monitoring, 2);
        put_array(&second, layout::ACTUAL_LAYER_THICKNESS, &[0.03f32]);
        put_array(&second, layout::COMMANDED_LAYER_THICKNESS, &[0.0f32]);
        // the other way round, and the actual attribute is not there at all
        let third = layer(monitoring, 3);
        put_scalar(&third, layout::COMMANDED_LAYER_THICKNESS, 0.03f32);
        // 1.0 is a placeholder of V16 and a real thickness in V17
        let fourth = layer(monitoring, 4);
        put_array(&fourth, layout::ACTUAL_LAYER_THICKNESS, &[1.0f32]);
        // no thickness attribute at all
        layer(monitoring, 5);
    })
    .expect("the layers are read");
    let dropped = Origin::PlaceholderDropped { raw: "0.0".into() };
    let of = |id: u32| build.layers.get(LayerId(id)).unwrap();

    let first = of(1);
    assert!(first.actual_thickness.is_none() && first.commanded_thickness.is_none());
    assert_eq!(first.origins.get("actual_thickness"), Some(&dropped));
    assert_eq!(first.origins.get("commanded_thickness"), Some(&dropped));

    let second = of(2);
    let actual = second.actual_thickness.as_ref().expect("a real thickness");
    assert_eq!(actual.value, f64::from(0.03f32));
    assert!(
        !actual.unit.is_known(),
        "V17 states no unit for a thickness"
    );
    assert!(second.commanded_thickness.is_none());
    assert_eq!(second.origins.get("actual_thickness"), None);
    assert_eq!(second.origins.get("commanded_thickness"), Some(&dropped));

    let third = of(3);
    assert!(third.actual_thickness.is_none());
    assert_eq!(
        third.origins.get("actual_thickness"),
        None,
        "an attribute the file does not have says nothing"
    );
    assert_eq!(
        third.commanded_thickness.as_ref().map(|t| t.value),
        Some(f64::from(0.03f32))
    );

    assert_eq!(of(4).actual_thickness.as_ref().map(|t| t.value), Some(1.0));
    let fifth = of(5);
    assert!(
        fifth.actual_thickness.is_none()
            && fifth.commanded_thickness.is_none()
            && fifth.origins.is_empty()
    );
}

// ---------------------------------------------------------------- what else a layer holds

#[test]
fn the_attributes_of_a_layer_that_are_not_typed_are_kept() {
    let build = read_mount("v17-layer-extra", |monitoring| {
        let group = layer(monitoring, 1);
        put_scalar(&group, layout::START_LAYER_TIME, 5i64);
        put_text(&group, "A_Note", "kept as written");
        put_array(&group, "A_Count", &[3i32]);
    })
    .expect("the layers are read");
    let layer = build.layers.get(LayerId(1)).unwrap();
    // the typed attribute is not kept twice, and the others are kept with their form
    assert!(layer.extra.attribute(layout::START_LAYER_TIME).is_none());
    assert_eq!(layer.extra.len(), 2);
    assert_eq!(
        layer.extra.attribute("A_Note").unwrap().value,
        AttrValue::Text(vec!["kept as written".into()])
    );
    assert_eq!(
        layer.extra.attribute("A_Count").unwrap().value,
        AttrValue::Signed(vec![3])
    );
}

// ---------------------------------------------------------------- the real files

#[test]
fn the_layers_of_the_gate_file_conform() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("the gate file opens");
    let build = opened.source().build().expect("the metadata");
    let found = check_parts(
        &build,
        &expectation("v17_triggered_2026_10_19.json"),
        &[Part::Build, Part::Layers],
    );
    assert!(
        found.is_empty(),
        "the layers do not conform:\n  {}",
        found.join("\n  ")
    );
    // a real file's layer has only the four attributes that the layer level types: nothing is left over
    assert!(build.layers.get(LayerId(1)).unwrap().extra.is_empty());
}

#[test]
fn the_layers_of_the_special_layer_miniature_conform() {
    let opened = open(production(), miniature_path(miniature::V17_SPECIAL_LAYERS))
        .expect("the miniature opens");
    let build = opened.source().build().expect("the metadata");
    // the lasers are not read yet, so the part is the layers: their ids (with gaps), times and both thicknesses
    let found = check_parts(
        &build,
        &expectation("special_layers_v17.json"),
        &[Part::Layers],
    );
    assert!(
        found.is_empty(),
        "the layers do not conform:\n  {}",
        found.join("\n  ")
    );
}
