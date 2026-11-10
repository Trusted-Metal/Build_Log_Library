//! The V16 adapter, part 1 (IP-12 phase 12C, task T12.3; decisions D12-2, D12-3, D12-8, D12-9 and D12-11).
//!
//! * The adapter is registered under `"16"`, with the place of its marker, **no capability yet** (decision D13-7: nothing is declared that the reader does not return, so `Samples` joins with
//!   phase 12F), a reader, and a writer and patcher declared unsupported for good; `create(16)` is `UnsupportedInVersion`.
//! * The build level, the context and the layers of the V16 miniature conform to `v16.json` (`Part::Build`, `Part::Context`, `Part::Layers`), the date is the string as written, a layer's times
//!   have no unit, and **nothing the file holds is dropped**: it is typed or kept in `extra`.
//! * **V16's placeholder is `1.0`**, where V17's is `0.0`: a thickness, a starting height, a plate's thickness or weight of `1.0` is not recorded and keeps its raw spelling, and `0.0` is a value.
//! * A request for samples answers `UnsupportedInVersion` until phase 12F.
//!
//! The small mounts are made at test time with the binding, and read with the adapter's own reader.

mod common;

use std::collections::BTreeSet;

use build_log_conformance::{check_parts, expectations_dir, Expectation, Part};
use build_log_io::adapters::v16::{self, layout};
use build_log_io::adapters::MarkerPlace;
use build_log_io::dispatch::{open, production, resolve_for_create, VersionKey};
use build_log_model::{
    BuildContext, BuildMeta, Capabilities, Direction, Error, ErrorClass, ErrorKind, Feature,
    LayerId, Origin, ParameterValue, UnitSource,
};
use build_log_testkit::{miniature, miniature_path};
use common::v16::{layer, made, monitoring};
use common::{put_array, put_scalar, put_text, temp_h5};
use hdf5::Group;

fn expectation() -> Expectation {
    Expectation::load(&expectations_dir().join("v16.json")).expect("the V16 expectation file")
}

/// What the V16 miniature says, through the registry.
fn build_of_the_miniature() -> BuildMeta {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    (*opened.source().build().expect("the metadata")).clone()
}

fn kind(error: &Error) -> String {
    format!("{:?}", error.kind())
}

// ---------------------------------------------------------------- registration

#[test]
fn the_v16_adapter_is_registered_under_16() {
    let set = common::production_set("16");
    assert_eq!(set.key().as_str(), "16");
    assert_eq!(set.name(), "v16");
    assert_eq!(
        set.marker(),
        &MarkerPlace::new(layout::MOUNT, layout::MARKER)
    );
    assert!(set.reader().is_provided());
    // the registration unit is the one the folder hands out
    assert_eq!(v16::adapter_set().key(), set.key());
    assert_eq!(v16::KEY, "16");
    assert_eq!(v16::NAME, "v16");
}

#[test]
fn a_v16_file_opens_as_schema_16_with_the_v16_adapter() {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    assert_eq!(opened.key().as_str(), "16");
    assert_eq!(opened.adapter(), "v16");
    // the file keeps its version: nothing upgrades it
    let build = opened.source().build().expect("the metadata");
    assert_eq!(build.layers_declared, Some(2));
    assert_eq!(build.layers_present(), 2);
}

#[test]
fn v16_declares_samples_and_correction_grids_and_nothing_else() {
    // what the reader returns is what is declared (decisions D12-9, D13-7 and D24-11): the metadata, the samples of the lasers and their correction grids, and no outcomes, events, images, event
    // columns or image regions, ever
    let declared = Capabilities::new()
        .with(Direction::Read, Feature::Samples)
        .with(Direction::Read, Feature::CorrectionGrids);
    let set = common::production_set("16");
    assert_eq!(set.capabilities(), &declared);
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    assert_eq!(*opened.capabilities(), declared);
    // reading is declared and writing is not: V16 is never written
    assert!(!set
        .capabilities()
        .supports(Direction::Write, Feature::Samples));
}

#[test]
fn the_writer_and_the_patcher_are_declared_unsupported_for_good() {
    let set = common::production_set("16");
    let writer = set.writer().unsupported_reason().expect("a reason");
    assert!(writer.contains("read-only"), "{writer}");
    let patcher = set.patcher().unsupported_reason().expect("a reason");
    assert!(patcher.contains("read-only"), "{patcher}");
    // and neither is "arrives later": V16 is never written
    assert!(!writer.contains("IP-08") && !patcher.contains("IP-11"));
}

#[test]
fn create_16_is_unsupported_in_version() {
    let key = VersionKey::new("16").expect("a key");
    let error = resolve_for_create(production(), &key).expect_err("V16 has no writer");
    match error.kind() {
        ErrorKind::UnsupportedInVersion { version, feature } => {
            assert_eq!(version, "16");
            assert!(
                feature.contains("creating a file") && feature.contains("read-only"),
                "{feature}"
            );
        }
        other => panic!("expected UnsupportedInVersion, found {other:?}"),
    }
    assert_eq!(error.class(), ErrorClass::Caller);
}

// ---------------------------------------------------------------- the miniature conforms

#[test]
fn the_v16_build_and_layers_equal_the_expectation() {
    let build = build_of_the_miniature();
    let found = check_parts(&build, &expectation(), &[Part::Build, Part::Layers]);
    assert!(
        found.is_empty(),
        "the build and the layers do not conform:\n  {}",
        found.join("\n  ")
    );
    // declared and present are counted separately
    assert_eq!(build.layers_declared, Some(2));
    assert_eq!(build.layers_present(), 2);
    // V16 has no build layer thickness, no status and no join: nothing is invented
    assert!(build.layer_thickness.is_none());
    assert_eq!(build.quality.outcome, build_log_model::Outcome::NotRecorded);
    assert!(build.provenance.opcua_join_status.is_none());
}

#[test]
fn the_v16_context_equals_the_expectation() {
    let build = build_of_the_miniature();
    let found = check_parts(&build, &expectation(), &[Part::Context]);
    assert!(
        found.is_empty(),
        "the context does not conform:\n  {}",
        found.join("\n  ")
    );
    // the four texts are recorded, and the three placeholders are not, each with its raw spelling
    assert_eq!(build.context.values().len(), 4);
    for name in ["plate.thickness", "plate.weight", "starting_height"] {
        assert_eq!(
            origin(&build, name),
            raw("1.0"),
            "{name} is V16's placeholder"
        );
    }
}

#[test]
fn the_date_is_the_string_as_written() {
    // the offset label of the miniature's date looks wrong (CT-UC-12), and nothing is corrected
    let build = build_of_the_miniature();
    assert_eq!(
        build.identity.date.as_deref(),
        Some("2026-01-26 19:37:22 UTC-05:00")
    );
    // and a text is text: another spelling, a trailing space and an empty text stay as they are
    for date in ["26/01/2026", "2026-01-26 19:37:22 UTC+99:00 ", ""] {
        let made = made("v16-date", |mount| put_text(mount, layout::DATE, date)).expect("it reads");
        assert_eq!(made.identity.date.as_deref(), Some(date), "{date:?}");
    }
    let without = made("v16-no-date", |_| {}).expect("it reads");
    assert_eq!(without.identity.date, None, "no date is not a date");
}

#[test]
fn a_v16_layer_time_has_no_unit() {
    // the integers of the file, in no unit that V16 states (decision D12-3): no unit and no derived instant
    let build = build_of_the_miniature();
    for (id, start, end) in [
        (1u32, 1_769_456_448_906_486i64, 1_769_456_473_287_880i64),
        (2, 1_769_456_486_191_607, 1_769_456_510_278_576),
    ] {
        let layer = build.layers.get(LayerId(id)).expect("the layer");
        for (time, expected) in [(&layer.start, start), (&layer.end, end)] {
            let time = time.as_ref().expect("the layer has its times");
            assert_eq!(time.value, expected);
            assert_eq!(time.unit_source, UnitSource::Unknown);
            assert!(!time.unit.is_known(), "V16 states no unit for a time");
        }
    }
}

// ---------------------------------------------------------------- nothing is dropped

/// Every group and dataset below (and including) `path`, and every attribute of each, found by looking at the file and nothing else.
fn walk(
    group: &Group,
    path: &str,
    nodes: &mut BTreeSet<String>,
    attributes: &mut BTreeSet<(String, String)>,
) {
    nodes.insert(path.to_string());
    for name in group.attr_names().expect("the attributes of a group") {
        attributes.insert((path.to_string(), name));
    }
    for name in group.member_names().expect("the members of a group") {
        let child = format!("{path}/{name}");
        if let Ok(inner) = group.group(&name) {
            walk(&inner, &child, nodes, attributes);
        } else {
            let dataset = group
                .dataset(&name)
                .expect("a member that is not a group is a dataset");
            nodes.insert(child.clone());
            for attribute in dataset.attr_names().expect("the attributes of a dataset") {
                attributes.insert((child.clone(), attribute));
            }
        }
    }
}

#[test]
fn nothing_in_the_v16_file_is_dropped() {
    let build = build_of_the_miniature();
    let file = hdf5::File::open(miniature_path(miniature::V16)).expect("the file");
    // what is in the file: the mount, and nothing else at the root
    let at_the_root: Vec<String> = file.member_names().expect("the members of the root");
    assert_eq!(
        at_the_root,
        [layout::MOUNT],
        "the file holds only the mount"
    );
    let mount_path = format!("/{}", layout::MOUNT);
    let (mut nodes, mut attributes) = (BTreeSet::new(), BTreeSet::new());
    walk(
        &file.group(&mount_path).expect("the mount"),
        &mount_path,
        &mut nodes,
        &mut attributes,
    );
    // a floor that shows the walk looked at the file and not at nothing: the miniature has 39 nodes (the mount, the layers' group, two layers, the lasers and their channels, and the calibration)
    assert!(
        nodes.len() >= 39,
        "the walk found only {} nodes: it looked at too little",
        nodes.len()
    );

    // what the adapter typed: the mount, the layers' group, the layers, and the attributes that the build level and the layers read
    let monitoring_path = format!("{mount_path}/{}", layout::MONITORING_DATA);
    let mut typed_nodes: BTreeSet<String> = [mount_path.clone(), monitoring_path.clone()].into();
    let mut typed_attributes: BTreeSet<(String, String)> = layout::TYPED_ON_THE_MOUNT
        .iter()
        .copied()
        .chain(layout::context_attributes())
        .map(|a| (mount_path.clone(), a.to_string()))
        .collect();
    // the build's sample period is typed here: every laser of this file has it in its time axis (`v16_lasers.rs`)
    typed_attributes.insert((monitoring_path.clone(), layout::SAMPLE_PERIOD.to_string()));
    // what the adapter kept, by the path it gives and the base that the path is relative to
    let mut kept_nodes: BTreeSet<String> = BTreeSet::new();
    let mut kept_attributes: BTreeSet<(String, String)> = BTreeSet::new();
    let mut keep = |extra: &build_log_model::Extra, base: &str| {
        for (name, _) in extra.attributes() {
            kept_attributes.insert((base.to_string(), name.to_string()));
        }
        for node in extra.nodes() {
            let at = format!("{base}/{}", node.path.join("/"));
            for (name, _) in node.attributes() {
                kept_attributes.insert((at.clone(), name.to_string()));
            }
            kept_nodes.insert(at);
        }
    };
    keep(&build.extra, &mount_path);
    for id in build.layers.ids() {
        let layer_path = format!("{monitoring_path}/{}{}", layout::LAYER_PREFIX, id.0);
        typed_nodes.insert(layer_path.clone());
        for name in layout::TYPED_ON_A_LAYER {
            typed_attributes.insert((layer_path.clone(), name.to_string()));
        }
        keep(&build.layers.get(id).unwrap().extra, &layer_path);
    }
    // the lasers (phase 12D): each laser's group in a layer, its two groups of channels where the file has them, and the dataset of each channel that is recorded in the layer
    for laser in build.lasers() {
        for id in laser.layers.ids() {
            let laser_path = format!(
                "{monitoring_path}/{}{}/{}",
                layout::LAYER_PREFIX,
                id.0,
                laser.key.key
            );
            typed_nodes.insert(laser_path.clone());
            for holder in [layout::LASER_DATA, layout::POSITION] {
                let held = format!("{laser_path}/{holder}");
                if nodes.contains(&held) {
                    typed_nodes.insert(held);
                }
            }
            for channel in &laser.channels {
                if channel.presence.get(id).is_some() {
                    let holder = match channel.kind {
                        build_log_model::ChannelKind::LaserPower
                        | build_log_model::ChannelKind::LaserState => layout::LASER_DATA,
                        _ => layout::POSITION,
                    };
                    let name = channel.key.name().expect("a channel has a name");
                    typed_nodes.insert(format!("{laser_path}/{holder}/{name}"));
                }
            }
            keep(&laser.layers.get(id).unwrap().extra, &laser_path);
        }
    }
    // the calibration (phase 12E): the group, the calibration of each laser that became a snapshot with its thirteen named attributes, and the mount's two bit resolutions, which the snapshots took;
    // what a snapshot keeps (the correction grid and the twelve attributes that are not named) is in its own `extra`, below the calibration
    let calibration_path = format!("{mount_path}/{}", layout::CALIBRATION_DATA);
    typed_nodes.insert(calibration_path.clone());
    for attribute in [
        layout::ACTUAL_BIT_RESOLUTION,
        layout::COMMANDED_BIT_RESOLUTION,
    ] {
        typed_attributes.insert((mount_path.clone(), attribute.to_string()));
    }
    for laser in build.lasers() {
        let Some(parameters) = &laser.parameters else {
            continue;
        };
        let at = format!(
            "{calibration_path}/{}{}",
            layout::CALIBRATION_PREFIX,
            laser.key.number
        );
        typed_nodes.insert(at.clone());
        for attribute in [
            layout::LASER_MODEL,
            layout::SCAN_HEAD_MODEL,
            layout::SCANNER_FIELD_SIZE_X,
            layout::SCANNER_FIELD_SIZE_Y,
            layout::SCANNER_FIELD_SIZE_Z,
            layout::SCANNER_OFFSET_X,
            layout::SCANNER_OFFSET_Y,
            layout::SCANNER_OFFSET_Z,
            layout::SCALING_X,
            layout::SCALING_Y,
            layout::MAJOR_AXIS_ANGLE,
            layout::RAYLEIGH_LENGTH_MAJOR_AXIS,
            layout::RAYLEIGH_LENGTH_MINOR_AXIS,
        ] {
            typed_attributes.insert((at.clone(), attribute.to_string()));
        }
        // the correction grid is typed: a dataset of the calibration that the snapshot lists (IP-24 phase 24B)
        for grid in &parameters.grids {
            typed_nodes.insert(format!("{at}/{}", grid.name));
        }
        keep(&parameters.extra, &at);
    }

    // every node and every attribute of the file is typed or kept, and nothing is kept that the file does not have
    let covered_nodes: BTreeSet<String> = typed_nodes.union(&kept_nodes).cloned().collect();
    let missing: Vec<&String> = nodes.difference(&covered_nodes).collect();
    assert!(missing.is_empty(), "nodes that are dropped: {missing:?}");
    let covered_attributes: BTreeSet<(String, String)> =
        typed_attributes.union(&kept_attributes).cloned().collect();
    let missing: Vec<&(String, String)> = attributes.difference(&covered_attributes).collect();
    assert!(
        missing.is_empty(),
        "attributes that are dropped: {missing:?}"
    );
    assert!(
        kept_nodes.is_subset(&nodes),
        "nodes that the file does not have: {:?}",
        kept_nodes.difference(&nodes).collect::<Vec<_>>()
    );
    assert!(
        kept_attributes.is_subset(&attributes),
        "attributes that the file does not have: {:?}",
        kept_attributes.difference(&attributes).collect::<Vec<_>>()
    );
    // and what the adapter does not type is kept: the two formula images, the correction grid of each laser, and the twelve attributes of each calibration that the CDM does not name
    assert!(!kept_nodes.is_empty() && !kept_attributes.is_empty());
    // the marker is the version itself: the dispatcher reads it and the build does not keep it as data
    assert!(
        build.extra.attribute(layout::MARKER).is_none(),
        "the marker is kept as data"
    );
    // **what the adapter types is not kept as well, held without the layout's own lists** (the walk above builds "typed" from them): the miniature's layers hold only the four attributes
    // that a layer types, so a layer keeps none, and the mount keeps none either (the calibration took the two bit resolutions, phase 12E)
    for id in build.layers.ids() {
        let layer = build.layers.get(id).unwrap();
        assert_eq!(
            layer.extra.attributes().count(),
            0,
            "a layer keeps an attribute that it types"
        );
    }
    let on_the_mount: Vec<&str> = build.extra.attributes().map(|(name, _)| name).collect();
    assert!(
        on_the_mount.is_empty(),
        "the mount keeps an attribute that the build level or the calibration types: {on_the_mount:?}"
    );
    // what is typed is not kept twice: a node that the adapter types (a laser, its channels) is not also kept, and the miniature has no attribute on a typed node that would make a description of it
    assert!(
        typed_nodes.is_disjoint(&kept_nodes),
        "nodes that are typed and kept: {:?}",
        typed_nodes.intersection(&kept_nodes).collect::<Vec<_>>()
    );
    assert!(
        typed_attributes.is_disjoint(&kept_attributes),
        "attributes that are typed and kept: {:?}",
        typed_attributes
            .intersection(&kept_attributes)
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------- V16's placeholder is 1.0

fn value(build: &BuildMeta, name: &str) -> Option<ParameterValue> {
    build
        .context
        .values()
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

fn number(build: &BuildMeta, name: &str) -> Option<f64> {
    match value(build, name)? {
        ParameterValue::Quantity(q) => Some(q.value),
        other => panic!("`{name}` is {other:?}, not a quantity"),
    }
}

fn origin(build: &BuildMeta, name: &str) -> Option<Origin> {
    build.origins.get(&BuildContext::origin_name(name)).cloned()
}

fn raw(text: &str) -> Option<Origin> {
    Some(Origin::PlaceholderDropped { raw: text.into() })
}

#[test]
fn a_context_value_of_one_is_not_recorded_and_zero_and_other_numbers_are() {
    // V16 writes 1.0 for a starting height, a plate's thickness and its weight when it does not know them; V17 writes 0.0, which is a value here
    let build = made("v16-context-numbers", |mount| {
        put_scalar(mount, layout::STARTING_HEIGHT, 0.0f32);
        put_scalar(mount, layout::BUILD_PLATE_THICKNESS, 1.0f32);
        put_scalar(mount, layout::BUILD_PLATE_WEIGHT, 2.5f32);
    })
    .expect("it reads");
    assert_eq!(number(&build, "starting_height"), Some(0.0));
    assert_eq!(origin(&build, "starting_height"), None);
    assert!(build.context.plate.thickness.is_none());
    assert_eq!(origin(&build, "plate.thickness"), raw("1.0"));
    assert_eq!(number(&build, "plate.weight"), Some(2.5));
    assert_eq!(origin(&build, "plate.weight"), None);
    // an integer one is the same placeholder with the same raw spelling, and a value that the file does not have says nothing
    let integer = made("v16-context-integer", |mount| {
        put_array(mount, layout::STARTING_HEIGHT, &[1i32]);
    })
    .expect("it reads");
    assert!(integer.context.starting_height.is_none());
    assert_eq!(origin(&integer, "starting_height"), raw("1.0"));
    assert_eq!(origin(&integer, "plate.weight"), None);
    // V16 has neither vocabulary of the plate's shape: the build says nothing about a vocabulary
    assert_eq!(origin(&build, "plate"), None);
}

#[test]
fn a_thickness_of_one_is_not_recorded_and_keeps_its_raw_value_and_other_numbers_are() {
    let build = made("v16-layer-thickness", |mount| {
        let monitoring = monitoring(mount);
        // both are the placeholder of V16
        let first = layer(&monitoring, 1);
        put_scalar(&first, layout::ACTUAL_LAYER_THICKNESS, 1.0f32);
        put_scalar(&first, layout::COMMANDED_LAYER_THICKNESS, 1.0f32);
        // one is real and the other is the placeholder: each is reported on its own, and 0.0 (V17's placeholder) is a value here
        let second = layer(&monitoring, 2);
        put_scalar(&second, layout::ACTUAL_LAYER_THICKNESS, 0.03f32);
        put_scalar(&second, layout::COMMANDED_LAYER_THICKNESS, 0.0f32);
        // only the commanded attribute is there
        let third = layer(&monitoring, 3);
        put_array(&third, layout::COMMANDED_LAYER_THICKNESS, &[0.03f32]);
        // no thickness attribute at all
        layer(&monitoring, 4);
    })
    .expect("the layers are read");
    let dropped = Origin::PlaceholderDropped { raw: "1.0".into() };
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
        "V16 states no unit for a thickness"
    );
    assert_eq!(second.origins.get("actual_thickness"), None);
    assert_eq!(
        second.commanded_thickness.as_ref().map(|t| t.value),
        Some(0.0),
        "0.0 is a thickness in V16"
    );
    assert_eq!(second.origins.get("commanded_thickness"), None);

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

    let fourth = of(4);
    assert!(
        fourth.actual_thickness.is_none()
            && fourth.commanded_thickness.is_none()
            && fourth.origins.is_empty()
    );
}

// ---------------------------------------------------------------- the layers

#[test]
fn layer_ids_come_from_the_group_names_and_may_have_gaps() {
    let build = made("v16-layer-ids", |mount| {
        put_array(mount, layout::LAYER_QUANTITY, &[99i32]);
        let monitoring = monitoring(mount);
        // created out of order, with gaps, and with names that are not a layer's
        for id in [12u32, 3, 7] {
            layer(&monitoring, id);
        }
        monitoring.create_group("Notes").expect("another group");
        monitoring
            .create_group("layer_5")
            .expect("a name in another case");
        monitoring
            .create_group(&format!("{}x", layout::LAYER_PREFIX))
            .expect("a prefix and no digits");
    })
    .expect("the layers are read");
    assert_eq!(
        build.layers.ids().collect::<Vec<_>>(),
        [LayerId(3), LayerId(7), LayerId(12)]
    );
    // declared and present are counted separately; neither is derived from the other
    assert_eq!(build.layers_declared, Some(99));
    assert_eq!(build.layers_present(), 3);
    // the groups that are not named for a layer are kept in the build's extra, at their place
    for name in ["Notes", "layer_5", "Layer_x"] {
        assert!(
            build
                .extra
                .node(&[layout::MONITORING_DATA.to_string(), name.to_string()])
                .is_some(),
            "{name} is kept"
        );
    }
}

#[test]
fn a_mount_without_its_layers_group_is_corrupt() {
    let on_disk = temp_h5("v16-no-layers", |mount| {
        put_scalar(mount, layout::MARKER, 16i32);
        put_text(mount, layout::BUILD_ID, "a build");
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let error = v16::read::read_build(&mount).expect_err("a build with no layers' group");
    assert!(
        matches!(error.kind(), ErrorKind::Corrupt { detail } if detail.contains(layout::MONITORING_DATA)),
        "{}",
        kind(&error)
    );
}

#[test]
fn two_groups_for_one_layer_and_an_id_that_does_not_fit_are_reported_on_their_layer() {
    // `Layer_1` and `Layer_01` are the same layer
    let error = made("v16-layer-twice", |mount| {
        let monitoring = monitoring(mount);
        layer(&monitoring, 1);
        monitoring
            .create_group(&format!("{}01", layout::LAYER_PREFIX))
            .expect("a second name");
    })
    .expect_err("two groups for one layer");
    assert!(
        matches!(error.kind(), ErrorKind::Corrupt { .. }),
        "{}",
        kind(&error)
    );
    assert_eq!(error.location().layer, Some(LayerId(1)));
    // digits that are not a 32-bit layer id are `OutOfWidth`, never a wrapped number
    let error = made("v16-layer-width", |mount| {
        monitoring(mount)
            .create_group(&format!("{}99999999999", layout::LAYER_PREFIX))
            .expect("a very large id");
    })
    .expect_err("an id that does not fit");
    assert!(
        matches!(error.kind(), ErrorKind::OutOfWidth { .. }),
        "{}",
        kind(&error)
    );
}
