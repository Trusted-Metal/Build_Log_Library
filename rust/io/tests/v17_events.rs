//! The OPC UA events of a V17 build (IP-13 phase 13K, task T13.13; decisions D13-21, D13-26 and D13-29).
//!
//! * A series is a group that holds the source timestamps, named by the groups on the way down from its events group; its kind is the events group it is in.
//! * It is recorded in places: inside layers, and in the bucket that no layer owns, beside the mount. The same series in several places is one series.
//! * A place says its records (the length of its timestamps) and, for a task, its height. The records themselves are not read.
//! * The type a series declares and whether it has an interval are the same in every place: a place that differs is reported and kept whole.
//! * A column shorter than its timestamps is reported, not repaired. What else the groups hold is kept.
//!
//! The conformance of the two files that hold events (the gate file and the images file), and that nothing in them is dropped, are held by `v17_gate.rs` and
//! `v17_images.rs`. The small files here are made at test time.

mod common;

use build_log_io::adapters::v17::layout;
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildMeta, Direction, EventDataType, EventKind, EventSeries, Feature, LayerId, Origin,
};
use build_log_testkit::{miniature, miniature_path};
use common::{dataset, layer, monitoring, put_scalar, put_text, temp_file};
use hdf5::Group;

fn built(file: &str) -> BuildMeta {
    let opened = open(production(), miniature_path(file)).expect("the file opens");
    (*opened.source().build().expect("the metadata")).clone()
}

/// A file whose mount has a build id, and whose layers' group and root `write` fills (the root is what sits beside the mount), read as a V17 build.
fn synthetic(label: &str, write: impl FnOnce(&Group, &Group)) -> BuildMeta {
    let on_disk = temp_file(label, |file| {
        let mount = file.create_group(layout::MOUNT).expect("the mount");
        put_text(&mount, layout::BUILD_ID, "a build");
        let layers = monitoring(&mount);
        write(&layers, &file.group("/").expect("the root"));
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group(layout::MOUNT).expect("the mount");
    build_log_io::adapters::v17::read::read_build(&mount).expect("the build reads")
}

/// The group of a layer that holds its events, made if it is not there.
fn sensors_of(layer: &Group) -> Group {
    layer
        .group(layout::GLOBAL_SENSORS)
        .or_else(|_| layer.create_group(layout::GLOBAL_SENSORS))
        .expect("the events of a layer")
}

/// The bucket that no layer owns, beside the mount.
fn bucket(root: &Group) -> Group {
    root.create_group(layout::GLOBAL_SENSORS)
        .and_then(|events| events.create_group(layout::UNASSIGNED))
        .expect("the bucket")
}

/// A series: the groups down to `path` inside the events group `kind` of `place`, its timestamps, and the columns named, each with its length. Returns its group.
fn series(
    place: &Group,
    kind: &str,
    path: &[&str],
    records: usize,
    columns: &[(&str, usize)],
) -> Group {
    let mut group = place
        .group(kind)
        .or_else(|_| place.create_group(kind))
        .expect("an events group");
    for part in path {
        group = group
            .group(part)
            .or_else(|_| group.create_group(part))
            .expect("a group of the path");
    }
    dataset::<u64>(&group, layout::SOURCE_TS, records);
    for (name, length) in columns {
        dataset::<f64>(&group, name, *length);
    }
    group
}

fn find<'a>(build: &'a BuildMeta, kind: EventKind, path: &[&str]) -> Option<&'a EventSeries> {
    build
        .events
        .iter()
        .find(|s| s.kind == kind && s.path.iter().map(String::as_str).eq(path.iter().copied()))
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

// ---------------------------------------------------------------- the files that have events

#[test]
fn the_gate_files_events_are_all_in_the_bucket_that_no_layer_owns() {
    let build = built(miniature::V17_TRIGGERED_2026_10_19);
    // ten sensors and the state of the run, recorded once, before the build, between layers and after it
    assert_eq!(build.events.len(), 11);
    assert_eq!(
        build
            .events
            .iter()
            .filter(|s| s.kind == EventKind::Sensor)
            .count(),
        10
    );
    assert_eq!(
        build
            .events
            .iter()
            .filter(|s| s.kind == EventKind::Run)
            .count(),
        1
    );
    for series in &build.events {
        assert!(series.no_layer.is_some(), "{:?}", series.path);
        assert!(series.layers.is_empty(), "{:?}", series.path);
        assert!(!series.interval, "{:?}", series.path);
    }
    // a series is named by the path the server gave it, and the run state declares no type
    let run = build
        .events
        .iter()
        .find(|s| s.kind == EventKind::Run)
        .expect("the run state");
    assert_eq!(run.data_type, None);
    assert!(build
        .events
        .iter()
        .filter(|s| s.kind == EventKind::Sensor)
        .all(|s| s.data_type.is_some() && s.path.len() >= 2));
}

#[test]
fn a_sensor_event_is_one_series_recorded_in_the_bucket_and_in_each_layer() {
    let build = built(miniature::V17_2026_08_18_IMAGES);
    assert_eq!(build.events.len(), 40);
    let velocity = find(
        &build,
        EventKind::Sensor,
        &["Actuators", "optical_axis::drive::node", "current_velocity"],
    )
    .expect("a sensor that was recorded in the bucket and in each layer");
    assert!(velocity.no_layer.is_some());
    let layers: Vec<u32> = velocity.layers.ids().map(|id| id.0).collect();
    assert_eq!(layers, [1, 2, 3]);
    // one series, so one declared type, and the records are those of the places
    assert!(velocity.data_type.is_some());
    assert_eq!(
        velocity.records(),
        velocity.no_layer.as_ref().unwrap().records
            + velocity
                .layers
                .iter()
                .map(|(_, part)| part.records)
                .sum::<u64>()
    );
    // no two series have the same kind and path
    let mut keys: Vec<(&EventKind, &Vec<String>)> =
        build.events.iter().map(|s| (&s.kind, &s.path)).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), build.events.len());
}

#[test]
fn a_task_event_keeps_its_interval_and_its_height() {
    let build = built(miniature::V17_2026_08_18_IMAGES);
    let tasks: Vec<&EventSeries> = build
        .events
        .iter()
        .filter(|s| s.kind == EventKind::Task)
        .collect();
    assert_eq!(tasks.len(), 4);
    for task in tasks {
        // a task has a start and a stop for each record, and the file gives the height of the place where it was recorded
        assert!(task.interval, "{:?}", task.path);
        assert!(task.layers.iter().count() > 0, "{:?}", task.path);
        for (id, part) in task.layers.iter() {
            assert!(part.height.is_some(), "{:?} in layer {}", task.path, id.0);
        }
    }
    // a sensor has no interval and no height
    for sensor in build.events.iter().filter(|s| s.kind == EventKind::Sensor) {
        assert!(!sensor.interval);
        assert!(sensor.layers.iter().all(|(_, part)| part.height.is_none()));
    }
}

#[test]
fn a_file_with_no_events_has_none() {
    for file in [miniature::V17_MOCK, miniature::V17_MARCH] {
        assert!(built(file).events.is_empty(), "{file}");
    }
}

// ---------------------------------------------------------------- places and kinds

#[test]
fn a_series_in_a_layer_and_in_the_bucket_is_one_series_with_two_places() {
    let build = synthetic("events-places", |layers, root| {
        let first = sensors_of(&layer(layers, 1));
        let second = sensors_of(&layer(layers, 2));
        let named = ["Gas", "flow", "value"];
        series(
            &first,
            layout::SENSOR_EVENTS,
            &named,
            5,
            &[(layout::EVENT_VALUE, 5)],
        );
        series(
            &second,
            layout::SENSOR_EVENTS,
            &named,
            7,
            &[(layout::EVENT_VALUE, 7)],
        );
        series(
            &bucket(root),
            layout::SENSOR_EVENTS,
            &named,
            11,
            &[(layout::EVENT_VALUE, 11)],
        );
    });
    assert_eq!(build.events.len(), 1);
    let series = &build.events[0];
    assert_eq!(series.kind, EventKind::Sensor);
    assert_eq!(series.path, path(&["Gas", "flow", "value"]));
    assert_eq!(series.layers.get(LayerId(1)).unwrap().records, 5);
    assert_eq!(series.layers.get(LayerId(2)).unwrap().records, 7);
    // the bucket is a place of its own and is not a layer
    assert_eq!(series.no_layer.as_ref().unwrap().records, 11);
    assert_eq!(series.layers.len(), 2);
    assert_eq!(series.records(), 23);
}

#[test]
fn the_events_group_a_series_is_in_gives_its_kind() {
    let build = synthetic("events-kinds", |layers, _| {
        let place = sensors_of(&layer(layers, 1));
        series(&place, layout::SENSOR_EVENTS, &["a", "b"], 1, &[]);
        series(&place, layout::TASK_EVENTS, &["a", "b"], 2, &[]);
        series(&place, layout::RUN_EVENTS, &["a", "b"], 3, &[]);
    });
    // the same path under three kinds is three series, in the order of their kinds
    let kinds: Vec<EventKind> = build.events.iter().map(|s| s.kind.clone()).collect();
    assert_eq!(kinds, [EventKind::Sensor, EventKind::Task, EventKind::Run]);
    let records: Vec<u64> = build.events.iter().map(EventSeries::records).collect();
    assert_eq!(records, [1, 2, 3]);
}

#[test]
fn a_layer_without_events_has_no_series_and_a_build_without_the_group_has_none() {
    let build = synthetic("events-none", |layers, _| {
        layer(layers, 1);
        sensors_of(&layer(layers, 2));
    });
    assert!(build.events.is_empty());
}

// ---------------------------------------------------------------- the type and the interval

#[test]
fn a_series_declares_its_type_and_only_the_two_known_words_are_named() {
    let build = synthetic("events-types", |layers, _| {
        let place = sensors_of(&layer(layers, 1));
        for (name, declared) in [
            ("double", Some("Double")),
            ("boolean", Some("Boolean")),
            ("other", Some("Int32")),
            ("none", None),
        ] {
            let group = series(&place, layout::SENSOR_EVENTS, &[name], 1, &[]);
            if let Some(declared) = declared {
                put_text(&group, layout::EVENT_DATA_TYPE, declared);
            }
        }
    });
    let declared = |name: &str| {
        find(&build, EventKind::Sensor, &[name])
            .unwrap()
            .data_type
            .clone()
    };
    assert_eq!(declared("double"), Some(EventDataType::Double));
    assert_eq!(declared("boolean"), Some(EventDataType::Boolean));
    assert_eq!(
        declared("other"),
        Some(EventDataType::Other("Int32".into()))
    );
    assert_eq!(declared("none"), None);
}

#[test]
fn a_series_has_an_interval_only_when_it_holds_both_its_start_and_its_stop() {
    let build = synthetic("events-interval", |layers, _| {
        let place = sensors_of(&layer(layers, 1));
        series(
            &place,
            layout::TASK_EVENTS,
            &["both"],
            2,
            &[(layout::START_NS, 2), (layout::STOP_NS, 2)],
        );
        series(
            &place,
            layout::TASK_EVENTS,
            &["only_start"],
            2,
            &[(layout::START_NS, 2)],
        );
    });
    assert!(find(&build, EventKind::Task, &["both"]).unwrap().interval);
    let lone = find(&build, EventKind::Task, &["only_start"]).unwrap();
    assert!(!lone.interval);
    // and it says why, on the place, rather than pretend that it is a point
    let part = lone.layers.get(LayerId(1)).unwrap();
    assert!(matches!(
        part.origins.get("interval"),
        Some(Origin::Other(_))
    ));
}

#[test]
fn the_height_of_a_task_is_a_value_of_its_place_and_may_differ_between_places() {
    let build = synthetic("events-height", |layers, _| {
        for (id, height) in [(1, 0.5), (2, 0.75)] {
            let place = sensors_of(&layer(layers, id));
            let group = series(&place, layout::TASK_EVENTS, &["move"], 1, &[]);
            put_scalar(&group, layout::Z_HEIGHT, height);
        }
    });
    let task = find(&build, EventKind::Task, &["move"]).unwrap();
    let heights: Vec<Option<f64>> = task
        .layers
        .iter()
        .map(|(_, part)| part.height.as_ref().map(|h| h.value))
        .collect();
    assert_eq!(heights, [Some(0.5), Some(0.75)]);
    // the file states no unit for it, and none is invented
    assert!(task.layers.iter().all(|(_, part)| {
        let unit = &part.height.as_ref().unwrap().unit;
        unit.id.is_none() && unit.spelling.is_none()
    }));
}

#[test]
fn a_place_that_declares_another_type_is_reported_on_itself_and_kept_whole() {
    let build = synthetic("events-type-differs", |layers, _| {
        for (id, declared) in [(1, "Double"), (2, "Boolean")] {
            let place = sensors_of(&layer(layers, id));
            let group = series(&place, layout::SENSOR_EVENTS, &["Gas", "flow"], 3, &[]);
            put_text(&group, layout::EVENT_DATA_TYPE, declared);
        }
    });
    // the first place is described; the second is not, and says why
    let flow = find(&build, EventKind::Sensor, &["Gas", "flow"]).unwrap();
    assert_eq!(flow.data_type, Some(EventDataType::Double));
    let ids: Vec<u32> = flow.layers.ids().map(|id| id.0).collect();
    assert_eq!(ids, [1]);
    let second = build.layers.get(LayerId(2)).unwrap();
    let reason = second
        .origins
        .iter()
        .find(|(name, _)| name.starts_with("events "))
        .expect("the fault is on the layer");
    assert!(
        matches!(reason.1, Origin::NotRead { cause: build_log_model::NotReadCause::Fault, reason: why } if why.contains("another place"))
    );
    // and what it holds is kept, so that nothing is lost
    let mut kept = path(&[layout::GLOBAL_SENSORS, layout::SENSOR_EVENTS, "Gas", "flow"]);
    assert!(second.extra.node(&kept).is_some());
    kept.push(layout::SOURCE_TS.into());
    assert!(second.extra.node(&kept).is_some());
}

// ---------------------------------------------------------------- what is not as expected

#[test]
fn a_column_shorter_than_the_timestamps_is_reported_and_not_repaired() {
    let build = synthetic("events-short-values", |layers, _| {
        let place = sensors_of(&layer(layers, 1));
        series(
            &place,
            layout::SENSOR_EVENTS,
            &["Gas", "flow"],
            10,
            &[(layout::EVENT_VALUE, 7)],
        );
    });
    let part = find(&build, EventKind::Sensor, &["Gas", "flow"])
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap()
        .clone();
    // the records are those of the timestamps, and the shortfall is said
    assert_eq!(part.records, 10);
    match part.origins.get("values") {
        Some(Origin::Other(why)) => assert!(why.contains('7') && why.contains("10"), "{why}"),
        other => panic!("expected the reason, got {other:?}"),
    }
}

#[test]
fn timestamps_that_are_not_a_list_are_reported_and_the_series_is_kept_whole() {
    let build = synthetic("events-not-a-list", |layers, _| {
        let place = sensors_of(&layer(layers, 1));
        let group = place
            .create_group(layout::SENSOR_EVENTS)
            .unwrap()
            .create_group("bad")
            .unwrap();
        group
            .new_dataset::<u64>()
            .shape([2, 2])
            .create(layout::SOURCE_TS)
            .unwrap();
        series(&place, layout::SENSOR_EVENTS, &["good"], 2, &[]);
    });
    assert!(find(&build, EventKind::Sensor, &["bad"]).is_none());
    assert!(find(&build, EventKind::Sensor, &["good"]).is_some());
    let layer = build.layers.get(LayerId(1)).unwrap();
    assert!(layer
        .origins
        .iter()
        .any(|(name, origin)| name.starts_with("events ")
            && matches!(origin, Origin::NotRead { cause: build_log_model::NotReadCause::Fault, reason: why } if why.contains("dimensions"))));
    let kept = path(&[
        layout::GLOBAL_SENSORS,
        layout::SENSOR_EVENTS,
        "bad",
        layout::SOURCE_TS,
    ]);
    assert_eq!(
        layer.extra.node(&kept).map(|n| n.dims.as_slice().to_vec()),
        Some(vec![2, 2])
    );
}

#[test]
fn events_nested_beyond_the_depth_that_is_followed_are_reported() {
    let build = synthetic("events-too-deep", |layers, _| {
        let place = sensors_of(&layer(layers, 1));
        let mut group = place.create_group(layout::SENSOR_EVENTS).unwrap();
        for level in 0..(build_log_io::hdf5::MAX_DEPTH + 6) {
            group = group.create_group(&format!("level{level}")).unwrap();
        }
    });
    assert!(build.events.is_empty());
    let layer = build.layers.get(LayerId(1)).unwrap();
    assert!(layer
        .origins
        .iter()
        .any(|(name, _)| name.starts_with("events")));
}

// ---------------------------------------------------------------- what else is held

#[test]
fn what_else_a_series_and_its_places_hold_is_kept_by_its_path() {
    let build = synthetic("events-kept", |layers, root| {
        let place = sensors_of(&layer(layers, 1));
        put_text(&place, "Place_Note", "kept with the place");
        let group = series(
            &place,
            layout::SENSOR_EVENTS,
            &["Gas", "flow"],
            3,
            &[("Quality", 3)],
        );
        put_text(&group, layout::EVENT_DATA_TYPE, "Double");
        put_text(&group, "Server_Note", "kept with the series");
        let top = root.create_group(layout::GLOBAL_SENSORS).unwrap();
        put_text(&top, "Top_Note", "kept with the group beside the mount");
        top.create_group("Elsewhere").unwrap();
    });
    let layer = build.layers.get(LayerId(1)).unwrap();
    // the declared type is typed and so is not kept; the other attribute and the unknown column are
    let node = layer
        .extra
        .node(&path(&[
            layout::GLOBAL_SENSORS,
            layout::SENSOR_EVENTS,
            "Gas",
            "flow",
        ]))
        .expect("the series' group keeps what it holds that is not known");
    let names: Vec<&str> = node.attributes().map(|(name, _)| name).collect();
    assert_eq!(names, ["Server_Note"]);
    assert!(layer
        .extra
        .node(&path(&[
            layout::GLOBAL_SENSORS,
            layout::SENSOR_EVENTS,
            "Gas",
            "flow",
            "Quality"
        ]))
        .is_some());
    // the group of the layer that holds the events keeps its own attribute
    let held = layer
        .extra
        .node(&path(&[layout::GLOBAL_SENSORS]))
        .expect("the events group keeps its attribute");
    assert_eq!(held.attributes().count(), 1);
    // and so does the group beside the mount, by a path that starts above it; the bucket is read and what else is in the group is kept
    let beside: Vec<String> = build.extra.nodes().map(|n| n.path.join("/")).collect();
    assert!(
        beside.iter().any(|p| p == "../Global_Sensors"),
        "{beside:?}"
    );
    assert!(
        beside.iter().any(|p| p == "../Global_Sensors/Elsewhere"),
        "{beside:?}"
    );
}

// ---------------------------------------------------------------- the capabilities

#[test]
fn events_are_declared_in_the_capabilities() {
    let set = common::v17_set();
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Events));
    // reading is declared and writing is not: the writer arrives with IP-08
    assert!(!set
        .capabilities()
        .supports(Direction::Write, Feature::Events));
}
