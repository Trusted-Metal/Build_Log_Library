//! The samples of V16 (IP-12 phase 12F, task T12.6; decisions D12-9 and D12-12, IP-06 decisions D06-9 and D06-15, and G17): the locator that tells the column engine where each laser channel is
//! stored, and the source that answers a request for a column through the engine.
//!
//! * `v16_samples_read_equal_the_oracles_fingerprints`: each laser channel of each layer of the V16 miniature, read through the facade's source in full as stored, equals the fingerprint that the
//!   independent script wrote (16 columns: the eight channels of the laser that fired, in two layers; the laser that did not fire has none).
//! * `int32_positions_and_u8_state_read_as_stored`: the power is a `float32`, the state a `uint8` and the six positions `int32`, each equal to what the binding itself reads; nothing is narrowed.
//! * **V16 stores each channel as one gzip chunk for all its values** (G17): `a_ranged_read_of_a_one_chunk_dataset_is_served_from_the_cache` shows that a range is a slice of the one decoded block, that the
//!   cache keeps it, and that the next range is a hit.
//! * The rest are what V16 says about a request: a channel that is not recorded is `NotPresent` (and the laser that did not fire has none to read), a dataset that the metadata promises and the file lacks is
//!   `Corrupt` while the other channels still read, a channel of no samples gives an empty column of the type that is stored, a time range is unsupported, a read can be cancelled, and `Samples` is declared.
//! * **The local files** (they skip when absent, and record numbers and do not gate them, D06-15): one real layer of V16 (about two million values a channel, one chunk each) is read in full and served warm
//!   from the cache, and the first read of the 207 MB file is timed.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use build_log_conformance::load_all;
use build_log_conformance::{check_columns, column_expectations, Expectation};
use build_log_io::adapters::v16::columns::V16Locator;
use build_log_io::adapters::v16::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_io::hdf5::describe_dataset;
use build_log_io::toolkit::source::SampleSource;
use build_log_model::{
    BuildMeta, CancelToken, ChannelKey, ColumnData, ColumnRange, ColumnRequest, ColumnSource,
    ErrorKind, LaserKey, LayerId, Precision, Presence, ReadContext, Shared, TimeValue,
};
use build_log_testkit::{large_fixture_or_skip, miniature, miniature_path};
use common::v16::{laser, layer, monitoring, mount_file};
use common::{pick_rows, Rng, Through};
use hdf5::File;

/// The channels of the laser that fired, in the order that the adapter lists them, with the type each is stored as.
const CHANNELS: [(&str, &str); 8] = [
    (layout::LASER_POWER, "F32"),
    (layout::LASER_STATE, "U8"),
    (layout::ACTUAL_X, "I32"),
    (layout::ACTUAL_Y, "I32"),
    (layout::ACTUAL_Z, "I32"),
    (layout::COMMANDED_X, "I32"),
    (layout::COMMANDED_Y, "I32"),
    (layout::COMMANDED_Z, "I32"),
];

fn expectation() -> Expectation {
    load_all()
        .expect("the expectation files")
        .into_iter()
        .find(|e| e.miniature() == miniature::V16)
        .expect("the V16 expectation")
}

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

fn laser_key(number: u32) -> LaserKey {
    LaserKey::new(format!("{}{number}", layout::LASER_PREFIX), number)
}

fn channel(name: &str) -> ChannelKey {
    ChannelKey::new([name])
}

/// The path of a channel's dataset in the miniature.
fn path_of(layer: u32, laser: u32, holder: &str, name: &str) -> String {
    format!(
        "{}/{}/{}{layer}/{}{laser}/{holder}/{name}",
        layout::MOUNT,
        layout::MONITORING_DATA,
        layout::LAYER_PREFIX,
        layout::LASER_PREFIX
    )
}

fn holder_of(name: &str) -> &'static str {
    if name == layout::LASER_POWER || name == layout::LASER_STATE {
        layout::LASER_DATA
    } else {
        layout::POSITION
    }
}

/// The source of a V16 mount that was made at test time: the build is read from it, and the source holds the mount.
type V16Source = SampleSource<V16Locator>;

fn source_from(mount: &hdf5::Group) -> V16Source {
    let build = Arc::new(v16::read::read_build(mount).expect("the build reads"));
    let locator = V16Locator::new(Arc::clone(&build), mount.clone());
    SampleSource::new(build, locator)
}

// ---------------------------------------------------------------- the miniature

#[test]
fn v16_samples_read_equal_the_oracles_fingerprints() {
    let expectation = expectation();
    // the oracle describes every channel that is recorded with samples, and the check cannot pass by having nothing to check
    let described = column_expectations(&expectation).expect("the fingerprints");
    assert_eq!(described.len(), 16);
    assert!(described.iter().all(|c| c.laser == laser_key(1)));
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let found = check_columns(&Through(opened.source()), &expectation);
    assert!(
        found.is_empty(),
        "the columns do not match their fingerprints:\n  {}",
        found.join("\n  ")
    );
    // and what the build says is recorded is what the oracle describes: the same 16 columns
    let build = opened.source().build().expect("the metadata");
    assert_eq!(recorded_columns(&build).len(), 16);
}

#[test]
fn int32_positions_and_u8_state_read_as_stored() {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let source = opened.source();
    let file = File::open(miniature_path(miniature::V16)).expect("the file");
    let context = ReadContext::new();
    for layer in [1u32, 2] {
        for (name, stored) in CHANNELS {
            let got = source
                .read(
                    &request(&laser_key(1), &channel(name), LayerId(layer)),
                    &context,
                )
                .unwrap_or_else(|e| panic!("layer {layer} {name}: {e}"));
            let dataset = file
                .dataset(&path_of(layer, 1, holder_of(name), name))
                .expect("the dataset");
            // the type is the type that the file stores, and every value is what the binding itself reads: nothing is narrowed, widened or converted
            let label = format!("layer {layer} {name}");
            match (stored, got) {
                ("F32", ColumnData::F32(values)) => assert_eq!(
                    values,
                    Shared::new(dataset.read_raw::<f32>().expect("the values")),
                    "{label}"
                ),
                ("U8", ColumnData::U8(values)) => assert_eq!(
                    values,
                    Shared::new(dataset.read_raw::<u8>().expect("the values")),
                    "{label}"
                ),
                ("I32", ColumnData::I32(values)) => assert_eq!(
                    values,
                    Shared::new(dataset.read_raw::<i32>().expect("the values")),
                    "{label}"
                ),
                (stored, other) => panic!("{label}: stored as {stored}, read as {other:?}"),
            }
        }
    }
}

#[test]
fn a_ranged_read_equals_a_slice_of_the_full_read() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let context = ReadContext::new();
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let source = opened.source();
    let build = source.build().expect("the metadata");
    for (laser, channel, layer, samples) in recorded_columns(&build) {
        let label = format!("layer {} {} {:?}", layer.0, laser.key, channel);
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

#[test]
fn a_ranged_read_of_a_one_chunk_dataset_is_served_from_the_cache() {
    let file = File::open(miniature_path(miniature::V16)).expect("the file");
    // V16 stores each channel as one chunk for all its values (G17): the whole dataset is one block
    for (name, _) in CHANNELS {
        let stored = describe_dataset(
            &file
                .dataset(&path_of(1, 1, holder_of(name), name))
                .expect("the dataset"),
        )
        .expect("described");
        assert_eq!(stored.chunks, Some(vec![25_000]), "{name}: one chunk");
    }
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let source = opened.source();
    let asked = request(&laser_key(1), &channel(layout::LASER_POWER), LayerId(1));
    let full = source.read(&asked, &ReadContext::new()).expect("in full");
    let kept = ReadContext::new().with_cache_budget(64 << 20);
    let before = source.cache_stats().expect("a cache");
    // the first range decodes the one block and keeps it; a range in another place of the same dataset is a slice of the block that is held
    let first = source
        .read(
            &asked.clone().with_range(ColumnRange::Indices(1_000..2_000)),
            &kept,
        )
        .expect("a range");
    let after_first = source.cache_stats().expect("stats");
    assert_eq!(after_first.misses, before.misses + 1, "{after_first:?}");
    assert_eq!(after_first.hits, before.hits, "{after_first:?}");
    // 25,000 values of a float32: the whole block is held, though a thousand values were asked for
    assert_eq!(
        after_first.bytes_held,
        25_000 * 4,
        "the block is held whole"
    );
    let second = source
        .read(
            &asked
                .clone()
                .with_range(ColumnRange::Indices(20_000..21_000)),
            &kept,
        )
        .expect("another range");
    let after_second = source.cache_stats().expect("stats");
    assert_eq!(after_second.hits, after_first.hits + 1, "{after_second:?}");
    assert_eq!(after_second.misses, after_first.misses, "{after_second:?}");
    assert_eq!(after_second.bytes_held, after_first.bytes_held);
    assert_eq!(after_second.evictions, 0);
    // and the values are those of the full column, from the block or not
    assert_eq!(first, full.slice(1_000..2_000).expect("a slice"));
    assert_eq!(second, full.slice(20_000..21_000).expect("a slice"));
}

#[test]
fn reads_with_a_cache_budget_equal_reads_without_one_for_every_column() {
    let plain = ReadContext::new();
    let kept = ReadContext::new().with_cache_budget(64 << 20);
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let source = opened.source();
    assert_eq!(
        source.cache_stats(),
        Some(Default::default()),
        "a cache, and nothing in it yet"
    );
    let build = source.build().expect("the metadata");
    for (laser, channel, layer, _) in &recorded_columns(&build) {
        let asked = request(laser, channel, *layer);
        let direct = source.read(&asked, &plain).expect("no budget");
        // cold (the block is decoded and kept) and warm (it is served from the cache): the same column
        let cold = source.read(&asked, &kept).expect("cold");
        let warm = source.read(&asked, &kept).expect("warm");
        assert_eq!(cold, direct, "layer {} {:?}", layer.0, channel);
        assert_eq!(warm, direct, "layer {} {:?}", layer.0, channel);
    }
    let stats = source.cache_stats().expect("stats");
    // the sixteen columns of the file are sixteen blocks, and each warm read is a hit: 16 power-and-state-and-position blocks of 25,000 values (a float32, a byte and six int32 in each layer)
    assert_eq!(stats.hits, 16, "{stats:?}");
    assert_eq!(stats.bytes_held, 2 * 29 * 25_000, "{stats:?}");
    assert_eq!(stats.evictions, 0);
}

// ---------------------------------------------------------------- what V16 says about a request

#[test]
fn a_laser_that_did_not_fire_has_no_column_and_what_is_not_recorded_is_not_present() {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let source = opened.source();
    let context = ReadContext::new();
    let not_present = |asked: ColumnRequest, what: &str| {
        let error = source.read(&asked, &context).expect_err("not present");
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{what}: {error}"
        );
    };
    // the control: recorded
    assert_eq!(
        source
            .read(
                &request(&laser_key(1), &channel(layout::LASER_POWER), LayerId(1)),
                &context
            )
            .expect("recorded")
            .len(),
        25_000
    );
    // the laser that did not fire keeps its group and no channel, so each of the eight channels is not there to read
    for (name, _) in CHANNELS {
        not_present(
            request(&laser_key(2), &channel(name), LayerId(1)),
            "a laser that did not fire",
        );
    }
    not_present(
        request(&laser_key(1), &channel(layout::LASER_POWER), LayerId(99)),
        "a layer that the build does not have",
    );
    not_present(
        request(&laser_key(7), &channel(layout::LASER_POWER), LayerId(1)),
        "a laser that the build does not have",
    );
    not_present(
        request(&laser_key(1), &channel("Not_A_Channel"), LayerId(1)),
        "a channel that the laser does not have",
    );
    not_present(
        ColumnRequest::new(LayerId(1), None, channel(layout::LASER_POWER)),
        "a column of the build and not of a laser",
    );
}

#[test]
fn a_missing_dataset_is_corrupt_and_the_other_channels_still_read() {
    // the metadata is read while the file has everything, and then a dataset is taken out, so the metadata promises what the file does not hold
    let on_disk = mount_file("v16-columns-missing", |mount| {
        laser(&layer(&monitoring(mount), 1), 1, 6);
    });
    let file = File::open_rw(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let source = source_from(&mount);
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
            &request(&laser_key(1), &channel(layout::ACTUAL_Y), LayerId(1)),
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
    for name in [
        layout::ACTUAL_X,
        layout::ACTUAL_Z,
        layout::LASER_POWER,
        layout::LASER_STATE,
    ] {
        let got = source
            .read(
                &request(&laser_key(1), &channel(name), LayerId(1)),
                &context,
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got.len(), 6, "{name}");
    }
}

#[test]
fn a_channel_of_no_samples_gives_an_empty_column_of_the_type_that_is_stored() {
    // a laser whose channels hold no sample: recorded and empty, with its channels described, and each read gives an empty column of its own type
    let on_disk = mount_file("v16-columns-empty", |mount| {
        laser(&layer(&monitoring(mount), 1), 1, 0);
    });
    let file = File::open(&on_disk.0).expect("the file opens");
    let source = source_from(&file.group("Mount").expect("the mount"));
    let context = ReadContext::new();
    for (name, stored) in CHANNELS {
        let got = source
            .read(
                &request(&laser_key(1), &channel(name), LayerId(1)),
                &context,
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(got.is_empty(), "{name}");
        let typed = matches!(
            (&got, stored),
            (ColumnData::F32(_), "F32") | (ColumnData::U8(_), "U8") | (ColumnData::I32(_), "I32")
        );
        assert!(typed, "{name}: {got:?}");
    }
}

#[test]
fn a_time_range_is_unsupported_in_v16_and_names_the_version() {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let asked = request(&laser_key(1), &channel(layout::LASER_POWER), LayerId(1)).with_range(
        ColumnRange::Time {
            from: TimeValue::from_file(0, None),
            to: TimeValue::from_file(10, None),
        },
    );
    let error = opened
        .source()
        .read(&asked, &ReadContext::new())
        .expect_err("not served");
    match error.kind() {
        ErrorKind::UnsupportedInVersion { version, feature } => {
            assert_eq!(version, v16::KEY);
            assert!(feature.contains("segment table"), "{feature}");
        }
        other => panic!("expected UnsupportedInVersion, got {other:?}"),
    }
}

#[test]
fn a_read_is_cancelled_before_it_starts_and_a_one_chunk_column_is_one_block() {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let asked = request(&laser_key(1), &channel(layout::LASER_POWER), LayerId(1));

    // **a column of V16 is one block** (G17): the engine looks at the token between blocks, so a token that fires with the first progress report has no block left to stop. The read is complete, it
    // reported once, and nothing was cut: a consumer that needs to stop a read of a V16 file can do so only before the block is decoded
    let token = CancelToken::new();
    let reports = Arc::new(AtomicUsize::new(0));
    let (cancel, count) = (token.clone(), Arc::clone(&reports));
    let context = ReadContext::new()
        .with_cancel(token)
        .with_progress(Arc::new(move |_, _| {
            count.fetch_add(1, Ordering::SeqCst);
            cancel.cancel();
        }));
    let whole = opened
        .source()
        .read(&asked, &context)
        .expect("the one block was read");
    assert_eq!(whole.len(), 25_000);
    assert_eq!(reports.load(Ordering::SeqCst), 1, "one block, one report");

    // cancelled before the read: nothing is read and the error says so
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

// ---------------------------------------------------------------- the local files (numbers are recorded, not gated: D06-15)

/// The eight channels of the first laser in the first layer of a build, as (laser, channel, layer, samples).
fn the_eight(build: &BuildMeta) -> Vec<(LaserKey, ChannelKey, LayerId, u64)> {
    let columns = recorded_columns(build);
    let (first_laser, _, first_layer, _) = columns[0].clone();
    let eight: Vec<_> = columns
        .into_iter()
        .filter(|(laser, _, layer, _)| *laser == first_laser && *layer == first_layer)
        .collect();
    assert_eq!(eight.len(), 8);
    eight
}

#[test]
fn the_local_v16_layer_reads_in_full_and_is_served_warm_from_the_cache() {
    let path = large_fixture_or_skip!("extracts/v16_layer_1.h5");
    let opened = open(production(), &path).expect("the real V16 layer opens");
    let source = opened.source();
    let build = source.build().expect("the metadata");
    let eight = the_eight(&build);
    let values = eight[0].3;
    assert!(
        values > 1_000_000,
        "a real layer holds millions of samples: {values}"
    );
    let file = File::open(&path).expect("the file");
    let plain = ReadContext::new();
    let kept = ReadContext::new().with_cache_budget(256 << 20);
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
    // numbers for the plan (Appendix D): with no budget every read decodes; with one, the first pass decodes and keeps, and the second is served from the cache
    let (uncached, direct) = timed(&plain);
    let (cold, first) = timed(&kept);
    let (warm, second) = timed(&kept);
    eprintln!(
        "the eight channels of one V16 laser-layer ({values} values each): no budget {uncached:?}, cold with a budget {cold:?}, warm {warm:?}"
    );
    for ((direct, first), second) in direct.iter().zip(&first).zip(&second) {
        assert_eq!(first, direct);
        assert_eq!(second, direct);
    }
    // each channel is one block: every warm read is a hit
    let stats = source.cache_stats().expect("stats");
    assert_eq!(stats.hits, 8, "{stats:?}");
    assert_eq!(stats.evictions, 0, "{stats:?}");
    // a float32, a byte and six int32 for each sample of a laser-layer: 29 bytes
    assert_eq!(stats.bytes_held, 29 * values, "{stats:?}");
    // the power and one position are the values that the binding itself reads
    let (laser, _, layer, _) = &eight[0];
    for (name, at) in [(layout::LASER_POWER, 0usize), (layout::ACTUAL_X, 2)] {
        let direct = file
            .dataset(&path_of(layer.0, laser.number, holder_of(name), name))
            .expect("the dataset");
        let want = if at == 0 {
            ColumnData::F32(Shared::new(direct.read_raw::<f32>().expect("values")))
        } else {
            ColumnData::I32(Shared::new(direct.read_raw::<i32>().expect("values")))
        };
        assert_eq!(first[at], want, "{name}");
    }
    // a range inside the block is a slice of it: warm, it costs a lookup
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

#[test]
fn the_first_read_of_the_large_v16_file_is_timed() {
    let path = large_fixture_or_skip!("Build Log Fixtures/V16_trimmed_fixture.h5");
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let start = std::time::Instant::now();
    let opened = open(production(), &path).expect("the trimmed V16 file opens");
    let open_time = start.elapsed();
    let source = opened.source();
    let build = source.build().expect("the metadata");
    assert_eq!(build.layers_present(), 15);
    let columns = recorded_columns(&build);
    let layers = build.layers_present();
    eprintln!(
        "the trimmed V16 file ({size} bytes, {layers} layers, {} recorded columns): open {open_time:?}",
        columns.len()
    );
    // the first read of one layer's channel, and the second, of the first layer's power and of the last layer's
    let kept = ReadContext::new().with_cache_budget(256 << 20);
    let first_layer = columns[0].2;
    let last_layer = columns.iter().map(|c| c.2).max().expect("a layer");
    for layer in [first_layer, last_layer] {
        let (laser, channel, _, samples) = columns
            .iter()
            .find(|(_, c, l, _)| *l == layer && *c == channel(layout::LASER_POWER))
            .cloned()
            .expect("the power of the layer");
        let asked = request(&laser, &channel, layer);
        let start = std::time::Instant::now();
        let cold = source.read(&asked, &kept).expect("the first read");
        let first = start.elapsed();
        let start = std::time::Instant::now();
        let warm = source.read(&asked, &kept).expect("the second read");
        let second = start.elapsed();
        eprintln!(
            "layer {} of the trimmed V16 file, the power ({samples} values): first read {first:?}, second read {second:?}",
            layer.0
        );
        assert_eq!(cold.len() as u64, samples);
        assert_eq!(warm, cold);
    }
    let stats = source.cache_stats().expect("stats");
    assert!(stats.hits >= 2, "{stats:?}");
}
