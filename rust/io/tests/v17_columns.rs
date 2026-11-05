//! The samples of V17 (IP-13 phase 13L, task T13.14; IP-06 decisions D06-3, D06-9 and D06-14): the locator that tells the column engine where each laser channel is stored, and
//! the source that answers a request for a column through the engine.
//!
//! * `every_stored_column_of_the_six_v17_files_matches_its_fingerprint`: each laser channel of each layer of the six V17 miniatures, read through the facade's source in full as
//!   stored, equals the fingerprint that the independent script wrote (IP-06 phase 6A). The March flavor, whose datasets are one chunk, is among them.
//! * `a_ranged_read_equals_a_slice_of_the_full_read`: the same columns by seeded ranges at every precision.
//! * The rest are what V17 says about a request: a channel that is not recorded is `NotPresent`, a recorded empty laser gives an empty column, a dataset that the metadata
//!   promises and the file lacks is `Corrupt` while the other channels still read, a time range is unsupported, a read can be cancelled, and `Samples` is declared.
//! * `the_local_march_layer_reads_in_full` reads one real March layer (3,951,272 values per channel, one chunk each) when the large local fixture is there.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use build_log_conformance::{check_columns, column_expectations, load_all};
use build_log_io::adapters::v17::source::V17Source;
use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_io::hdf5::describe_dataset;
use build_log_model::{
    BuildMeta, CancelToken, ChannelKey, ColumnData, ColumnRange, ColumnRequest, ColumnSource,
    Direction, ErrorKind, Feature, LaserKey, LayerId, Precision, Presence, ReadContext, TimeValue,
};
use build_log_testkit::{large_fixture_or_skip, miniature, miniature_path};
use common::{laser, layer, monitoring, pick_rows, put_text, temp_h5, Rng, Through};
use hdf5::File;

const SIX: [&str; 6] = [
    miniature::V17_MOCK,
    miniature::V17_OCT15,
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
    miniature::V17_MARCH,
    miniature::V17_SPECIAL_LAYERS,
];

/// Every laser channel of a build that is recorded with samples in a layer: the laser, the channel, the layer and the number of samples.
fn recorded_columns(build: &BuildMeta) -> Vec<(LaserKey, ChannelKey, LayerId, u64)> {
    let mut found = Vec::new();
    for laser in build.lasers() {
        for channel in &laser.channels {
            for (layer, presence) in channel.presence.iter() {
                if let Presence::Recorded { samples } = presence {
                    found.push((laser.key.clone(), channel.key.clone(), layer, samples.get()));
                }
            }
        }
    }
    found
}

fn request(laser: &LaserKey, channel: &ChannelKey, layer: LayerId) -> ColumnRequest {
    ColumnRequest::new(layer, Some(laser.clone()), channel.clone())
}

// ---------------------------------------------------------------- the six miniatures

#[test]
fn every_stored_column_of_the_six_v17_files_matches_its_fingerprint() {
    let expectations = load_all().expect("the expectation files");
    let mut columns = 0;
    for name in SIX {
        let expectation = expectations
            .iter()
            .find(|e| e.miniature() == name)
            .unwrap_or_else(|| panic!("no expectation file for {name}"));
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let lines = check_columns(&Through(opened.source()), expectation);
        assert!(lines.is_empty(), "{name}:\n{}", lines.join("\n"));
        // the check had something to check, and every channel recorded with samples was in it
        let described = column_expectations(expectation).expect("the columns").len();
        let recorded = recorded_columns(&opened.source().build().expect("the metadata")).len();
        assert!(described > 0, "{name}: nothing was checked");
        assert_eq!(described, recorded, "{name}");
        columns += described;
    }
    assert!(columns >= 200, "{columns} columns were checked");
}

#[test]
fn a_ranged_read_equals_a_slice_of_the_full_read() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let context = ReadContext::new();
    for name in SIX {
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let source = opened.source();
        let build = source.build().expect("the metadata");
        for (laser, channel, layer, samples) in recorded_columns(&build) {
            let label = format!("{name}: layer {} {} {:?}", layer.0, laser.key, channel);
            let asked = request(&laser, &channel, layer);
            let full = source.read(&asked, &context).expect("the whole column");
            assert_eq!(full.len() as u64, samples, "{label}");
            for _ in 0..4 {
                let rows = pick_rows(&mut rng, full.len(), 10_000);
                let precision = [Precision::Native, Precision::F32, Precision::F64][rng.up_to(2)];
                let got = source
                    .read(
                        &asked
                            .clone()
                            .with_range(ColumnRange::Indices(rows.start as u64..rows.end as u64))
                            .with_precision(precision),
                        &context,
                    )
                    .unwrap_or_else(|e| panic!("{label}: rows {rows:?}: {e}"));
                let want = full
                    .slice(rows.clone())
                    .expect("a slice")
                    .with_precision(precision)
                    .expect("numbers");
                assert_eq!(got, want, "{label}: rows {rows:?} at {precision:?}");
            }
        }
    }
}

#[test]
fn the_march_one_chunk_datasets_read() {
    for name in [miniature::V17_MARCH, miniature::V17_SPECIAL_LAYERS] {
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let source = opened.source();
        let build = source.build().expect("the metadata");
        let (laser, channel, layer, samples) = recorded_columns(&build)
            .into_iter()
            .next()
            .expect("a recorded column");
        // the dataset is one chunk for all its values (G17), which is what makes the March files slow to pan
        let file = File::open(miniature_path(name)).expect("the file");
        let path = format!(
            "{}/{}/{}{}/{}/{}/{}",
            layout::MOUNT,
            layout::MONITORING_DATA,
            layout::LAYER_PREFIX,
            layer.0,
            laser.key,
            layout::LASER_DATA,
            layout::LASER_POWER
        );
        let stored =
            describe_dataset(&file.dataset(&path).expect("the dataset")).expect("described");
        assert_eq!(
            stored.chunks,
            Some(vec![samples as usize]),
            "{name}: one chunk"
        );
        // and it reads in full, and a range inside it is the same values
        let context = ReadContext::new();
        let asked = request(&laser, &channel, layer);
        let full = source.read(&asked, &context).expect("the whole column");
        assert_eq!(full.len() as u64, samples, "{name}");
        let part = source
            .read(
                &asked.with_range(ColumnRange::Indices(1_000..2_000)),
                &context,
            )
            .expect("a range");
        assert_eq!(part, full.slice(1_000..2_000).expect("a slice"), "{name}");
    }
}

// ---------------------------------------------------------------- what V17 says about a request

/// A source over a file made at test time: the layers' group is written by `write`, and the build is read from it.
fn source_over(label: &str, write: impl FnOnce(&hdf5::Group)) -> (common::TempH5, File, V17Source) {
    let on_disk = temp_h5(label, |mount| {
        put_text(mount, layout::BUILD_ID, "a build");
        write(&monitoring(mount));
    });
    let file = File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let build = v17::read::read_build(&mount).expect("the build reads");
    (on_disk, file, V17Source::new(build, mount))
}

fn laser_key(number: u32) -> LaserKey {
    LaserKey::new(format!("{}{number}", layout::LASER_PREFIX), number)
}

fn power() -> ChannelKey {
    ChannelKey::new([layout::LASER_POWER])
}

#[test]
fn a_channel_not_recorded_in_a_layer_is_not_present() {
    let (_disk, _file, source) = source_over("v17-columns-not-present", |layers| {
        laser(&layer(layers, 1), 1, 6, None);
        // layer 2 has no laser group at all: the laser is not recorded there
        layer(layers, 2);
    });
    let context = ReadContext::new();
    let not_present = |asked: ColumnRequest, what: &str| {
        let error = source.read(&asked, &context).expect_err("not present");
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{what}: {error}"
        );
    };
    // recorded: the control
    assert_eq!(
        source
            .read(&request(&laser_key(1), &power(), LayerId(1)), &context)
            .expect("recorded")
            .len(),
        6
    );
    not_present(
        request(&laser_key(1), &power(), LayerId(2)),
        "a layer that has no group for the laser",
    );
    not_present(
        request(&laser_key(1), &power(), LayerId(99)),
        "a layer that the build does not have",
    );
    not_present(
        request(&laser_key(7), &power(), LayerId(1)),
        "a laser that the build does not have",
    );
    not_present(
        request(
            &laser_key(1),
            &ChannelKey::new(["Not_A_Channel"]),
            LayerId(1),
        ),
        "a channel that the laser does not have",
    );
    not_present(
        ColumnRequest::new(LayerId(1), None, power()),
        "a column of the build and not of a laser",
    );
}

#[test]
fn a_recorded_empty_laser_gives_an_empty_column() {
    let (_disk, _file, source) = source_over("v17-columns-empty", |layers| {
        let first = layer(layers, 1);
        laser(&first, 1, 6, None);
        // the second laser is recorded in the layer and holds no sample, as in the files that keep an empty second laser in every layer
        laser(&first, 2, 0, None);
    });
    let context = ReadContext::new();
    for (channel, expected) in [
        (layout::LASER_POWER, "F32"),
        (layout::LASER_STATE, "U8"),
        (layout::ACTUAL_X, "I32"),
    ] {
        let got = source
            .read(
                &request(&laser_key(2), &ChannelKey::new([channel]), LayerId(1)),
                &context,
            )
            .unwrap_or_else(|e| panic!("{channel}: {e}"));
        assert!(got.is_empty(), "{channel}");
        // empty, and of the type that is stored
        let typed = matches!(
            (&got, expected),
            (ColumnData::F32(_), "F32") | (ColumnData::U8(_), "U8") | (ColumnData::I32(_), "I32")
        );
        assert!(typed, "{channel}: {got:?}");
    }
}

#[test]
fn a_missing_dataset_is_corrupt_and_the_other_channels_still_read() {
    // the metadata is read while the file has everything, and then a dataset is taken out, so the metadata promises what the file does not hold
    let on_disk = temp_h5("v17-columns-missing", |mount| {
        put_text(mount, layout::BUILD_ID, "a build");
        laser(&layer(&monitoring(mount), 1), 1, 6, None);
    });
    let file = File::open_rw(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let build = v17::read::read_build(&mount).expect("the build reads");
    let source = V17Source::new(build, mount.clone());
    mount
        .group(&format!(
            "{}/{}1/{}1/{}",
            layout::MONITORING_DATA,
            layout::LAYER_PREFIX,
            layout::LASER_PREFIX,
            layout::POSITION
        ))
        .expect("the group of the positions")
        .unlink(layout::ACTUAL_Y)
        .expect("a dataset is removed");

    let context = ReadContext::new();
    let error = source
        .read(
            &request(
                &laser_key(1),
                &ChannelKey::new([layout::ACTUAL_Y]),
                LayerId(1),
            ),
            &context,
        )
        .expect_err("the file does not hold it");
    match error.kind() {
        ErrorKind::Corrupt { detail } => assert!(detail.contains("no dataset"), "{detail}"),
        other => panic!("expected Corrupt, got {other:?}"),
    }
    // located by the engine, and the other channels of the same laser still read
    assert_eq!(error.location().layer, Some(LayerId(1)));
    assert_eq!(error.location().laser, Some(laser_key(1)));
    for channel in [
        layout::ACTUAL_X,
        layout::ACTUAL_Z,
        layout::LASER_POWER,
        layout::LASER_STATE,
    ] {
        let got = source
            .read(
                &request(&laser_key(1), &ChannelKey::new([channel]), LayerId(1)),
                &context,
            )
            .unwrap_or_else(|e| panic!("{channel}: {e}"));
        assert_eq!(got.len(), 6, "{channel}");
    }
}

#[test]
fn a_time_range_is_unsupported_in_v17_and_names_the_version() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("opens");
    let build = opened.source().build().expect("the metadata");
    let (laser, channel, layer, _) = recorded_columns(&build)
        .into_iter()
        .next()
        .expect("a column");
    let asked = request(&laser, &channel, layer).with_range(ColumnRange::Time {
        from: TimeValue::from_file(0, None),
        to: TimeValue::from_file(10, None),
    });
    let error = opened
        .source()
        .read(&asked, &ReadContext::new())
        .expect_err("not served");
    match error.kind() {
        ErrorKind::UnsupportedInVersion { version, feature } => {
            assert_eq!(version, v17::KEY);
            assert!(feature.contains("segment table"), "{feature}");
        }
        other => panic!("expected UnsupportedInVersion, got {other:?}"),
    }
}

#[test]
fn a_read_can_be_cancelled() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("opens");
    let build = opened.source().build().expect("the metadata");
    let (laser, channel, layer, samples) = recorded_columns(&build)
        .into_iter()
        .next()
        .expect("a column");
    assert!(samples > 10_000, "the column has more than one block");
    let asked = request(&laser, &channel, layer);

    // cancelled by the first progress report: nothing is returned
    let token = CancelToken::new();
    let reports = Arc::new(AtomicUsize::new(0));
    let (cancel, count) = (token.clone(), Arc::clone(&reports));
    let context = ReadContext::new()
        .with_cancel(token)
        .with_progress(Arc::new(move |_, _| {
            count.fetch_add(1, Ordering::SeqCst);
            cancel.cancel();
        }));
    let outcome = opened.source().read(&asked, &context);
    assert!(matches!(
        outcome.expect_err("cancelled").kind(),
        ErrorKind::Cancelled
    ));
    assert_eq!(
        reports.load(Ordering::SeqCst),
        1,
        "no block is read after the token fires"
    );

    // cancelled before the read
    let token = CancelToken::new();
    token.cancel();
    let outcome = opened
        .source()
        .read(&asked, &ReadContext::new().with_cancel(token));
    assert!(matches!(
        outcome.expect_err("cancelled").kind(),
        ErrorKind::Cancelled
    ));
}

#[test]
fn samples_are_declared_in_the_capabilities() {
    let set = common::v17_set();
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Samples));
    // reading is declared and writing is not: the writer arrives with IP-08
    assert!(!set
        .capabilities()
        .supports(Direction::Write, Feature::Samples));
}

// ---------------------------------------------------------------- the cache, through the source (IP-06 phase 6C)

#[test]
fn reads_with_a_cache_budget_equal_reads_without_one_for_every_column_of_the_six_files() {
    let plain = ReadContext::new();
    let kept = ReadContext::new().with_cache_budget(64 << 20);
    for name in SIX {
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let source = opened.source();
        assert_eq!(
            source.cache_stats(),
            Some(Default::default()),
            "{name}: a cache, and nothing in it yet"
        );
        let build = source.build().expect("the metadata");
        let columns = recorded_columns(&build);
        for (laser, channel, layer, _) in &columns {
            let asked = request(laser, channel, *layer);
            let direct = source.read(&asked, &plain).expect("no budget");
            // cold (the blocks are decoded and kept) and warm (they are served from the cache): the same column
            let cold = source.read(&asked, &kept).expect("cold");
            let warm = source.read(&asked, &kept).expect("warm");
            assert_eq!(
                cold, direct,
                "{name}: layer {} {} {:?}",
                layer.0, laser.key, channel
            );
            assert_eq!(
                warm, direct,
                "{name}: layer {} {} {:?}",
                layer.0, laser.key, channel
            );
        }
        let stats = source.cache_stats().expect("stats");
        assert!(
            stats.hits > 0 && stats.bytes_held > 0 && stats.bytes_held <= 64 << 20,
            "{name}: {stats:?}"
        );
        assert_eq!(stats.evictions, 0, "{name}");
    }
}

#[test]
fn the_local_march_layer_is_served_warm_from_the_cache() {
    let path = large_fixture_or_skip!("extracts/v17_march_layer_1.h5");
    let opened = open(production(), &path).expect("the real March layer opens");
    let source = opened.source();
    let build = source.build().expect("the metadata");
    let columns = recorded_columns(&build);
    // the eight channels of the first laser in the first layer
    let (first_laser, _, first_layer, _) = columns[0].clone();
    let eight: Vec<_> = columns
        .into_iter()
        .filter(|(laser, _, layer, _)| *laser == first_laser && *layer == first_layer)
        .collect();
    assert_eq!(eight.len(), 8);
    let kept = ReadContext::new().with_cache_budget(256 << 20);
    let plain = ReadContext::new();
    let timed = |context: &ReadContext| {
        let start = std::time::Instant::now();
        let all: Vec<ColumnData> = eight
            .iter()
            .map(|(laser, channel, layer, _)| {
                source
                    .read(&request(laser, channel, *layer), context)
                    .expect("a column")
            })
            .collect();
        (start.elapsed(), all)
    };
    // numbers for the plan (Appendix B): with no budget every read decodes; with one, the first pass decodes and keeps, and the second is served from the cache
    let (uncached, direct) = timed(&plain);
    let (cold, first) = timed(&kept);
    let (warm, second) = timed(&kept);
    eprintln!(
        "the eight channels of one March laser-layer ({} values each): no budget {uncached:?}, cold with a budget {cold:?}, warm {warm:?}",
        first[0].len()
    );
    for ((direct, first), second) in direct.iter().zip(&first).zip(&second) {
        assert_eq!(first, direct);
        assert_eq!(second, direct);
    }
    // a dataset of one chunk is one block: every warm read is a hit. The pass with no budget looked for each block too (and found none), so the misses are those of two passes
    let stats = source.cache_stats().expect("stats");
    assert_eq!(
        (stats.hits, stats.misses, stats.evictions),
        (8, 16, 0),
        "{stats:?}"
    );
    // the eight blocks are held: 29 bytes for each sample of a laser-layer (a float32, a byte and six int32), about 115 MB for this one
    let values = first[0].len() as u64;
    assert_eq!(stats.bytes_held, 29 * values, "{stats:?}");
    // a range inside the block is a slice of it, with no copy: warm, it costs a lookup
    let (laser, channel, layer, _) = &eight[0];
    let start = std::time::Instant::now();
    let part = source
        .read(
            &request(laser, channel, *layer).with_range(ColumnRange::Indices(1_000_000..1_100_000)),
            &kept,
        )
        .expect("a range");
    eprintln!(
        "a range of 100,000 values of one warm channel: {:?}",
        start.elapsed()
    );
    assert_eq!(part, first[0].slice(1_000_000..1_100_000).expect("a slice"));
}

// ---------------------------------------------------------------- a real layer, when the local file is there

#[test]
fn the_local_march_layer_reads_in_full() {
    let path = large_fixture_or_skip!("extracts/v17_march_layer_1.h5");
    let opened = open(production(), &path).expect("the real March layer opens");
    let source = opened.source();
    let build = source.build().expect("the metadata");
    let file = File::open(&path).expect("the file");
    let context = ReadContext::new();
    let columns = recorded_columns(&build);
    assert!(columns.len() >= 8, "{} columns", columns.len());
    for (laser, channel, layer, samples) in &columns {
        let got = source
            .read(&request(laser, channel, *layer), &context)
            .unwrap_or_else(|e| panic!("{} {channel:?}: {e}", laser.key));
        assert_eq!(got.len() as u64, *samples, "{} {channel:?}", laser.key);
    }
    // two of them against the binding's own read of the same datasets: the power and one position, of the first laser in the file
    let (laser, _, layer, samples) = &columns[0];
    let base = format!(
        "{}/{}/{}{}/{}",
        layout::MOUNT,
        layout::MONITORING_DATA,
        layout::LAYER_PREFIX,
        layer.0,
        laser.key
    );
    assert!(
        *samples > 1_000_000,
        "a real layer holds millions of samples: {samples}"
    );
    let direct_power = file
        .dataset(&format!(
            "{base}/{}/{}",
            layout::LASER_DATA,
            layout::LASER_POWER
        ))
        .expect("the power")
        .read_raw::<f32>()
        .expect("read directly");
    let power = source
        .read(&request(laser, &power(), *layer), &context)
        .expect("the power");
    assert_eq!(
        power,
        ColumnData::F32(build_log_model::Shared::new(direct_power))
    );
    let direct_x = file
        .dataset(&format!("{base}/{}/{}", layout::POSITION, layout::ACTUAL_X))
        .expect("a position")
        .read_raw::<i32>()
        .expect("read directly");
    let x = source
        .read(
            &request(laser, &ChannelKey::new([layout::ACTUAL_X]), *layer),
            &context,
        )
        .expect("the position");
    assert_eq!(x, ColumnData::I32(build_log_model::Shared::new(direct_x)));
}
