//! The gate of the V17 adapter (IP-13 phase 13D, task T13.6; decisions D13-3, D13-9 and D13-19).
//!
//! * **The gate file** (`v17_triggered_2026_10_19.h5`, tauri's current output with the OPC UA join) conforms to its whole expectation file.
//! * **The other V17 files are reported, not asserted** (D13-9): the report says what still disagrees, and a file joins `GATED` in the phase that gates it.
//! * **Nothing in the gate file is dropped** (D13-3): an independent walk of the file, every group, dataset and attribute of the mount and of the two groups
//!   beside it, finds each one either typed or kept in `extra`, and never both.
//! * **A V17 payload nested in a larger file reads the same as a standalone one** (D13-19), the groups beside the mount included.
//! * A file that says 17 and has no layers' group is `Corrupt`.

mod common;

use std::collections::BTreeSet;
use std::fs;

use build_log_conformance::{check, expectations_dir, Expectation};
use build_log_io::adapters::v17::layout;
use build_log_io::dispatch::{open, open_at, production};
use build_log_model::{BuildMeta, ChannelKind, ErrorKind, LayerId};
use build_log_testkit::{miniature, miniature_path};
use common::{
    dataset, laser, layer, monitoring, put_array, put_scalar, put_text, scanner_card, temp_file,
};
use hdf5::Group;

/// The files whose whole conformance is asserted. A file joins this list in the phase of the plan that gates it (13E, 13F, 13G).
const GATED: [&str; 1] = [miniature::V17_TRIGGERED_2026_10_19];

/// Every V17 miniature, with its expectation file.
const V17_FILES: [(&str, &str); 6] = [
    (
        miniature::V17_TRIGGERED_2026_10_19,
        "v17_triggered_2026_10_19.json",
    ),
    (miniature::V17_MOCK, "v17_mock.json"),
    (miniature::V17_OCT15, "v17_oct15.json"),
    (
        miniature::V17_2026_08_18_IMAGES,
        "v17_2026_08_18_images.json",
    ),
    (miniature::V17_MARCH, "v17_march.json"),
    (miniature::V17_SPECIAL_LAYERS, "special_layers_v17.json"),
];

fn build_of(file: &str) -> BuildMeta {
    let opened =
        open(production(), miniature_path(file)).expect("the file opens through the registry");
    (*opened.source().build().expect("the metadata")).clone()
}

fn disagreements(file: &str, json: &str) -> Vec<String> {
    let expectation =
        Expectation::load(&expectations_dir().join(json)).expect("an expectation file");
    check(&build_of(file), &expectation)
}

// ---------------------------------------------------------------- the gate

#[test]
fn the_gate_file_conforms() {
    let found = disagreements(
        miniature::V17_TRIGGERED_2026_10_19,
        "v17_triggered_2026_10_19.json",
    );
    assert!(
        found.is_empty(),
        "the gate file does not conform:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_other_v17_files_are_reported_and_not_asserted() {
    let mut report = String::from(
        "V17 conformance report: the whole check of each V17 miniature (decision D13-9)\n\n",
    );
    let mut others = 0;
    for (file, json) in V17_FILES {
        let found = disagreements(file, json);
        let gated = GATED.contains(&file);
        if gated {
            // a gated file must conform: the report is not the place where a regression is allowed to hide
            assert!(
                found.is_empty(),
                "{file} is gated and does not conform:\n  {}",
                found.join("\n  ")
            );
        } else {
            others += 1;
        }
        report.push_str(&format!(
            "{file}: {}, {} disagreement(s)\n",
            if gated { "gated" } else { "reported only" },
            found.len()
        ));
        for line in &found {
            report.push_str(&format!("    {line}\n"));
        }
    }
    // the report is a file as well as text, so that it can be read without asking the test to print
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("v17-conformance-report.txt");
    fs::write(&path, &report).expect("the report can be written");
    println!("{report}\n(written to {})", path.display());
    assert_eq!(
        others,
        V17_FILES.len() - GATED.len(),
        "every file that is not gated is in the report"
    );
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
        let child = format!("{}/{name}", path.trim_end_matches('/'));
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
fn nothing_in_the_gate_file_is_dropped() {
    let path = miniature_path(miniature::V17_TRIGGERED_2026_10_19);
    let build = build_of(miniature::V17_TRIGGERED_2026_10_19);
    let file = hdf5::File::open(&path).expect("the file");
    let root = file.group("/").expect("the root");

    // what is in the file: the mount and the two groups beside it, and nothing else at the root
    let mount_path = format!("/{}", layout::MOUNT);
    let besides: Vec<String> = layout::BESIDE_THE_MOUNT
        .iter()
        .map(|n| format!("/{n}"))
        .collect();
    let at_the_root: BTreeSet<String> = root
        .member_names()
        .unwrap()
        .into_iter()
        .map(|n| format!("/{n}"))
        .collect();
    let ours: BTreeSet<String> = std::iter::once(mount_path.clone())
        .chain(besides.iter().cloned())
        .collect();
    assert!(
        at_the_root.is_subset(&ours),
        "something at the root of the file is not V17's: {at_the_root:?}"
    );
    let (mut nodes, mut attributes) = (BTreeSet::new(), BTreeSet::new());
    for top in &ours {
        if at_the_root.contains(top) {
            walk(&file.group(top).unwrap(), top, &mut nodes, &mut attributes);
        }
    }
    assert!(
        nodes.len() > 100,
        "the walk found only {} nodes: it looked at too little",
        nodes.len()
    );

    // what the adapter typed, from the structure of the build and the names of the layout
    let monitoring_path = format!("{mount_path}/{}", layout::MONITORING_DATA);
    let mut typed_nodes: BTreeSet<String> = [mount_path.clone(), monitoring_path.clone()].into();
    let mut typed_attributes: BTreeSet<(String, String)> = layout::TYPED_ON_THE_MOUNT
        .iter()
        .map(|a| (mount_path.clone(), a.to_string()))
        .collect();
    let mut kept_nodes: BTreeSet<String> = BTreeSet::new();
    let mut kept_attributes: BTreeSet<(String, String)> = BTreeSet::new();
    let absolute = |base: &str, relative: &[String]| -> String {
        match relative.first().map(String::as_str) {
            Some("..") => format!("/{}", relative[1..].join("/")),
            _ => format!("{base}/{}", relative.join("/")),
        }
    };
    let mut keep = |extra: &build_log_model::Extra, base: &str| {
        for (name, _) in extra.attributes() {
            kept_attributes.insert((base.to_string(), name.to_string()));
        }
        for node in extra.nodes() {
            let at = absolute(base, &node.path);
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
    for laser in build.lasers() {
        for id in laser.layers.ids() {
            let laser_path = format!(
                "{monitoring_path}/{}{}/{}",
                layout::LAYER_PREFIX,
                id.0,
                laser.key.key
            );
            typed_nodes.insert(laser_path.clone());
            for name in layout::TYPED_ON_A_LASER {
                typed_attributes.insert((laser_path.clone(), name.to_string()));
            }
            keep(&laser.layers.get(id).unwrap().extra, &laser_path);
            // the groups of channels are typed where the file has them, and so is each channel's dataset where the channel is recorded in the layer
            for holder in [layout::LASER_DATA, layout::POSITION] {
                let held = format!("{laser_path}/{holder}");
                if nodes.contains(&held) {
                    typed_nodes.insert(held);
                }
            }
            for channel in &laser.channels {
                if channel.presence.get(id).is_some() {
                    let (holder, name) = match &channel.kind {
                        ChannelKind::LaserPower => (layout::LASER_DATA, layout::LASER_POWER),
                        ChannelKind::LaserState => (layout::LASER_DATA, layout::LASER_STATE),
                        other => (layout::POSITION, position_name(other)),
                    };
                    typed_nodes.insert(format!("{laser_path}/{holder}/{name}"));
                }
            }
        }
    }

    // every node of the file is typed or kept, and every kept node is in the file
    let accounted: BTreeSet<String> = typed_nodes.union(&kept_nodes).cloned().collect();
    let missing: Vec<&String> = nodes.difference(&accounted).collect();
    let invented: Vec<&String> = accounted.difference(&nodes).collect();
    assert!(
        missing.is_empty(),
        "nodes of the file that are neither typed nor kept: {missing:?}"
    );
    assert!(
        invented.is_empty(),
        "nodes the adapter holds that the file does not: {invented:?}"
    );
    // and so is every attribute, and none is both: an attribute that is typed is not also kept
    let typed_present: BTreeSet<(String, String)> = typed_attributes
        .intersection(&attributes)
        .cloned()
        .collect();
    assert!(
        typed_present.is_disjoint(&kept_attributes),
        "kept twice: {:?}",
        typed_present
            .intersection(&kept_attributes)
            .collect::<Vec<_>>()
    );
    let accounted: BTreeSet<(String, String)> =
        typed_present.union(&kept_attributes).cloned().collect();
    let missing: Vec<&(String, String)> = attributes.difference(&accounted).collect();
    let invented: Vec<&(String, String)> = accounted.difference(&attributes).collect();
    assert!(
        missing.is_empty(),
        "attributes of the file that are neither typed nor kept: {missing:?}"
    );
    assert!(
        invented.is_empty(),
        "attributes the adapter holds that the file does not: {invented:?}"
    );
    // the groups beside the mount are among what is kept, by a path that starts above the mount
    for beside in &besides {
        if at_the_root.contains(beside) {
            assert!(kept_nodes.contains(beside), "{beside} is not kept");
        }
    }
}

fn position_name(kind: &ChannelKind) -> &'static str {
    use build_log_model::Axis::{X, Y, Z};
    match kind {
        ChannelKind::ActualPosition { axis: X } => layout::ACTUAL_X,
        ChannelKind::ActualPosition { axis: Y } => layout::ACTUAL_Y,
        ChannelKind::ActualPosition { axis: Z } => layout::ACTUAL_Z,
        ChannelKind::CommandedPosition { axis: X } => layout::COMMANDED_X,
        ChannelKind::CommandedPosition { axis: Y } => layout::COMMANDED_Y,
        ChannelKind::CommandedPosition { axis: Z } => layout::COMMANDED_Z,
        other => panic!("not a position: {other:?}"),
    }
}

// ---------------------------------------------------------------- nested and standalone, and a file that is not what it says

/// A small V17 payload: the mount, with a layer, two lasers and a scanner card, and the two groups beside it. Written into `parent`, the root of a file or a
/// group of a larger one.
fn write_payload(parent: &Group) {
    let mount = parent.create_group(layout::MOUNT).expect("the mount");
    put_array(&mount, layout::MARKER, &[17i32]);
    put_text(&mount, layout::BUILD_ID, "a build");
    put_text(&mount, layout::DATE, "2026-10-19 20:16:17 UTC+00:00");
    put_array(&mount, layout::LAYER_QUANTITY, &[1i32]);
    put_text(&mount, "A_Setting", "kept");
    let layers = monitoring(&mount);
    let first = layer(&layers, 1);
    put_scalar(&first, layout::START_LAYER_TIME, 1_000i64);
    laser(&first, 1, 6, Some((1_000_000, 1_500_000)));
    laser(&first, 2, 0, None);
    scanner_card(&mount, 1, 10.0);
    let besides = parent
        .create_group(layout::GLOBAL_SENSORS)
        .expect("beside the mount");
    let series = besides.create_group("unassigned").unwrap();
    dataset::<f32>(&series, "Value", 3);
    parent
        .create_group(layout::CONFIGURATION)
        .expect("beside the mount");
}

#[test]
fn a_nested_v17_payload_reads_the_same_as_a_standalone_file() {
    let standalone = temp_file("v17-standalone", |file| write_payload(file));
    let nested = temp_file("v17-nested", |file| {
        // the payload sits in a group of a larger file, next to data that is not V17's
        let container = file.create_group("container").unwrap();
        file.create_group("Something_Else").unwrap();
        write_payload(&container);
    });
    let alone = open(production(), &standalone.0).expect("the standalone file opens");
    let file = hdf5::File::open(&nested.0).unwrap();
    let mount = file
        .group("container")
        .unwrap()
        .group(layout::MOUNT)
        .unwrap();
    let inside = open_at(production(), &mount).expect("the nested mount opens");
    let (alone, inside) = (
        alone.source().build().unwrap(),
        inside.source().build().unwrap(),
    );
    assert_eq!(*alone, *inside);
    // it is a real build, and what sits beside the mount is part of it, by a path that starts above the mount
    assert_eq!(alone.layers_present(), 1);
    assert_eq!(alone.lasers().len(), 2);
    assert_eq!(
        alone
            .laser(&build_log_model::LaserKey::new("Laser_1", 1))
            .unwrap()
            .layers
            .get(LayerId(1))
            .unwrap()
            .presence
            .samples(),
        Some(6)
    );
    let kept: Vec<String> = alone.extra.nodes().map(|n| n.path.join("/")).collect();
    assert!(
        kept.iter()
            .any(|p| p == "../Global_Sensors/unassigned/Value"),
        "{kept:?}"
    );
    assert!(kept.iter().any(|p| p == "../Configuration"), "{kept:?}");
    // a group beside the mount that V17 does not define is not V17's and is not kept
    assert!(
        !kept.iter().any(|p| p.contains("Something_Else")),
        "{kept:?}"
    );
}

#[test]
fn a_file_that_says_17_but_has_no_monitoring_data_is_corrupt() {
    let file = temp_file("v17-no-monitoring", |file| {
        let mount = file.create_group(layout::MOUNT).unwrap();
        put_array(&mount, layout::MARKER, &[17i32]);
        put_text(&mount, layout::BUILD_ID, "a build");
    });
    let error = open(production(), &file.0)
        .map(|_| ())
        .expect_err("a build with no layers' group is not read as an empty one");
    match error.kind() {
        ErrorKind::Corrupt { detail } => {
            assert!(detail.contains(layout::MONITORING_DATA), "{detail}")
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}
