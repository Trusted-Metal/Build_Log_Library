//! The lasers of a V17 build (IP-13 phase 13D, task T13.5; decisions D13-4, D13-5, D13-6, D13-18 and D13-21).
//!
//! * A laser is a group named for it in a layer; its presence is the length of its power: with samples, empty, or (no group) not recorded.
//! * A channel carries its kind, axis, stored type and the unit that the adapter declares; its presence is by layer.
//! * The time axis is the laser's own: its window is its start and end as written, its period the scanner card's of the optical train of the same number.
//! * A laser or a channel that cannot be read is reported on itself and the build still opens; what else a group holds is kept.
//!
//! The synthetic mounts are made at test time with the binding, and read with the reader itself.

mod common;

use build_log_conformance::{check_parts, expectations_dir, Expectation, Part};
use build_log_io::adapters::v17::{layers::read_layers, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    Accuracy, Axis, BuildIdentity, BuildMeta, ChannelDType, ChannelKind, ClockClass, EnumLabels,
    LaserKey, LayerId, NodeKind, NumericType, Origin, Presence, UnitSource,
};
use build_log_testkit::{miniature, miniature_path};
use common::{dataset, laser, layer, monitoring, put_array, put_scalar, scanner_card, temp_h5};
use hdf5::Group;

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

/// A mount that `write` fills, read into a build with the layers and the lasers.
fn read_mount(label: &str, write: impl FnOnce(&Group)) -> BuildMeta {
    let on_disk = temp_h5(label, write);
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let mut build = empty_build();
    read_layers(&mount, &mut build).expect("the layers and lasers are read");
    build
}

fn expectation(name: &str) -> Expectation {
    Expectation::load(&expectations_dir().join(name)).expect("an expectation file")
}

// ---------------------------------------------------------------- presence

#[test]
fn a_laser_group_without_samples_is_recorded_empty() {
    let build = read_mount("v17-laser-empty", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        laser(&first, 1, 5, None);
        // a laser whose group holds nothing at all, and one whose datasets have no samples: both are in the file and empty
        first
            .create_group(&format!("{}2", layout::LASER_PREFIX))
            .expect("an empty laser group");
        laser(&first, 3, 0, None);
    });
    let key = |n: u32| LaserKey::new(format!("{}{n}", layout::LASER_PREFIX), n);
    let presence = |n: u32| {
        build
            .laser(&key(n))
            .expect("the laser")
            .layers
            .get(LayerId(1))
            .expect("in the layer")
            .presence
    };
    assert_eq!(presence(1).samples(), Some(5));
    assert_eq!(presence(2), Presence::RecordedEmpty);
    assert_eq!(presence(3), Presence::RecordedEmpty);
    let axis = &build
        .laser(&key(3))
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap()
        .time_axis;
    assert_eq!(axis.stored_samples, Some(0));

    // the real files: the second laser of the 2026-10-15 file, and the layers of the special-layer miniature where it did not fire
    let oct15 = open(production(), miniature_path(miniature::V17_OCT15)).expect("opens");
    let build = oct15.source().build().unwrap();
    let second = build.laser(&key(2)).expect("the second laser");
    assert_eq!(
        second.layers.get(LayerId(1)).unwrap().presence,
        Presence::RecordedEmpty
    );
    let special = open(production(), miniature_path(miniature::V17_SPECIAL_LAYERS)).expect("opens");
    let build = special.source().build().unwrap();
    let second = build.laser(&key(2)).expect("the second laser");
    assert_eq!(
        second.layers.get(LayerId(2006)).unwrap().presence.samples(),
        Some(25_000)
    );
    for id in [2007, 2008] {
        assert_eq!(
            second.layers.get(LayerId(id)).unwrap().presence,
            Presence::RecordedEmpty,
            "layer {id}"
        );
    }
}

#[test]
fn a_missing_laser_group_is_not_recorded() {
    let build = read_mount("v17-laser-missing", |mount| {
        let layers = monitoring(mount);
        laser(&layer(&layers, 1), 1, 3, None);
        let second = layer(&layers, 2);
        laser(&second, 1, 3, None);
        laser(&second, 2, 3, None);
    });
    let second = build
        .laser(&LaserKey::new("Laser_2", 2))
        .expect("the laser exists in the build");
    // in layer 1 it has no group: there is no entry, which is the file's "nothing recorded", and not a zero
    assert!(second.layers.get(LayerId(1)).is_none());
    assert_eq!(
        second.layers.get(LayerId(2)).unwrap().presence.samples(),
        Some(3)
    );
    // the lasers are the lasers of all the layers, in numeric order
    let numbers: Vec<u32> = build.lasers().iter().map(|l| l.key.number).collect();
    assert_eq!(numbers, [1, 2]);
}

// ---------------------------------------------------------------- channels

#[test]
fn a_channel_carries_its_kind_axis_type_stored_type_and_declared_unit() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("the gate file opens");
    let build = opened.source().build().unwrap();
    let laser = build
        .laser(&LaserKey::new("Laser_1", 1))
        .expect("the first laser");
    let kinds: Vec<ChannelKind> = laser.channels.iter().map(|c| c.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            ChannelKind::LaserPower,
            ChannelKind::LaserState,
            ChannelKind::ActualPosition { axis: Axis::X },
            ChannelKind::ActualPosition { axis: Axis::Y },
            ChannelKind::ActualPosition { axis: Axis::Z },
            ChannelKind::CommandedPosition { axis: Axis::X },
            ChannelKind::CommandedPosition { axis: Axis::Y },
            ChannelKind::CommandedPosition { axis: Axis::Z },
        ]
    );
    let power = &laser.channels[0];
    assert_eq!(power.dtype, ChannelDType::Numeric(NumericType::F32));
    assert_eq!(
        (power.unit.id.as_deref(), power.unit_source),
        (Some("V"), UnitSource::DeclaredByAdapter)
    );
    assert_eq!(power.clock_class, ClockClass::LaserSynchronous);
    // the state is an enum whose labels nobody declared, and has no unit
    let state = &laser.channels[1];
    assert_eq!(
        state.dtype,
        ChannelDType::Enum {
            labels: EnumLabels::Unknown
        }
    );
    assert_eq!(state.unit_source, UnitSource::Unknown);
    // a position is stored as it is stored (`int32`) and its unit is declared
    let actual_x = &laser.channels[2];
    assert_eq!(actual_x.dtype, ChannelDType::Numeric(NumericType::I32));
    assert_eq!(
        (actual_x.unit.id.as_deref(), actual_x.unit_source),
        (Some("bits"), UnitSource::DeclaredByAdapter)
    );
    assert_eq!(
        actual_x.presence.get(LayerId(1)).and_then(|p| p.samples()),
        Some(25_000)
    );

    // the type is what is stored: the same channel stored as `float64` is described as `float64`
    let build = read_mount("v17-channel-types", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = first.create_group("Laser_1").unwrap();
        let data = group.create_group(layout::LASER_DATA).unwrap();
        dataset::<f64>(&data, layout::LASER_POWER, 4);
        dataset::<u16>(&data, layout::LASER_STATE, 4);
    });
    let laser = build.laser(&LaserKey::new("Laser_1", 1)).unwrap();
    assert_eq!(
        laser.channels.len(),
        2,
        "only the channels that the file has are described"
    );
    assert_eq!(
        laser.channels[0].dtype,
        ChannelDType::Numeric(NumericType::F64)
    );
    assert_eq!(
        laser.channels[1].dtype,
        ChannelDType::Enum {
            labels: EnumLabels::Unknown
        }
    );
}

// ---------------------------------------------------------------- the time axis

#[test]
fn the_time_axis_window_is_the_lasers_own_times_as_written() {
    let build = read_mount("v17-laser-window", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        laser(
            &first,
            1,
            10,
            Some((1_792_440_977_031_000, 1_792_440_991_421_396)),
        );
        // only a start, and nothing: a window needs both ends, and none is invented
        let group = laser(&first, 2, 10, None);
        put_array(&group, layout::START_LASER_TIME, &[5i64]);
        laser(&first, 3, 10, None);
    });
    let axis = |n: u32| {
        build
            .laser(&LaserKey::new(format!("Laser_{n}"), n))
            .unwrap()
            .layers
            .get(LayerId(1))
            .unwrap()
            .time_axis
            .clone()
    };
    let window = axis(1).window.expect("a window");
    assert_eq!(
        (window.start.value, window.end.value),
        (1_792_440_977_031_000, 1_792_440_991_421_396)
    );
    for time in [&window.start, &window.end] {
        assert_eq!(
            time.unit_source,
            UnitSource::Unknown,
            "no file says what unit a laser time is in"
        );
        assert!(time.unit.spelling.is_none());
    }
    assert_eq!(axis(1).stored_samples, Some(10));
    assert!(axis(2).window.is_none() && axis(3).window.is_none());

    // the real files: the current output has a window, the March output has none
    let gate = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .unwrap();
    let build = gate.source().build().unwrap();
    let window = build
        .laser(&LaserKey::new("Laser_1", 1))
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap()
        .time_axis
        .window
        .clone()
        .expect("a window");
    // the laser's times are not the layer's: they are kept as written and not converted to one clock
    let layer_start = build
        .layers
        .get(LayerId(1))
        .unwrap()
        .start
        .as_ref()
        .unwrap()
        .value;
    assert_eq!(window.start.value, layer_start * 1000);
    let march = open(production(), miniature_path(miniature::V17_MARCH)).unwrap();
    let build = march.source().build().unwrap();
    let axis = &build
        .laser(&LaserKey::new("Laser_1", 1))
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap()
        .time_axis;
    assert!(axis.window.is_none(), "the March files hold no laser times");
}

#[test]
fn the_sample_period_is_paired_with_the_laser_by_number() {
    let build = read_mount("v17-laser-period", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        laser(&first, 1, 4, None);
        laser(&first, 2, 4, None);
        laser(&first, 3, 4, None);
        // trains 1 and 2 have a scanner card with a period; laser 3 has no train
        scanner_card(mount, 1, 10.0);
        scanner_card(mount, 2, 20.0);
    });
    let axis = |n: u32| {
        build
            .laser(&LaserKey::new(format!("Laser_{n}"), n))
            .unwrap()
            .layers
            .get(LayerId(1))
            .unwrap()
            .time_axis
            .clone()
    };
    assert_eq!(axis(1).period.as_ref().map(|p| p.value), Some(10.0));
    assert_eq!(axis(2).period.as_ref().map(|p| p.value), Some(20.0));
    assert!(
        !axis(1).period.as_ref().unwrap().unit.is_known(),
        "the scanner card gives no unit"
    );
    // with a period the times follow from it (nominal); without one the axis claims no spacing
    assert_eq!(axis(1).accuracy, Accuracy::Nominal);
    assert!(axis(3).period.is_none());
    assert_eq!(axis(3).accuracy, Accuracy::Unknown);
}

// ---------------------------------------------------------------- what cannot be read, and what else a group holds

#[test]
fn a_laser_that_cannot_be_read_is_reported_on_itself_and_the_build_opens() {
    let build = read_mount("v17-laser-unreadable", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        laser(&first, 1, 4, None);
        // the second laser's start is two numbers where one belongs: the laser cannot be read
        let broken = laser(&first, 2, 4, None);
        put_array(&broken, layout::START_LASER_TIME, &[1i64, 2]);
        // in the second layer the first laser has a position that is a table, and a power stored as another type than in the first layer
        let second = layer(&layers, 2);
        let group = laser(&second, 1, 4, None);
        group
            .unlink(layout::POSITION)
            .expect("the positions are replaced");
        let position = group.create_group(layout::POSITION).unwrap();
        position
            .new_dataset::<i32>()
            .shape((2, 2))
            .create(layout::ACTUAL_X)
            .expect("a two-dimensional channel");
        let data = group.group(layout::LASER_DATA).unwrap();
        data.unlink(layout::LASER_POWER).unwrap();
        dataset::<f64>(&data, layout::LASER_POWER, 4);
    });
    // the build opened, and the first laser of the first layer is whole
    let first = build.laser(&LaserKey::new("Laser_1", 1)).unwrap();
    assert_eq!(
        first.layers.get(LayerId(1)).unwrap().presence.samples(),
        Some(4)
    );
    assert!(first.layers.get(LayerId(1)).unwrap().origins.is_empty());

    // the broken laser is not recorded in that layer, with the reason on itself, and what its group holds is kept in the layer's extra
    let broken = build
        .laser(&LaserKey::new("Laser_2", 2))
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap();
    assert_eq!(broken.presence, Presence::NotRecorded);
    match broken.origins.get("laser") {
        Some(Origin::Other(why)) => assert!(
            why.contains("not read") && why.contains("2 values"),
            "{why}"
        ),
        other => panic!("expected the reason, got {other:?}"),
    }
    let layer_one = build.layers.get(LayerId(1)).unwrap();
    let kept: Vec<String> = layer_one.extra.nodes().map(|n| n.path.join("/")).collect();
    assert!(kept.iter().any(|p| p == "Laser_2"), "{kept:?}");

    // in the second layer: the position that is a table is left out of that layer with its reason, and so is the power that is stored as another type
    let in_second = first.layers.get(LayerId(2)).unwrap();
    assert_eq!(in_second.presence.samples(), Some(4));
    for channel in ["channel Actual_X", "channel Laser_Power"] {
        match in_second.origins.get(channel) {
            Some(Origin::Other(why)) => assert!(why.contains("not read"), "{channel}: {why}"),
            other => panic!("expected a reason for {channel}, got {other:?}"),
        }
    }
    let power = first
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::LaserPower)
        .unwrap();
    assert!(power.presence.get(LayerId(1)).is_some() && power.presence.get(LayerId(2)).is_none());
    let x = first
        .channels
        .iter()
        .find(|c| c.kind == ChannelKind::ActualPosition { axis: Axis::X })
        .unwrap();
    assert!(x.presence.get(LayerId(1)).is_some() && x.presence.get(LayerId(2)).is_none());
}

#[test]
fn what_else_a_laser_and_its_layer_hold_is_kept() {
    let build = read_mount("v17-laser-extra", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = laser(&first, 1, 2, None);
        put_array(&group, "complete", &[1u8]);
        let sensors = group.create_group("Synchronous_Analog_Sensors").unwrap();
        put_scalar(&sensors, "Number_Of_Sensors", 0i32);
        // an unlisted dataset next to the power, and an attribute on a group of channels
        let data = group.group(layout::LASER_DATA).unwrap();
        dataset::<f32>(&data, "A_Spare_Column", 3);
        put_scalar(&group.group(layout::POSITION).unwrap(), "A_Note", 9i32);
        // what the layer holds besides its lasers
        first.create_group("Global_Sensors").unwrap();
        dataset::<f64>(&first, "A_Table", 2);
    });
    let entry = build
        .laser(&LaserKey::new("Laser_1", 1))
        .unwrap()
        .layers
        .get(LayerId(1))
        .unwrap();
    // an attribute of the laser that is not typed is kept; the two that are typed are not
    assert!(entry.extra.attribute("complete").is_some());
    assert!(entry.extra.attribute(layout::START_LASER_TIME).is_none());
    let nodes: Vec<(String, NodeKind)> = entry
        .extra
        .nodes()
        .map(|n| (n.path.join("/"), n.kind.clone()))
        .collect();
    assert!(
        nodes.contains(&("Synchronous_Analog_Sensors".to_string(), NodeKind::Group)),
        "{nodes:?}"
    );
    assert!(
        nodes.contains(&("Laser_Data/A_Spare_Column".to_string(), NodeKind::Dataset)),
        "{nodes:?}"
    );
    // the group of the positions is typed, so it is kept only for what it holds that is not a channel: here an attribute
    let position = entry
        .extra
        .node(&["Position".to_string()])
        .expect("the attribute of the group of positions");
    assert!(position.attribute("A_Note").is_some());
    // the channels themselves are not kept twice
    assert!(entry
        .extra
        .nodes()
        .all(|n| !n.path.iter().any(|p| p == layout::LASER_POWER)));
    // and the layer keeps what is not a laser
    let kept: Vec<String> = build
        .layers
        .get(LayerId(1))
        .unwrap()
        .extra
        .nodes()
        .map(|n| n.path.join("/"))
        .collect();
    assert_eq!(kept, ["A_Table", "Global_Sensors"]);
}

// ---------------------------------------------------------------- the real files

#[test]
fn the_lasers_of_the_gate_file_conform() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("the gate file opens");
    let build = opened.source().build().expect("the metadata");
    let found = check_parts(
        &build,
        &expectation("v17_triggered_2026_10_19.json"),
        &[Part::Build, Part::Layers, Part::Lasers],
    );
    assert!(
        found.is_empty(),
        "the lasers do not conform:\n  {}",
        found.join("\n  ")
    );
    assert_eq!(build.lasers().len(), 2);
    assert!(build.lasers().iter().all(|l| l.channels.len() == 8));
}
