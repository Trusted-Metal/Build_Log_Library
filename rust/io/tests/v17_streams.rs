//! A laser's stream in a layer, from its channels (IP-07 phase 7A, task T07.1; decisions D07-1 and D07-2).
//!
//! The reader said a laser's stream is as long as its **power**. tauri's writer omits every empty channel, so a laser with positions and no power is a normal output of it, and the library hid
//! all of that laser's samples; and a dataset with a **null dataspace** (HDF5 allows a type, no data and no dimension) was refused as corrupt, which for the power removed the whole laser.
//!
//! * The stream is as long as its **longest channel**. A channel that is not there is not a difference. Where the channels that are there differ, the stored samples carry the origin
//!   `LongestChannel`, nothing is cut or padded, and each channel keeps its own count.
//! * A channel with a null dataspace is **recorded and empty** with the origin `NullDataspace` on the laser's entry for the layer; a channel of length zero is recorded and empty with none.
//! * `the_twelve_edge_case_scenarios_give_the_expected_presence`: the owner's twelve scenarios (the actual positions, the commanded positions, the power or the state, each null, of length zero
//!   or removed), made at test time from the shape of the fixture of 2026-08-12 (no `Actual_Z`; `int16` commanded positions).
//! * A laser's number is read as a number, and `Laser_10` comes after `Laser_9`.
//!
//! The expectation of each file is written out here, and the oracle's tests (`tools/fixtures/tests/test_expectations.py`) hold the same rule on files made from the mock miniature, by `h5py`.

mod common;

use build_log_conformance::{expectations_dir, load_all};
use build_log_io::adapters::v17::{layers::read_layers, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    Axis, BuildIdentity, BuildMeta, BuildSource, ChannelKind, LaserKey, LayerId, Origin, Presence,
};
use build_log_testkit::{miniature, miniature_path};
use common::{layer, monitoring, put_array, temp_h5};
use hdf5::Group;

/// How many samples each channel of the base file holds.
const N: usize = 1_000;

/// The four groups of channels of the fixture, which are what a scenario changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Actual,
    Commanded,
    Power,
    State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    Null,
    Zero,
    Removed,
}

impl Part {
    fn names(self) -> &'static [&'static str] {
        match self {
            Part::Actual => &[layout::ACTUAL_X, layout::ACTUAL_Y],
            Part::Commanded => &[
                layout::COMMANDED_X,
                layout::COMMANDED_Y,
                layout::COMMANDED_Z,
            ],
            Part::Power => &[layout::LASER_POWER],
            Part::State => &[layout::LASER_STATE],
        }
    }
}

const PARTS: [Part; 4] = [Part::Actual, Part::Commanded, Part::Power, Part::State];

fn empty_build() -> BuildMeta {
    BuildMeta::new(BuildIdentity {
        build_id: Some("a build".into()),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    })
}

/// One dataset of `T`: `samples` values, none (length zero), or a null dataspace.
fn make<T: hdf5::H5Type>(group: &Group, name: &str, samples: usize, how: Option<How>) {
    match how {
        Some(How::Removed) => {}
        Some(How::Null) => {
            // `empty` alone makes a scalar, which holds one value; a null dataspace is asked for
            group
                .new_dataset_builder()
                .empty::<T>()
                .shape(hdf5::Extents::Null)
                .create(name)
                .expect("a dataset with a null dataspace");
        }
        Some(How::Zero) => {
            group
                .new_dataset::<T>()
                .shape(0)
                .create(name)
                .expect("a dataset of length zero");
        }
        None => {
            group
                .new_dataset::<T>()
                .shape(samples)
                .create(name)
                .expect("a dataset");
        }
    }
}

/// A laser in a layer with the channels of the fixture of 2026-08-12: the power as `float32`, the state as `uint8`, the actual positions X and Y as `int32`, the commanded positions as
/// `int16`, **no `Actual_Z`**. `changes` says what happens to the channels of a part.
fn fixture_laser(layer: &Group, number: u32, samples: usize, changes: &[(Part, How)]) -> Group {
    let group = layer
        .create_group(&format!("{}{number}", layout::LASER_PREFIX))
        .expect("a laser's group");
    let how = |part: Part| changes.iter().find(|(p, _)| *p == part).map(|(_, h)| *h);
    let data = group.create_group(layout::LASER_DATA).expect("laser data");
    make::<f32>(&data, layout::LASER_POWER, samples, how(Part::Power));
    make::<u8>(&data, layout::LASER_STATE, samples, how(Part::State));
    let position = group.create_group(layout::POSITION).expect("positions");
    for name in Part::Actual.names() {
        make::<i32>(&position, name, samples, how(Part::Actual));
    }
    for name in Part::Commanded.names() {
        make::<i16>(&position, name, samples, how(Part::Commanded));
    }
    group
}

/// A build read from one mount that `write` fills.
fn read_mount(label: &str, write: impl FnOnce(&Group)) -> BuildMeta {
    let on_disk = temp_h5(label, write);
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let mut build = empty_build();
    read_layers(&mount, &mut build).expect("the layers and lasers are read");
    build
}

/// The first laser of a one-layer file made by `fixture_laser` with `changes`.
fn read_fixture(label: &str, changes: &[(Part, How)]) -> BuildMeta {
    read_mount(label, |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        fixture_laser(&first, 1, N, changes);
    })
}

fn laser_one() -> LaserKey {
    LaserKey::new("Laser_1", 1)
}

fn stream(build: &BuildMeta) -> &build_log_model::LaserLayerMeta {
    build
        .laser(&laser_one())
        .expect("the laser")
        .layers
        .get(LayerId(1))
        .expect("the laser's entry for the layer")
}

/// The presence of the channel with this dataset name in the first layer, or `None` if the laser has no such channel in it.
fn channel(build: &BuildMeta, name: &str) -> Option<Presence> {
    let kind = match name {
        n if n == layout::LASER_POWER => ChannelKind::LaserPower,
        n if n == layout::LASER_STATE => ChannelKind::LaserState,
        n if n == layout::ACTUAL_X => ChannelKind::ActualPosition { axis: Axis::X },
        n if n == layout::ACTUAL_Y => ChannelKind::ActualPosition { axis: Axis::Y },
        n if n == layout::COMMANDED_X => ChannelKind::CommandedPosition { axis: Axis::X },
        n if n == layout::COMMANDED_Y => ChannelKind::CommandedPosition { axis: Axis::Y },
        n if n == layout::COMMANDED_Z => ChannelKind::CommandedPosition { axis: Axis::Z },
        other => panic!("{other} is not a channel of the fixture"),
    };
    build
        .laser(&laser_one())
        .expect("the laser")
        .channels
        .iter()
        .find(|c| c.kind == kind)
        .and_then(|c| c.presence.get(LayerId(1)).copied())
}

fn recorded(samples: usize) -> Presence {
    Presence::from_count(samples as u64)
}

// ---------------------------------------------------------------- the stream is as long as its longest channel

#[test]
fn a_laser_with_positions_and_no_power_is_recorded_with_the_positions_count() {
    // tauri omits every empty channel: no power, no state, or neither
    for (label, removed) in [
        ("no-power", vec![(Part::Power, How::Removed)]),
        ("no-state", vec![(Part::State, How::Removed)]),
        (
            "neither",
            vec![(Part::Power, How::Removed), (Part::State, How::Removed)],
        ),
    ] {
        let build = read_fixture(&format!("v17-streams-{label}"), &removed);
        let entry = stream(&build);
        assert_eq!(
            entry.presence,
            recorded(N),
            "{label}: the laser is recorded"
        );
        assert_eq!(
            entry.time_axis.stored_samples,
            Some(N as u64),
            "{label}: and its stored samples are the positions'"
        );
        // the channels that are there agree, and a channel that is not there is not a difference: no origin of any kind
        assert!(entry.origins.is_empty(), "{label}: {:?}", entry.origins);
        // each channel that is there keeps its count, and the one that is not is not described for this layer
        for name in Part::Actual.names().iter().chain(Part::Commanded.names()) {
            assert_eq!(channel(&build, name), Some(recorded(N)), "{label} {name}");
        }
        for (part, _) in &removed {
            for name in part.names() {
                assert_eq!(channel(&build, name), None, "{label} {name}");
            }
        }
    }
}

#[test]
fn a_laser_whose_channels_differ_takes_the_longest_and_says_so() {
    // the power has a few samples and the positions have many: nothing is cut, and the stream is the longer
    let build = read_mount("v17-streams-differ", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = fixture_laser(&first, 1, N, &[(Part::Power, How::Removed)]);
        let data = group.group(layout::LASER_DATA).expect("laser data");
        data.new_dataset::<f32>()
            .shape(10)
            .create(layout::LASER_POWER)
            .expect("a short power");
    });
    let entry = stream(&build);
    assert_eq!(entry.presence, recorded(N));
    assert_eq!(entry.time_axis.stored_samples, Some(N as u64));
    assert_eq!(
        entry.origins.get("stored_samples"),
        Some(&Origin::LongestChannel)
    );
    // every channel keeps its own count: the power is not padded, and the positions are not cut
    assert_eq!(channel(&build, layout::LASER_POWER), Some(recorded(10)));
    assert_eq!(channel(&build, layout::COMMANDED_X), Some(recorded(N)));
    // a channel of length zero is a difference too
    let build = read_fixture("v17-streams-zero-power", &[(Part::Power, How::Zero)]);
    assert_eq!(stream(&build).presence, recorded(N));
    assert_eq!(
        stream(&build).origins.get("stored_samples"),
        Some(&Origin::LongestChannel)
    );
    assert_eq!(
        channel(&build, layout::LASER_POWER),
        Some(Presence::RecordedEmpty)
    );
}

#[test]
fn a_laser_of_empty_channels_and_a_laser_with_no_channel_are_recorded_empty_with_no_origin_for_the_stream(
) {
    let build = read_mount("v17-streams-empty", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        // a laser whose every channel has no sample, and a laser whose group holds nothing at all
        fixture_laser(
            &first,
            1,
            N,
            &[
                (Part::Actual, How::Zero),
                (Part::Commanded, How::Zero),
                (Part::Power, How::Zero),
                (Part::State, How::Zero),
            ],
        );
        first
            .create_group(&format!("{}2", layout::LASER_PREFIX))
            .expect("a laser with no channel");
    });
    for number in [1, 2] {
        let laser = build
            .laser(&LaserKey::new(format!("Laser_{number}"), number))
            .expect("the laser");
        let entry = laser.layers.get(LayerId(1)).expect("the entry");
        assert_eq!(entry.presence, Presence::RecordedEmpty, "laser {number}");
        assert_eq!(entry.time_axis.stored_samples, Some(0), "laser {number}");
        // all the channels that are there are empty, so they agree: the stream has no origin
        assert!(
            entry.origins.is_empty(),
            "laser {number}: {:?}",
            entry.origins
        );
    }
}

// ---------------------------------------------------------------- a null dataspace is recorded and empty

#[test]
fn a_null_dataspace_channel_is_recorded_empty_with_an_origin() {
    let build = read_fixture(
        "v17-streams-null-commanded",
        &[(Part::Commanded, How::Null)],
    );
    let entry = stream(&build);
    for name in Part::Commanded.names() {
        // recorded, empty, and the entry says it is null; the channel is still described
        assert_eq!(
            channel(&build, name),
            Some(Presence::RecordedEmpty),
            "{name}"
        );
        assert_eq!(
            entry.origins.get(&format!("channel {name}")),
            Some(&Origin::NullDataspace),
            "{name}"
        );
        // the typed lookup holds the same origin as the text key (IP-26 phase 26C)
        assert_eq!(
            entry.channel_origin(name),
            Some(&Origin::NullDataspace),
            "{name}"
        );
    }
    // the others have no origin, and the stream is theirs, with the origin that the channels differ
    for name in Part::Actual.names() {
        assert_eq!(channel(&build, name), Some(recorded(N)), "{name}");
        assert!(entry.origins.get(&format!("channel {name}")).is_none());
        assert!(entry.channel_origin(name).is_none(), "{name}");
    }
    assert_eq!(entry.presence, recorded(N));
    assert_eq!(
        entry.origins.get("stored_samples"),
        Some(&Origin::LongestChannel)
    );
    // a dataset of length zero is empty too, and it is not null: it has no origin of its own
    let zero = read_fixture(
        "v17-streams-zero-commanded",
        &[(Part::Commanded, How::Zero)],
    );
    assert_eq!(
        channel(&zero, layout::COMMANDED_X),
        Some(Presence::RecordedEmpty)
    );
    assert!(stream(&zero)
        .origins
        .get(&format!("channel {}", layout::COMMANDED_X))
        .is_none());
}

#[test]
fn a_null_power_does_not_remove_the_laser() {
    // the power was refused as corrupt, and the whole laser went with it, though the other channels hold every sample
    let build = read_fixture("v17-streams-null-power", &[(Part::Power, How::Null)]);
    let entry = stream(&build);
    assert_eq!(entry.presence, recorded(N), "the laser is recorded");
    assert_eq!(
        channel(&build, layout::LASER_POWER),
        Some(Presence::RecordedEmpty)
    );
    assert_eq!(
        entry
            .origins
            .get(&format!("channel {}", layout::LASER_POWER)),
        Some(&Origin::NullDataspace)
    );
    assert_eq!(channel(&build, layout::ACTUAL_X), Some(recorded(N)));
    assert_eq!(channel(&build, layout::COMMANDED_Z), Some(recorded(N)));
    // and the laser is in the layer's list of lasers
    let lasers = build.lasers();
    assert_eq!(lasers.len(), 1);
    assert!(lasers[0].layers.contains(LayerId(1)));
}

#[test]
fn a_power_that_cannot_be_read_leaves_the_laser_recorded_from_its_other_channels() {
    // a power that is a table is damage, not a null dataspace: it is not read, with its reason, as any channel is (D13-21), and it no longer takes the laser with it
    let build = read_mount("v17-streams-table-power", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = fixture_laser(&first, 1, N, &[(Part::Power, How::Removed)]);
        let data = group.group(layout::LASER_DATA).expect("laser data");
        data.new_dataset::<f32>()
            .shape((2, 2))
            .create(layout::LASER_POWER)
            .expect("a table");
    });
    let entry = stream(&build);
    assert_eq!(entry.presence, recorded(N));
    match entry
        .origins
        .get(&format!("channel {}", layout::LASER_POWER))
    {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, build_log_model::NotReadCause::Fault);
            assert!(reason.contains("dimensions"), "{reason}");
        }
        other => panic!("expected a reason, got {other:?}"),
    }
    assert_eq!(channel(&build, layout::LASER_POWER), None);
    assert!(entry.origins.get("stored_samples").is_none());
}

#[test]
fn a_channel_that_cannot_be_read_is_recorded_as_not_read_with_its_cause() {
    // IP-07 phase 7B: a channel that is a table is damage of the file, and the origin on the laser's entry for the layer says so by a type and not by a sentence
    let build = read_mount("v17-streams-table-channel", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = fixture_laser(&first, 1, N, &[(Part::Actual, How::Removed)]);
        let position = group.group(layout::POSITION).expect("positions");
        position
            .new_dataset::<i32>()
            .shape((2, 2))
            .create(layout::ACTUAL_X)
            .expect("a table");
    });
    let entry = stream(&build);
    match entry.origins.get(&format!("channel {}", layout::ACTUAL_X)) {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, build_log_model::NotReadCause::Fault);
            assert!(reason.contains("2 dimensions"), "{reason}");
        }
        other => panic!("expected the typed origin, got {other:?}"),
    }
    // the channel is left out of that layer, and the laser is recorded from the others
    assert_eq!(channel(&build, layout::ACTUAL_X), None);
    assert_eq!(entry.presence, recorded(N));
}

#[test]
fn a_group_of_channels_that_is_not_a_group_and_a_channel_that_is_a_group_are_a_fault_of_the_laser()
{
    // IP-07 phase 7E: each group of channels is opened and listed once, and a channel is looked for among the names: what is wrong with them is said as before, on the laser itself
    let by_holder = read_mount("v17-streams-holder-dataset", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = fixture_laser(&first, 1, N, &[]);
        group.unlink(layout::LASER_DATA).expect("the group goes");
        group
            .new_dataset::<f32>()
            .shape(3)
            .create(layout::LASER_DATA)
            .expect("a dataset where the group belongs");
    });
    let by_channel = read_mount("v17-streams-channel-group", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = fixture_laser(&first, 1, N, &[]);
        let data = group.group(layout::LASER_DATA).expect("laser data");
        data.unlink(layout::LASER_POWER).expect("the power goes");
        data.create_group(layout::LASER_POWER)
            .expect("a group where the power belongs");
    });
    for (what, build, said) in [
        (
            "a group of channels that is a dataset",
            by_holder,
            "is not a group",
        ),
        ("a channel that is a group", by_channel, "is not a dataset"),
    ] {
        let entry = stream(&build);
        assert_eq!(entry.presence, Presence::NotRecorded, "{what}");
        match entry.origins.get("laser") {
            Some(Origin::NotRead { cause, reason }) => {
                assert_eq!(*cause, build_log_model::NotReadCause::Fault, "{what}");
                assert!(reason.contains(said), "{what}: {reason}");
            }
            other => panic!("{what}: expected the laser to be not read, got {other:?}"),
        }
    }
}

#[test]
fn a_scalar_channel_is_not_a_null_dataspace_and_is_not_read() {
    // a scalar has no dimension either, but it holds one value: it is not a stream of samples, and it is not null, so it is damage and is reported as any channel that cannot be read is
    let build = read_mount("v17-streams-scalar-power", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = fixture_laser(&first, 1, N, &[(Part::Power, How::Removed)]);
        let data = group.group(layout::LASER_DATA).expect("laser data");
        data.new_dataset::<f32>()
            .create(layout::LASER_POWER)
            .expect("a scalar");
    });
    let entry = stream(&build);
    match entry
        .origins
        .get(&format!("channel {}", layout::LASER_POWER))
    {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, build_log_model::NotReadCause::Fault);
            assert!(reason.contains("0 dimensions"), "{reason}");
        }
        other => panic!("a scalar is not a null dataspace: {other:?}"),
    }
    assert_eq!(channel(&build, layout::LASER_POWER), None);
    assert_eq!(entry.presence, recorded(N));
}

// ---------------------------------------------------------------- the twelve scenarios

#[test]
fn the_twelve_edge_case_scenarios_give_the_expected_presence() {
    let mut checked = 0;
    for part in PARTS {
        for how in [How::Null, How::Zero, How::Removed] {
            let label = format!("{part:?}-{how:?}");
            let build = read_fixture(&format!("v17-streams-s-{label}"), &[(part, how)]);
            let entry = stream(&build);
            // the other three parts hold every sample, so the stream is N, whatever happens to this one
            assert_eq!(
                entry.presence,
                recorded(N),
                "{label}: the laser is recorded"
            );
            assert_eq!(entry.time_axis.stored_samples, Some(N as u64), "{label}");
            // the channels differ unless the part is not there at all
            let differs = how != How::Removed;
            assert_eq!(
                entry.origins.get("stored_samples").cloned(),
                differs.then_some(Origin::LongestChannel),
                "{label}"
            );
            let mut expected_origins = usize::from(differs);
            for other in PARTS {
                for name in other.names() {
                    let found = channel(&build, name);
                    let origin = entry.origins.get(&format!("channel {name}")).cloned();
                    if other != part {
                        assert_eq!(found, Some(recorded(N)), "{label} {name}");
                        assert_eq!(origin, None, "{label} {name}");
                        continue;
                    }
                    match how {
                        How::Null => {
                            assert_eq!(found, Some(Presence::RecordedEmpty), "{label} {name}");
                            assert_eq!(origin, Some(Origin::NullDataspace), "{label} {name}");
                            expected_origins += 1;
                        }
                        How::Zero => {
                            assert_eq!(found, Some(Presence::RecordedEmpty), "{label} {name}");
                            assert_eq!(origin, None, "{label} {name}");
                        }
                        How::Removed => {
                            assert_eq!(found, None, "{label} {name}");
                            assert_eq!(origin, None, "{label} {name}");
                        }
                    }
                }
            }
            // and nothing else is said about the entry
            assert_eq!(entry.origins.iter().count(), expected_origins, "{label}");
            checked += 1;
        }
    }
    assert_eq!(checked, 12);
}

// ---------------------------------------------------------------- stable numbers

#[test]
fn lasers_are_numbered_not_sorted_as_text() {
    // 10 and 11 come after 9, though as text "Laser_10" comes before "Laser_2"
    let build = read_mount("v17-streams-numbers", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        for number in [11u32, 2, 10, 9] {
            fixture_laser(&first, number, 4, &[]);
        }
        // in the second layer only two of them fired
        let second = layer(&layers, 2);
        for number in [10u32, 2] {
            fixture_laser(&second, number, 4, &[]);
        }
        put_array(&second, "A_Layer_Flag", &[1u8]);
    });
    let numbers: Vec<u32> = build.lasers().iter().map(|l| l.key.number).collect();
    assert_eq!(numbers, [2, 9, 10, 11]);
    let names: Vec<&str> = build.lasers().iter().map(|l| l.key.key.as_str()).collect();
    assert_eq!(names, ["Laser_2", "Laser_9", "Laser_10", "Laser_11"]);
    // the lasers of a layer are in the same order, and a layer lists only the lasers that have something in it
    let mut source = build_log_model::InMemorySource::new();
    source.set_build(build);
    let in_first: Vec<u32> = source
        .lasers_in_layer(LayerId(1))
        .expect("the first layer")
        .iter()
        .map(|k| k.number)
        .collect();
    let in_second: Vec<u32> = source
        .lasers_in_layer(LayerId(2))
        .expect("the second layer")
        .iter()
        .map(|k| k.number)
        .collect();
    assert_eq!((in_first, in_second), (vec![2, 9, 10, 11], vec![2, 10]));
}

#[test]
fn a_lasers_channels_are_listed_in_the_layouts_order_whatever_layer_each_was_first_seen_in() {
    // IP-12 phase 12A: the layer-major reader lists a channel when it first meets it. Layer 1 has only the actual positions, layer 2 has the power and the state too, so the power is
    // met after the positions and must still be listed first
    let build = read_mount("v17-streams-channel-order", |mount| {
        let layers = monitoring(mount);
        fixture_laser(
            &layer(&layers, 1),
            1,
            4,
            &[
                (Part::Power, How::Removed),
                (Part::State, How::Removed),
                (Part::Commanded, How::Removed),
            ],
        );
        fixture_laser(&layer(&layers, 2), 1, 4, &[]);
    });
    let kinds: Vec<ChannelKind> = build
        .laser(&laser_one())
        .expect("the laser")
        .channels
        .iter()
        .map(|c| c.kind.clone())
        .collect();
    assert_eq!(
        kinds,
        [
            ChannelKind::LaserPower,
            ChannelKind::LaserState,
            ChannelKind::ActualPosition { axis: Axis::X },
            ChannelKind::ActualPosition { axis: Axis::Y },
            ChannelKind::CommandedPosition { axis: Axis::X },
            ChannelKind::CommandedPosition { axis: Axis::Y },
            ChannelKind::CommandedPosition { axis: Axis::Z },
        ]
    );
}

#[test]
fn a_laser_whose_window_does_not_fit_is_reported_on_itself_and_the_build_opens() {
    // IP-12 phase 12A: a value that does not fit its width is a fact about the file's content, so it is reported on the laser it is in (D13-21) and does not stop the open
    let build = read_mount("v17-streams-window-too-big", |mount| {
        let first = layer(&monitoring(mount), 1);
        let group = fixture_laser(&first, 1, N, &[]);
        put_array(&group, layout::START_LASER_TIME, &[u64::MAX]);
    });
    let entry = stream(&build);
    assert_eq!(entry.presence, Presence::NotRecorded);
    match entry.origins.get("laser") {
        Some(Origin::NotRead { cause, .. }) => {
            assert_eq!(*cause, build_log_model::NotReadCause::Fault)
        }
        other => panic!("expected the typed origin on the laser, got {other:?}"),
    }
}

// ---------------------------------------------------------------- the files that we hold

#[test]
fn the_eight_miniatures_keep_their_expectations() {
    // the rule changes nothing in the files that we hold: every empty laser in them is empty in every channel. The expectation files (written by the independent script) say neither origin...
    for expectation in load_all().expect("the expectation files") {
        let text = std::fs::read_to_string(
            expectations_dir().join(
                std::path::Path::new(expectation.miniature())
                    .with_extension("json")
                    .file_name()
                    .expect("a name"),
            ),
        )
        .expect("the file");
        assert!(
            !text.contains("stored_samples_origin") && !text.contains("NullDataspace"),
            "{}",
            expectation.miniature()
        );
    }
    // ...and the six V17 files, read, carry neither: no stream has an origin of its stored samples, and no channel says it is null
    for name in [
        miniature::V17_MOCK,
        miniature::V17_OCT15,
        miniature::V17_TRIGGERED_2026_10_19,
        miniature::V17_2026_08_18_IMAGES,
        miniature::V17_MARCH,
        miniature::V17_SPECIAL_LAYERS,
    ] {
        let opened = open(production(), miniature_path(name)).expect("a miniature opens");
        let build = opened.source().build().expect("the metadata");
        let mut entries = 0;
        for laser in build.lasers() {
            for (_, entry) in laser.layers.iter() {
                entries += 1;
                assert!(entry.origins.get("stored_samples").is_none(), "{name}");
                assert!(
                    entry
                        .origins
                        .iter()
                        .all(|(_, origin)| *origin != Origin::NullDataspace),
                    "{name}"
                );
            }
        }
        assert!(entries > 0, "{name} holds laser streams");
    }
}
