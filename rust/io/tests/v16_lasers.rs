//! The V16 adapter, part 2: lasers, channels and the sample period (IP-12 phase 12D, task T12.4; decisions D12-4, D12-5 and D12-9, with the stream rule of D07-1 and D07-2 and the rule of D13-21).
//!
//! * The lasers and channels of the V16 miniature conform to `v16.json` (`Part::Lasers`). **A laser that did not fire keeps its group** (`Laser_2`: its two groups of channels and nothing in them),
//!   so it is recorded and empty in every layer with no channel described.
//! * **The sample period is the build's**: one attribute of the layers' group (`10us`), read as one quantity and given to every laser, with the unit it states and a nominal accuracy. A file without
//!   it has a time axis of unknown accuracy, and **an attribute that is typed is not also kept, and one that nobody was given is kept**.
//! * **V16 has no window and no completeness flag**: an attribute that looks like one is kept and is not read as one.
//! * **The stream rule is V17's rule**, because the reader is the toolkit's: a stream is as long as its longest channel, a null dataspace is a recorded empty channel, a channel that is not there is not a
//!   difference, and a channel or a laser that cannot be read is reported on itself while the build opens.
//!
//! The small mounts are made at test time with the binding, and read with the adapter's own reader.

mod common;

use build_log_conformance::{check_parts, expectations_dir, Expectation, Part};
use build_log_io::adapters::v16::{lasers::optional_embedded_quantity, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    Accuracy, BuildMeta, ChannelKey, ErrorKind, LaserKey, LaserMeta, LayerId, NotReadCause, Origin,
    Presence, UnitSource,
};
use build_log_testkit::{miniature, miniature_path};
use common::v16::{empty_laser, laser, layer, made, monitoring};
use common::{dataset, put_array, put_scalar, put_text, temp_h5};
use hdf5::Group;

fn expectation() -> Expectation {
    Expectation::load(&expectations_dir().join("v16.json")).expect("the V16 expectation file")
}

fn build_of_the_miniature() -> BuildMeta {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    (*opened.source().build().expect("the metadata")).clone()
}

fn laser_of(build: &BuildMeta, number: u32) -> &LaserMeta {
    build
        .laser(&LaserKey::new(format!("Laser_{number}"), number))
        .expect("the laser")
}

fn recorded(samples: u64) -> Presence {
    assert!(samples > 0, "a count of zero is recorded empty");
    Presence::from_count(samples)
}

/// A build whose layers' group has the sample period `period` (none if `None`), with one layer that `write` fills.
fn one_layer(label: &str, period: Option<&str>, write: impl FnOnce(&Group)) -> BuildMeta {
    made(label, |mount| {
        let layers = monitoring(mount);
        if let Some(period) = period {
            put_text(&layers, layout::SAMPLE_PERIOD, period);
        }
        write(&layer(&layers, 1));
    })
    .expect("the build is read")
}

// ---------------------------------------------------------------- the miniature

#[test]
fn the_v16_channels_equal_the_expectation() {
    let build = build_of_the_miniature();
    let found = check_parts(&build, &expectation(), &[Part::Lasers]);
    assert!(
        found.is_empty(),
        "the lasers do not conform:\n  {}",
        found.join("\n  ")
    );
    // two lasers; the first has the eight channels of V17's output, in this order, each recorded in both layers
    assert_eq!(build.lasers().len(), 2);
    let first = laser_of(&build, 1);
    let names: Vec<&str> = first
        .channels
        .iter()
        .map(|c| c.key.name().expect("a name"))
        .collect();
    assert_eq!(
        names,
        [
            "Laser_Power",
            "Laser_State",
            "Actual_X",
            "Actual_Y",
            "Actual_Z",
            "Commanded_X",
            "Commanded_Y",
            "Commanded_Z"
        ]
    );
    for channel in &first.channels {
        for id in [LayerId(1), LayerId(2)] {
            assert_eq!(channel.presence.get(id), Some(&recorded(25_000)));
        }
    }
}

#[test]
fn an_empty_laser_2_is_recorded_empty_in_every_layer() {
    // V16 keeps the group of a laser that did not fire: it is recorded and empty, and no channel of it is described (decision D12-4)
    let build = build_of_the_miniature();
    let second = laser_of(&build, 2);
    assert!(second.channels.is_empty());
    for id in [LayerId(1), LayerId(2)] {
        let entry = second.layers.get(id).expect("recorded in the layer");
        assert_eq!(entry.presence, Presence::RecordedEmpty);
        assert_eq!(entry.time_axis.stored_samples, Some(0));
        // empty is a fact about the stream, and nothing says that anything went wrong
        assert!(entry.origins.is_empty());
    }
    // the first laser is recorded in the same layers
    let first = laser_of(&build, 1);
    for id in [LayerId(1), LayerId(2)] {
        assert_eq!(first.layers.get(id).unwrap().presence, recorded(25_000));
    }
}

#[test]
fn the_sample_period_is_one_build_quantity_for_every_laser() {
    let build = build_of_the_miniature();
    let mut periods = Vec::new();
    for number in [1, 2] {
        for id in [LayerId(1), LayerId(2)] {
            let axis = &laser_of(&build, number).layers.get(id).unwrap().time_axis;
            let period = axis.period.as_ref().expect("every laser has the period");
            assert_eq!(period.value, 10.0);
            assert_eq!(period.unit_source, UnitSource::FromFile);
            assert_eq!(period.unit.spelling.as_deref(), Some("us"));
            // a period that the file gives makes the axis nominal: it is not measured
            assert_eq!(axis.accuracy, Accuracy::Nominal);
            periods.push(period.clone());
        }
    }
    // one quantity: the same for each laser in each layer, even for the laser that is empty
    assert!(periods.windows(2).all(|pair| pair[0] == pair[1]));
    // and it is typed: the attribute is not kept as well
    assert!(build
        .extra
        .node(&[layout::MONITORING_DATA.to_string()])
        .is_none());
}

#[test]
fn a_v16_laser_has_no_window_and_no_flag() {
    let build = build_of_the_miniature();
    for laser in build.lasers() {
        for id in laser.layers.ids() {
            let entry = laser.layers.get(id).unwrap();
            assert!(entry.time_axis.window.is_none(), "{} in {id}", laser.key);
            assert_eq!(entry.complete, None, "{} in {id}", laser.key);
        }
    }
    // an attribute that looks like a window or a flag in another version is not one in V16: it is kept as written and not read
    let build = one_layer("v16-no-window", Some("10us"), |layer| {
        let group = laser(layer, 1, 5);
        put_array(&group, "Start_Laser_Time", &[5i64]);
        put_array(&group, "End_Laser_Time", &[9i64]);
        put_array(&group, "complete", &[1i32]);
    });
    let entry = laser_of(&build, 1).layers.get(LayerId(1)).unwrap();
    assert!(entry.time_axis.window.is_none());
    assert_eq!(entry.complete, None);
    let kept: Vec<&str> = entry.extra.attributes().map(|(name, _)| name).collect();
    assert_eq!(kept, ["End_Laser_Time", "Start_Laser_Time", "complete"]);
}

// ---------------------------------------------------------------- the stream rule is V17's rule

#[test]
fn the_v16_stream_rule_is_the_v17_rule() {
    // a laser is as long as its longest channel (D07-1), and a laser with no power is still recorded from the channels that it has
    let build = one_layer("v16-stream-rule", Some("10us"), |layer| {
        let group = empty_laser(layer, 1);
        let data = group.group(layout::LASER_DATA).unwrap();
        dataset::<u8>(&data, layout::LASER_STATE, 10);
        let position = group.group(layout::POSITION).unwrap();
        dataset::<i32>(&position, layout::ACTUAL_X, 25);
        dataset::<i32>(&position, layout::COMMANDED_X, 25);
    });
    let first = laser_of(&build, 1);
    let entry = first.layers.get(LayerId(1)).unwrap();
    assert_eq!(entry.presence, recorded(25));
    // the channels differ in length, so the stored samples are those of the longest, and the origin says so; nothing is cut or padded
    assert_eq!(
        entry.origins.get("stored_samples"),
        Some(&Origin::LongestChannel)
    );
    let count = |name: &str| {
        first
            .channels
            .iter()
            .find(|c| c.key == ChannelKey::new([name]))
            .and_then(|c| c.presence.get(LayerId(1)).cloned())
    };
    assert_eq!(count(layout::LASER_STATE), Some(recorded(10)));
    assert_eq!(count(layout::ACTUAL_X), Some(recorded(25)));
    // the power is not there, and that is not a difference: it is not described
    assert_eq!(count(layout::LASER_POWER), None);
    assert!(first
        .channels
        .iter()
        .all(|c| c.key != ChannelKey::new([layout::LASER_POWER])));
}

#[test]
fn a_laser_group_with_a_missing_channel_reads_and_says_so() {
    // the laser has seven of its eight channels in this layer and all eight in the other: the missing one is not recorded in the first layer, and the build opens
    let build = made("v16-missing-channel", |mount| {
        let layers = monitoring(mount);
        put_text(&layers, layout::SAMPLE_PERIOD, "10us");
        let first = layer(&layers, 1);
        let group = laser(&first, 1, 7);
        group
            .group(layout::POSITION)
            .unwrap()
            .unlink(layout::ACTUAL_Y)
            .expect("the channel is removed");
        laser(&layer(&layers, 2), 1, 7);
    })
    .expect("the build is read");
    let first = laser_of(&build, 1);
    let actual_y = first
        .channels
        .iter()
        .find(|c| c.key == ChannelKey::new([layout::ACTUAL_Y]))
        .expect("the channel is described, from the layer that has it");
    assert_eq!(
        actual_y.presence.get(LayerId(1)),
        None,
        "not recorded in the first layer"
    );
    assert_eq!(actual_y.presence.get(LayerId(2)), Some(&recorded(7)));
    // the stream is as long as it was, and a channel that is not there is not a difference
    let entry = first.layers.get(LayerId(1)).unwrap();
    assert_eq!(entry.presence, recorded(7));
    assert!(entry.origins.is_empty());
}

#[test]
fn a_null_dataspace_is_a_recorded_empty_channel_and_a_table_is_not_read() {
    let build = one_layer("v16-null-and-table", Some("10us"), |layer| {
        let group = laser(layer, 1, 6);
        // the power is replaced by a dataset that has a type and no data (D07-2)
        let data = group.group(layout::LASER_DATA).unwrap();
        data.unlink(layout::LASER_POWER)
            .expect("the power is removed");
        data.new_dataset_builder()
            .empty::<f32>()
            .shape(hdf5::Extents::Null)
            .create(layout::LASER_POWER)
            .expect("a dataset with a null dataspace");
        // and a position is a table: damage of the file, which is reported on the channel (D13-21)
        let position = group.group(layout::POSITION).unwrap();
        position.unlink(layout::COMMANDED_Z).expect("removed");
        position
            .new_dataset::<i32>()
            .shape((2, 2))
            .create(layout::COMMANDED_Z)
            .expect("a table");
    });
    let first = laser_of(&build, 1);
    let entry = first.layers.get(LayerId(1)).unwrap();
    assert_eq!(
        entry.presence,
        recorded(6),
        "the laser is recorded from the other channels"
    );
    let power = first
        .channels
        .iter()
        .find(|c| c.key == ChannelKey::new([layout::LASER_POWER]))
        .expect("the power is described");
    assert_eq!(
        power.presence.get(LayerId(1)),
        Some(&Presence::RecordedEmpty)
    );
    assert_eq!(
        entry
            .origins
            .get(&format!("channel {}", layout::LASER_POWER)),
        Some(&Origin::NullDataspace)
    );
    // the typed lookup holds the same origins as the text keys (IP-26 phase 26C)
    assert_eq!(
        entry.channel_origin(layout::LASER_POWER),
        Some(&Origin::NullDataspace)
    );
    assert_eq!(
        entry.channel_origin(layout::COMMANDED_Z),
        entry
            .origins
            .get(&format!("channel {}", layout::COMMANDED_Z))
    );
    match entry
        .origins
        .get(&format!("channel {}", layout::COMMANDED_Z))
    {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, NotReadCause::Fault);
            assert!(reason.contains("2 dimensions"), "{reason}");
        }
        other => panic!("expected a reason, got {other:?}"),
    }
    assert!(first
        .channels
        .iter()
        .all(|c| c.key != ChannelKey::new([layout::COMMANDED_Z])));
}

#[test]
fn a_laser_that_cannot_be_read_is_reported_on_itself_and_kept() {
    // `Laser_1` is a dataset and not a group: the laser is not recorded in that layer, with the reason, what is there is kept, and the other laser is read
    let build = one_layer("v16-laser-damage", Some("10us"), |layer| {
        layer
            .new_dataset::<f32>()
            .shape(3)
            .create(format!("{}1", layout::LASER_PREFIX).as_str())
            .expect("a dataset where a laser's group belongs");
        laser(layer, 2, 4);
    });
    let damaged = laser_of(&build, 1).layers.get(LayerId(1)).unwrap();
    assert_eq!(damaged.presence, Presence::NotRecorded);
    match damaged.origins.get("laser") {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, NotReadCause::Fault);
            assert!(reason.contains("not a group"), "{reason}");
        }
        other => panic!("expected a reason, got {other:?}"),
    }
    let layer = build.layers.get(LayerId(1)).unwrap();
    assert!(layer.extra.node(&["Laser_1".to_string()]).is_some());
    assert_eq!(
        laser_of(&build, 2).layers.get(LayerId(1)).unwrap().presence,
        recorded(4)
    );
}

#[test]
fn a_laser_with_no_group_of_channels_is_recorded_and_empty() {
    // the group of a laser that has no group of channels at all: recorded, empty, nothing described
    let build = one_layer("v16-bare-laser", Some("10us"), |layer| {
        layer
            .create_group(format!("{}1", layout::LASER_PREFIX).as_str())
            .expect("a laser's group");
    });
    let bare = laser_of(&build, 1);
    assert!(bare.channels.is_empty());
    assert_eq!(
        bare.layers.get(LayerId(1)).unwrap().presence,
        Presence::RecordedEmpty
    );
}

// ---------------------------------------------------------------- the sample period

#[test]
fn the_period_is_text_with_its_unit_or_a_number_and_a_unit_is_never_guessed() {
    // `10us` states its unit; a number alone and a spelling that nobody has listed are kept as written, with an unknown unit
    let cases: [(&str, f64, UnitSource, Option<&str>); 4] = [
        ("10us", 10.0, UnitSource::FromFile, Some("us")),
        ("10.0", 10.0, UnitSource::Unknown, None),
        ("10 us", 10.0, UnitSource::FromFile, Some("us")),
        ("10ums", 10.0, UnitSource::Unknown, Some("ums")),
    ];
    for (n, (text, value, source, spelling)) in cases.into_iter().enumerate() {
        let build = one_layer(&format!("v16-period-{n}"), Some(text), |layer| {
            laser(layer, 1, 3);
        });
        let axis = &laser_of(&build, 1)
            .layers
            .get(LayerId(1))
            .unwrap()
            .time_axis;
        let period = axis.period.as_ref().expect("the period");
        assert_eq!(period.value, value, "{text}");
        assert_eq!(period.unit_source, source, "{text}");
        assert_eq!(period.unit.spelling.as_deref(), spelling, "{text}");
        assert_eq!(axis.accuracy, Accuracy::Nominal, "{text}");
    }
    // a number stored as a number is the same quantity with no unit
    let build = made("v16-period-number", |mount| {
        let layers = monitoring(mount);
        put_scalar(&layers, layout::SAMPLE_PERIOD, 10.0f32);
        laser(&layer(&layers, 1), 1, 3);
    })
    .expect("the build is read");
    let period = laser_of(&build, 1)
        .layers
        .get(LayerId(1))
        .unwrap()
        .time_axis
        .period
        .clone()
        .expect("the period");
    assert_eq!(
        (period.value, period.unit_source),
        (10.0, UnitSource::Unknown)
    );
}

#[test]
fn a_file_without_a_period_has_a_time_axis_of_unknown_accuracy() {
    let build = one_layer("v16-no-period", None, |layer| {
        laser(layer, 1, 3);
    });
    let axis = &laser_of(&build, 1)
        .layers
        .get(LayerId(1))
        .unwrap()
        .time_axis;
    assert!(axis.period.is_none());
    assert_eq!(axis.accuracy, Accuracy::Unknown);
    // nothing is made up, and nothing is said of a value that the file does not have
    assert!(laser_of(&build, 1).origins.get("sample_period").is_none());
}

#[test]
fn a_period_that_cannot_be_read_is_reported_on_the_laser_and_the_attribute_is_kept() {
    let build = one_layer("v16-bad-period", Some("ten"), |layer| {
        laser(layer, 1, 3);
    });
    let first = laser_of(&build, 1);
    match first.origins.get("sample_period") {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, NotReadCause::Fault);
            assert!(reason.contains(layout::SAMPLE_PERIOD), "{reason}");
        }
        other => panic!("expected a reason, got {other:?}"),
    }
    let axis = &first.layers.get(LayerId(1)).unwrap().time_axis;
    assert!(axis.period.is_none());
    assert_eq!(axis.accuracy, Accuracy::Unknown);
    // nothing the file holds is dropped: the attribute that could not be read is kept
    let kept = build
        .extra
        .node(&[layout::MONITORING_DATA.to_string()])
        .expect("the layers' group is kept for its attribute");
    assert!(kept.attribute(layout::SAMPLE_PERIOD).is_some());
}

#[test]
fn a_build_with_no_laser_keeps_the_sample_period() {
    // there is no laser to give the period to, so the attribute is not typed: it is kept as the file wrote it
    let build = one_layer("v16-no-laser", Some("10us"), |_| {});
    assert!(build.lasers().is_empty());
    let kept = build
        .extra
        .node(&[layout::MONITORING_DATA.to_string()])
        .expect("the layers' group is kept for its attribute");
    assert!(kept.attribute(layout::SAMPLE_PERIOD).is_some());
}

#[test]
fn the_embedded_quantity_reader_takes_a_number_or_text_and_names_the_attribute_it_refuses() {
    let on_disk = temp_h5("v16-embedded", |g| {
        put_scalar(g, "number", 12.5f64);
        put_array(g, "one", &[7i32]);
        put_text(g, "text", "250 ms");
        put_text(g, "words", "soon");
        put_text(g, "empty", "");
        put_array(g, "several", &[1.0f64, 2.0]);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let g = file.group("Mount").expect("the mount");
    let read = |name: &str| optional_embedded_quantity(&g, name);
    let number = read("number").unwrap().expect("a quantity");
    assert_eq!(
        (number.value, number.unit_source),
        (12.5, UnitSource::Unknown)
    );
    assert_eq!(read("one").unwrap().map(|q| q.value), Some(7.0));
    let text = read("text").unwrap().expect("a quantity");
    assert_eq!(
        (text.value, text.unit_source),
        (250.0, UnitSource::FromFile)
    );
    assert!(read("absent").unwrap().is_none());
    for name in ["words", "empty"] {
        let error = read(name).expect_err(name);
        assert!(
            matches!(error.kind(), ErrorKind::Corrupt { detail } if detail.contains(name)),
            "{name}: {:?}",
            error.kind()
        );
    }
    // several values where one belongs are `Corrupt`, and the first is never taken
    assert!(matches!(
        read("several").expect_err("several").kind(),
        ErrorKind::Corrupt { .. }
    ));
}
