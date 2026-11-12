//! The findings and the availability of a V17 build (IP-07 phases 7C and 7D, tasks T07.3 and T07.4; decisions D07-3 to D07-5).
//!
//! * `the_findings_and_the_availability_of_the_six_v17_files_equal_the_expectations`: each V17 miniature, opened through the registry, gives the findings and the availability that the
//!   independent script wrote (the conformance checks), with no sample read.
//! * The rest are what a made file says: a channel shorter than its stream is a warning and degrades only that channel; a laser that cannot be read is a fault, is unreadable, and changes no
//!   other laser; a type the engine cannot read is unsupported and names the feature, and **a read of it says the same**; a layer with no laser is a note; the two reports are made once.

mod common;

use std::sync::Arc;

use build_log_conformance::{check_availability, check_validation, expectations_dir, Expectation};
use build_log_io::adapters::v17::source::V17Source;
use build_log_io::adapters::v17::{layers::read_layers, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    Availability, BuildIdentity, BuildMeta, BuildSource, ChannelKey, ColumnRequest, ColumnSource,
    Direction, ErrorKind, Feature, FindingCode, Item, ItemKind, LaserKey, LayerId, Presence,
    ReadContext,
};
use build_log_testkit::{miniature, miniature_path};
use common::{laser, layer, monitoring, put_array, temp_h5};
use hdf5::Group;

const SIX: [&str; 6] = [
    miniature::V17_MOCK,
    miniature::V17_OCT15,
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
    miniature::V17_MARCH,
    miniature::V17_SPECIAL_LAYERS,
];

fn expectation_of(miniature: &str) -> Expectation {
    let name = std::path::Path::new(miniature).with_extension("json");
    Expectation::load(&expectations_dir().join(name.file_name().expect("a name")))
        .expect("an expectation file")
}

/// A V17 source over a mount that `write` fills (made at test time; the file stays open for the life of the source).
fn source_over(label: &str, write: impl FnOnce(&Group)) -> (common::TempH5, V17Source) {
    let on_disk = temp_h5(label, write);
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let mut build = BuildMeta::new(BuildIdentity {
        build_id: Some("a build".into()),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    });
    read_layers(&mount, &mut build).expect("the layers and lasers are read");
    (on_disk, V17Source::new(build, mount))
}

fn laser_key(number: u32) -> LaserKey {
    LaserKey::new(format!("Laser_{number}"), number)
}

fn channel_item(layer: u32, number: u32, name: &str) -> Item {
    Item::Channel {
        layer: LayerId(layer),
        laser: Some(laser_key(number)),
        channel: ChannelKey::new([name]),
    }
}

// ---------------------------------------------------------------- the six files

#[test]
fn the_findings_and_the_availability_of_the_six_v17_files_equal_the_expectations() {
    let mut items = 0;
    for name in SIX {
        let expectation = expectation_of(name);
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let source = opened.source();
        let found = check_validation(source, &expectation);
        assert!(found.is_empty(), "{name}: {found:?}");
        let found = check_availability(source, &expectation);
        assert!(found.is_empty(), "{name}: {found:?}");
        items += source.availability().expect("a report").counts().total();
    }
    // every layer, every laser in every layer and every channel in the layers where its laser is recorded, in six files
    assert!(items > 300, "{items} items");
}

#[test]
fn the_findings_of_the_special_layer_file_are_the_one_note_that_the_oracle_gives() {
    let opened =
        open(production(), miniature_path(miniature::V17_SPECIAL_LAYERS)).expect("it opens");
    let report = opened.source().validate().expect("a report");
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert_eq!(report.findings[0].code, FindingCode::LayerCountDiffers);
    assert_eq!(report.counts.notes, 1);
    // the other five files have nothing to say
    for name in SIX.iter().filter(|n| **n != miniature::V17_SPECIAL_LAYERS) {
        let opened = open(production(), miniature_path(name)).expect("it opens");
        assert!(
            opened.source().validate().expect("a report").is_clean(),
            "{name}"
        );
    }
}

// ---------------------------------------------------------------- made files

#[test]
fn a_channel_shorter_than_the_stream_is_a_warning_and_degrades_only_that_channel() {
    let (_file, source) = source_over("v17-reports-short", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = laser(&first, 1, 1_000, None);
        let data = group.group(layout::LASER_DATA).expect("laser data");
        data.unlink(layout::LASER_POWER).expect("the power goes");
        data.new_dataset::<f32>()
            .shape(10)
            .create(layout::LASER_POWER)
            .expect("a short power");
    });
    let report = source.validate().expect("a report");
    assert_eq!(
        report.count_of(FindingCode::ChannelShorterThanStream),
        1,
        "{report:?}"
    );
    assert_eq!(report.counts.warnings, 1);
    let finding = &report.findings[0];
    assert_eq!(
        finding.location.channel,
        Some(ChannelKey::new([layout::LASER_POWER]))
    );
    assert_eq!(finding.location.layer, Some(LayerId(1)));
    let availability = source.availability().expect("a report");
    match availability.status(&channel_item(1, 1, layout::LASER_POWER)) {
        Availability::Degraded { issues } => {
            assert_eq!(issues.len(), 1);
            assert!(issues[0].message.contains("10") && issues[0].message.contains("1000"));
        }
        other => panic!("expected Degraded, got {other:?}"),
    }
    // the other seven channels are present, and the laser and the layer are
    assert!(matches!(
        availability.status(&channel_item(1, 1, layout::ACTUAL_X)),
        Availability::Present
    ));
    assert!(matches!(
        availability.status(&Item::Laser {
            layer: LayerId(1),
            laser: laser_key(1)
        }),
        Availability::Present
    ));
    assert_eq!(availability.counts().degraded, 1);
    assert_eq!(availability.exceptions().len(), 1);
    // a degraded channel is still read
    let power = source
        .read(
            &ColumnRequest::new(
                LayerId(1),
                Some(laser_key(1)),
                ChannelKey::new([layout::LASER_POWER]),
            ),
            &ReadContext::new(),
        )
        .expect("the short power reads");
    assert_eq!(power.len(), 10);
}

#[test]
fn a_laser_that_cannot_be_read_is_a_fault_and_unreadable_and_changes_no_other_laser() {
    let (_file, source) = source_over("v17-reports-broken", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        laser(&first, 1, 50, None);
        // the second laser's start is two numbers where one belongs: it cannot be read
        let broken = laser(&first, 2, 50, None);
        put_array(&broken, layout::START_LASER_TIME, &[1i64, 2]);
    });
    let report = source.validate().expect("a report");
    assert_eq!(report.count_of(FindingCode::NotRead), 1, "{report:?}");
    assert_eq!(report.counts.faults, 1);
    assert_eq!(report.findings[0].location.laser, Some(laser_key(2)));
    assert_eq!(report.findings[0].location.layer, Some(LayerId(1)));
    let availability = source.availability().expect("a report");
    match availability.status(&Item::Laser {
        layer: LayerId(1),
        laser: laser_key(2),
    }) {
        Availability::Unreadable { error } => {
            assert!(error.to_string().contains("2 values"), "{error}");
            assert_eq!(error.location().laser, Some(laser_key(2)));
        }
        other => panic!("expected Unreadable, got {other:?}"),
    }
    // laser 1 is whole: present, with all eight channels present, and it reads
    assert!(matches!(
        availability.status(&Item::Laser {
            layer: LayerId(1),
            laser: laser_key(1)
        }),
        Availability::Present
    ));
    for name in [layout::LASER_POWER, layout::ACTUAL_X, layout::COMMANDED_Z] {
        assert!(
            matches!(
                availability.status(&channel_item(1, 1, name)),
                Availability::Present
            ),
            "{name}"
        );
    }
    assert_eq!(availability.counts_for(ItemKind::Laser).unreadable, 1);
    assert_eq!(availability.counts_for(ItemKind::Laser).present, 1);
    assert_eq!(availability.exceptions().len(), 1);
    let read = source
        .read(
            &ColumnRequest::new(
                LayerId(1),
                Some(laser_key(1)),
                ChannelKey::new([layout::LASER_POWER]),
            ),
            &ReadContext::new(),
        )
        .expect("the first laser still reads");
    assert_eq!(read.len(), 50);
}

#[test]
fn a_type_the_engine_cannot_read_is_unsupported_and_a_read_of_it_says_the_same() {
    let (_file, source) = source_over("v17-reports-unsupported", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        let group = laser(&first, 1, 20, None);
        // a position stored as an unsigned 16-bit integer, which the engine does not read (it reads the signed one since phase 6I)
        let position = group.group(layout::POSITION).expect("positions");
        position.unlink(layout::ACTUAL_X).expect("a position goes");
        position
            .new_dataset::<u16>()
            .shape(20)
            .create(layout::ACTUAL_X)
            .expect("an unsigned position");
    });
    let availability = source.availability().expect("a report");
    match availability.status(&channel_item(1, 1, layout::ACTUAL_X)) {
        Availability::Unsupported { feature } => {
            assert!(
                feature.contains("column stored as") && feature.contains("U16"),
                "{feature}"
            );
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    assert!(matches!(
        availability.status(&channel_item(1, 1, layout::ACTUAL_Y)),
        Availability::Present
    ));
    assert_eq!(availability.counts_for(ItemKind::Channel).unsupported, 1);
    // that a type is not read is a fact about the library and not about the file: there is no finding
    assert!(source.validate().expect("a report").is_clean());
    // and the read says what the report says
    let error = source
        .read(
            &ColumnRequest::new(
                LayerId(1),
                Some(laser_key(1)),
                ChannelKey::new([layout::ACTUAL_X]),
            ),
            &ReadContext::new(),
        )
        .expect_err("it is not read");
    assert!(
        matches!(error.kind(), ErrorKind::UnsupportedInVersion { .. }),
        "{error}"
    );
}

#[test]
fn a_layer_with_no_laser_is_a_note() {
    let (_file, source) = source_over("v17-reports-no-laser", |mount| {
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        laser(&first, 1, 5, None);
        // a layer with no laser in it
        layer(&layers, 2);
    });
    let report = source.validate().expect("a report");
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert_eq!(report.findings[0].code, FindingCode::LayerWithoutLasers);
    assert_eq!(report.findings[0].location.layer, Some(LayerId(2)));
    assert_eq!(report.counts.notes, 1);
    let availability = source.availability().expect("a report");
    // a note degrades nothing: the layer is present and the laser is not recorded in it
    assert!(matches!(
        availability.status(&Item::Layer(LayerId(2))),
        Availability::Present
    ));
    assert!(matches!(
        availability.status(&Item::Laser {
            layer: LayerId(2),
            laser: laser_key(1)
        }),
        Availability::NotRecorded
    ));
    assert!(availability.exceptions().is_empty());
}

// ---------------------------------------------------------------- capabilities and availability agree, once

#[test]
fn capabilities_and_availability_agree() {
    // V17 declares that it reads samples, and then every channel that is recorded with samples in the six files is available, and reads
    let set = common::v17_set();
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Samples));
    let context = ReadContext::new();
    let mut agreed = 0;
    for name in SIX {
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let source = opened.source();
        let availability = source.availability().expect("a report");
        assert_eq!(
            availability.counts().unsupported,
            0,
            "{name}: nothing the file holds is a type that the engine does not read"
        );
        let build = source.build().expect("the metadata");
        for laser in build.lasers() {
            for channel in &laser.channels {
                for (layer, presence) in channel.presence.iter() {
                    if !matches!(presence, Presence::Recorded { .. }) {
                        continue;
                    }
                    let item = Item::Channel {
                        layer,
                        laser: Some(laser.key.clone()),
                        channel: channel.key.clone(),
                    };
                    let request =
                        ColumnRequest::new(layer, Some(laser.key.clone()), channel.key.clone());
                    // what the report says is readable reads, and the other way round
                    assert_eq!(
                        availability.status(&item).is_readable(),
                        source.read(&request, &context).is_ok(),
                        "{name} {item:?}"
                    );
                    agreed += 1;
                }
            }
        }
    }
    assert!(agreed >= 200, "{agreed} columns");
}

#[test]
fn the_reports_are_computed_once_for_a_source() {
    let opened = open(production(), miniature_path(miniature::V17_MOCK)).expect("the file opens");
    let source = opened.source();
    let (first, second) = (
        source.validate().expect("a report"),
        source.validate().expect("a report"),
    );
    assert!(
        Arc::ptr_eq(&first, &second),
        "the validation report is the same one"
    );
    let (first, second) = (
        source.availability().expect("a report"),
        source.availability().expect("a report"),
    );
    assert!(
        Arc::ptr_eq(&first, &second),
        "the availability report is the same one"
    );
    // and from several threads at once it is still one
    let reports: Vec<_> = std::thread::scope(|scope| {
        (0..4)
            .map(|_| scope.spawn(|| source.availability().expect("a report")))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().expect("a thread"))
            .collect()
    });
    assert!(reports.iter().all(|r| Arc::ptr_eq(r, &first)));
}
