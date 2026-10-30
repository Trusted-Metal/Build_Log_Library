//! The column engine (IP-06 phase 6B, task T06.2; decisions D06-3, D06-4, D06-9 and D06-17), proved before any adapter uses it.
//!
//! The engine asks a locator for the dataset of a request. Here the locator is a test one over the eight miniatures (the channel of a request names a dataset by its path in
//! the file), so nothing of any version's layout is in the engine or in its tests.
//!
//! * `a_ranged_read_equals_a_slice_of_the_full_read`: every one-dimensional numeric dataset of every miniature, read by seeded ranges (on and across block boundaries, at
//!   every precision), equals the answer of an `InMemorySource` that holds the same values, and the slice of the full read.
//! * The rest are the contracts one by one: the stored type decides the variant, precision is converted block by block, an empty range is an empty column, a range outside the
//!   column is `InvalidIndex` and says the stored count, a time range is unsupported and says why, a cancelled read returns no partial column, progress ends at the total, and
//!   every error says which column it was about.

mod common;

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use build_log_io::columns::read_column;
use build_log_io::hdf5::{block_rows, Region};
use build_log_model::{
    CancelToken, ChannelKey, ColumnData, ColumnRange, ColumnRequest, ColumnSource, Error,
    ErrorKind, InMemorySource, LaserKey, LayerId, Precision, ReadContext, Shared, TimeValue,
};
use build_log_testkit::miniature_path;
use common::{datasets, locator_over, pick_rows, put, FileLocator, Rng, MINIATURES};
use hdf5::types::{FloatSize, IntSize, TypeDescriptor};
use hdf5::{Dataset, File};

// ---------------------------------------------------------------- a test locator, and requests for it

const VERSION: &str = common::LOCATOR_VERSION;
const LAYER: LayerId = LayerId(3);

fn laser() -> LaserKey {
    LaserKey::new("Laser_2", 2)
}

fn key(path: &str) -> ChannelKey {
    ChannelKey::new([path])
}

fn request(path: &str) -> ColumnRequest {
    ColumnRequest::new(LAYER, Some(laser()), key(path))
}

fn indices(path: &str, range: Range<u64>) -> ColumnRequest {
    request(path).with_range(ColumnRange::Indices(range))
}

/// The whole of a one-dimensional numeric dataset, read by the binding itself, as the Model holds it; `None` for any other dataset.
fn reference(dataset: &Dataset) -> Option<ColumnData> {
    if dataset.ndim() != 1 {
        return None;
    }
    match dataset.dtype().and_then(|d| d.to_descriptor()) {
        Ok(TypeDescriptor::Integer(IntSize::U2)) => {
            Some(ColumnData::I16(Shared::new(dataset.read_raw().ok()?)))
        }
        Ok(TypeDescriptor::Integer(IntSize::U4)) => {
            Some(ColumnData::I32(Shared::new(dataset.read_raw().ok()?)))
        }
        Ok(TypeDescriptor::Unsigned(IntSize::U8)) => {
            Some(ColumnData::U64(Shared::new(dataset.read_raw().ok()?)))
        }
        Ok(TypeDescriptor::Float(FloatSize::U4)) => {
            Some(ColumnData::F32(Shared::new(dataset.read_raw().ok()?)))
        }
        Ok(TypeDescriptor::Float(FloatSize::U8)) => {
            Some(ColumnData::F64(Shared::new(dataset.read_raw().ok()?)))
        }
        Ok(TypeDescriptor::Unsigned(IntSize::U1)) => {
            Some(ColumnData::U8(Shared::new(dataset.read_raw().ok()?)))
        }
        _ => None,
    }
}

fn variant(data: &ColumnData) -> &'static str {
    match data {
        ColumnData::I16(_) => "I16",
        ColumnData::I32(_) => "I32",
        ColumnData::U64(_) => "U64",
        ColumnData::F32(_) => "F32",
        ColumnData::F64(_) => "F64",
        ColumnData::U8(_) => "U8",
        _ => "another",
    }
}

fn kind_of(outcome: Result<ColumnData, Error>) -> Error {
    outcome.expect_err("an error")
}

// ---------------------------------------------------------------- the property test over the miniatures

#[test]
fn a_ranged_read_equals_a_slice_of_the_full_read() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let context = ReadContext::new();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for name in MINIATURES {
        let locator =
            FileLocator::new(File::open(miniature_path(name)).expect("a miniature opens"));
        for (path, dataset) in datasets(&locator.file) {
            let Some(full) = reference(&dataset) else {
                continue;
            };
            let label = format!("{name} {path}");
            *seen.entry(variant(&full)).or_default() += 1;
            // the reference answers: an in-memory source that holds the same values
            let mut memory = InMemorySource::new();
            memory.insert(LAYER, Some(laser()), key(&path), full.clone());
            let size = block_rows(&dataset, &Region::whole()).expect("the block size");

            // the whole column, as stored
            let whole = read_column(&request(&path), &locator, &context)
                .unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(whole, full, "{label}: the whole column");
            assert_eq!(variant(&whole), variant(&full), "{label}");

            for _ in 0..8 {
                let rows = pick_rows(&mut rng, full.len(), size);
                let precision = [Precision::Native, Precision::F32, Precision::F64][rng.up_to(2)];
                let asked =
                    indices(&path, rows.start as u64..rows.end as u64).with_precision(precision);
                let got = read_column(&asked, &locator, &context)
                    .unwrap_or_else(|e| panic!("{label}: rows {rows:?} at {precision:?}: {e}"));
                let want = memory
                    .read(&asked, &context)
                    .expect("the reference reads it");
                assert_eq!(
                    got, want,
                    "{label}: rows {rows:?} at {precision:?} differ from the in-memory source"
                );
                let slice = full
                    .slice(rows.clone())
                    .expect("a slice")
                    .with_precision(precision)
                    .expect("numbers");
                assert_eq!(got, slice, "{label}: rows {rows:?} at {precision:?} differ from the slice of the full read");
            }
        }
    }
    for wanted in ["I32", "U64", "F32", "F64", "U8"] {
        assert!(
            seen.contains_key(wanted),
            "no {wanted} column was checked: {seen:?}"
        );
    }
}

#[test]
fn the_stored_type_decides_the_variant() {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for name in MINIATURES {
        let locator =
            FileLocator::new(File::open(miniature_path(name)).expect("a miniature opens"));
        for (path, dataset) in datasets(&locator.file) {
            let Some(full) = reference(&dataset) else {
                continue;
            };
            let got =
                read_column(&request(&path), &locator, &ReadContext::new()).expect("it reads");
            // what is stored, and never what a precision or the metadata would say
            let stored = format!(
                "{:?}",
                dataset
                    .dtype()
                    .and_then(|d| d.to_descriptor())
                    .expect("a type")
            );
            seen.insert(stored, variant(&got));
            assert_eq!(variant(&got), variant(&full), "{name} {path}");
        }
    }
    assert_eq!(
        seen.values()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        ["F32", "F64", "I32", "U64", "U8"].into_iter().collect(),
        "{seen:?}"
    );
}

// ---------------------------------------------------------------- datasets made at test time

#[test]
fn precision_converts_block_by_block_and_equals_the_whole_column_conversion() {
    // more than two blocks of 10,000, with values that `f32` cannot hold exactly, so that a conversion that differs from the Model's shows
    let ints: Vec<i32> = (0..25_000).map(|i| i32::MAX - 3 * i).collect();
    let bigs: Vec<u64> = (0..25_000u64).map(|i| u64::MAX - 7 * i).collect();
    let doubles: Vec<f64> = (0..25_000).map(|i| 0.1 * f64::from(i) + 1.0e-9).collect();
    let singles: Vec<f32> = (0..25_000).map(|i| 0.1 * i as f32).collect();
    let bytes: Vec<u8> = (0..25_000).map(|i| (i % 251) as u8).collect();
    let (_file, locator) = locator_over("engine-precision", |g| {
        put(g, "ints", &ints, Some(10_000));
        put(g, "bigs", &bigs, Some(10_000));
        put(g, "doubles", &doubles, Some(10_000));
        put(g, "singles", &singles, Some(10_000));
        put(g, "bytes", &bytes, Some(10_000));
    });
    let context = ReadContext::new();
    for name in ["ints", "bigs", "doubles", "singles", "bytes"] {
        let path = format!("Mount/{name}");
        let native = read_column(&request(&path), &locator, &context).expect("native");
        for precision in [Precision::F32, Precision::F64] {
            let got = read_column(
                &request(&path).with_precision(precision),
                &locator,
                &context,
            )
            .expect("converted");
            let want = native.clone().with_precision(precision).expect("numbers");
            assert_eq!(
                got, want,
                "{name} at {precision:?}: block by block is not the whole column converted"
            );
            // and a range that crosses two block boundaries
            let part = read_column(
                &indices(&path, 9_990..20_010).with_precision(precision),
                &locator,
                &context,
            )
            .expect("a converted range");
            assert_eq!(
                part,
                want.slice(9_990..20_010).expect("a slice"),
                "{name} at {precision:?}"
            );
        }
    }
}

#[test]
fn an_empty_range_is_an_empty_column() {
    let (_file, locator) = locator_over("engine-empty", |g| {
        put(
            g,
            "x",
            &(0..100).map(|i| i as f32).collect::<Vec<_>>(),
            None,
        );
        put::<i32>(g, "none", &[], None);
    });
    let context = ReadContext::new();
    // a..a, anywhere inside the column and at its end, is an empty column of the stored type
    for at in [0u64, 7, 100] {
        let got =
            read_column(&indices("Mount/x", at..at), &locator, &context).expect("an empty column");
        assert_eq!(got, ColumnData::F32(Shared::new(Vec::new())), "at {at}");
    }
    // a column of no value is an empty column of its type, for the whole of it and for no precision change
    let got = read_column(&request("Mount/none"), &locator, &context).expect("an empty column");
    assert_eq!(got, ColumnData::I32(Shared::new(Vec::new())));
    let got = read_column(
        &request("Mount/none").with_precision(Precision::F64),
        &locator,
        &context,
    )
    .expect("an empty column");
    assert_eq!(got, ColumnData::F64(Shared::new(Vec::new())));
}

#[test]
fn a_null_dataspace_channel_reads_as_an_empty_column_of_the_stored_type() {
    // IP-07 phase 7A, D07-2: a dataset with a null dataspace has a type, no dimension and no value. It reads as an empty column of its stored type (not as damage, which a table is), at any
    // precision, and a range is checked against its length, which is zero.
    let (_file, locator) = locator_over("engine-null", |g| {
        make_null::<i16>(g, "i16");
        make_null::<i32>(g, "i32");
        make_null::<u64>(g, "u64");
        make_null::<f32>(g, "f32");
        make_null::<f64>(g, "f64");
        make_null::<u8>(g, "u8");
        // a type that a column still does not have is refused for that, null or not
        make_null::<u16>(g, "u16");
        // a scalar has no dimension either, and it is not null: it is a dataset that is not a column
        g.new_dataset::<f32>().create("scalar").expect("a scalar");
    });
    let context = ReadContext::new();
    for (name, native) in [
        ("i16", ColumnData::I16(Shared::new(Vec::new()))),
        ("i32", ColumnData::I32(Shared::new(Vec::new()))),
        ("u64", ColumnData::U64(Shared::new(Vec::new()))),
        ("f32", ColumnData::F32(Shared::new(Vec::new()))),
        ("f64", ColumnData::F64(Shared::new(Vec::new()))),
        ("u8", ColumnData::U8(Shared::new(Vec::new()))),
    ] {
        let path = format!("Mount/{name}");
        let got = read_column(&request(&path), &locator, &context)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, native, "{name}: an empty column of the stored type");
        // at a precision, it is an empty column of that precision, as an empty column converts
        for precision in [Precision::F32, Precision::F64] {
            let converted = read_column(
                &request(&path).with_precision(precision),
                &locator,
                &context,
            )
            .expect("an empty column");
            assert_eq!(
                converted,
                native.clone().with_precision(precision).expect("numbers"),
                "{name} at {precision:?}"
            );
        }
        // the empty range is the empty column, and a range past its length is outside it and says the length
        assert_eq!(
            read_column(&indices(&path, 0..0), &locator, &context).expect("0..0"),
            native,
            "{name}"
        );
        match kind_of(read_column(&indices(&path, 0..1), &locator, &context)).kind() {
            ErrorKind::InvalidIndex { what } => {
                assert!(what.contains("0 values"), "{name}: {what}")
            }
            other => panic!("{name}: expected InvalidIndex, got {other:?}"),
        }
    }
    // a type that is not read is refused, null or not; a scalar is a dataset of no dimension that is not null, and is not a column
    match kind_of(read_column(&request("Mount/u16"), &locator, &context)).kind() {
        ErrorKind::UnsupportedInVersion { feature, .. } => {
            assert!(feature.contains("column stored as"), "{feature}")
        }
        other => panic!("expected UnsupportedInVersion, got {other:?}"),
    }
    match kind_of(read_column(&request("Mount/scalar"), &locator, &context)).kind() {
        ErrorKind::Corrupt { detail } => assert!(detail.contains("0 dimensions"), "{detail}"),
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

fn make_null<T: hdf5::H5Type>(group: &hdf5::Group, name: &str) {
    group
        .new_dataset_builder()
        .empty::<T>()
        .shape(hdf5::Extents::Null)
        .create(name)
        .expect("a dataset with a null dataspace");
}

#[test]
#[allow(clippy::reversed_empty_ranges)]
fn a_range_outside_the_column_is_invalid_index_and_says_the_stored_count() {
    let (_file, locator) = locator_over("engine-outside", |g| {
        put(g, "x", &vec![1.5f32; 25_000], Some(10_000))
    });
    let context = ReadContext::new();
    for (what, range) in [
        ("one value too many", 0..25_001u64),
        ("far past the end", 30_000..30_010),
        ("start after end", 10..5),
        ("a range that does not fit a usize", 0..u64::MAX),
    ] {
        let error = kind_of(read_column(&indices("Mount/x", range), &locator, &context));
        match error.kind() {
            ErrorKind::InvalidIndex { what: said } => {
                assert!(
                    said.contains("25000"),
                    "{what}: the stored count is not said: {said}"
                )
            }
            other => panic!("{what}: expected InvalidIndex, got {other:?}"),
        }
    }
    // the last value is in, and one more is not
    assert!(read_column(&indices("Mount/x", 24_999..25_000), &locator, &context).is_ok());
}

#[test]
fn a_time_range_is_unsupported_and_says_why() {
    let (_file, locator) = locator_over("engine-time", |g| put(g, "x", &[1i32, 2, 3], None));
    let time = request("Mount/x").with_range(ColumnRange::Time {
        from: TimeValue::from_file(0, None),
        to: TimeValue::from_file(10, None),
    });
    let error = kind_of(read_column(&time, &locator, &ReadContext::new()));
    match error.kind() {
        ErrorKind::UnsupportedInVersion { version, feature } => {
            assert_eq!(version, VERSION, "the locator's version is the one named");
            assert!(
                feature.contains("time range") && feature.contains("segment table"),
                "{feature}"
            );
        }
        other => panic!("expected UnsupportedInVersion, got {other:?}"),
    }
}

#[test]
fn a_cancelled_read_returns_cancelled_and_no_partial_column() {
    let (_file, locator) = locator_over("engine-cancel", |g| {
        put(
            g,
            "x",
            &(0..25_000).map(|i| i as f32).collect::<Vec<_>>(),
            Some(10_000),
        )
    });
    // cancelled by the first progress report: the first block was read, and nothing of it is returned, as stored and converted
    for precision in [Precision::Native, Precision::F64] {
        let token = CancelToken::new();
        let seen = Arc::new(AtomicUsize::new(0));
        let (cancel, count) = (token.clone(), Arc::clone(&seen));
        let context = ReadContext::new()
            .with_cancel(token)
            .with_progress(Arc::new(move |_done, _total| {
                count.fetch_add(1, Ordering::SeqCst);
                cancel.cancel();
            }));
        let outcome = read_column(
            &request("Mount/x").with_precision(precision),
            &locator,
            &context,
        );
        assert!(
            matches!(outcome.expect_err("cancelled").kind(), ErrorKind::Cancelled),
            "{precision:?}"
        );
        assert_eq!(
            seen.load(Ordering::SeqCst),
            1,
            "no block is read after the token fires ({precision:?})"
        );
    }
    // cancelled before the read: the locator is not even asked
    let token = CancelToken::new();
    token.cancel();
    let asked = locator.asked();
    let outcome = read_column(
        &request("Mount/x"),
        &locator,
        &ReadContext::new().with_cancel(token),
    );
    assert!(matches!(
        outcome.expect_err("cancelled").kind(),
        ErrorKind::Cancelled
    ));
    assert_eq!(locator.asked(), asked, "a cancelled read asks for nothing");
}

#[test]
fn progress_ends_at_the_total_values() {
    let (_file, locator) = locator_over("engine-progress", |g| {
        put(
            g,
            "x",
            &(0..25_000).map(|i| i as f32).collect::<Vec<_>>(),
            Some(10_000),
        )
    });
    let seen: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let context = ReadContext::new().with_progress(Arc::new(move |done, total| {
        sink.lock().expect("a lock").push((done, total));
    }));
    for (what, asked, expected) in [
        (
            "the whole column",
            request("Mount/x"),
            vec![(10_000, 25_000), (20_000, 25_000), (25_000, 25_000)],
        ),
        (
            "a range that starts in the middle of a block",
            indices("Mount/x", 3_000..22_000),
            vec![(7_000, 19_000), (17_000, 19_000), (19_000, 19_000)],
        ),
        (
            "the same at another precision",
            indices("Mount/x", 3_000..22_000).with_precision(Precision::F64),
            vec![(7_000, 19_000), (17_000, 19_000), (19_000, 19_000)],
        ),
        (
            "an empty range reports nothing",
            indices("Mount/x", 5..5),
            vec![],
        ),
    ] {
        seen.lock().expect("a lock").clear();
        read_column(&asked, &locator, &context).expect("a read");
        let reported = seen.lock().expect("a lock").clone();
        assert_eq!(reported, expected, "{what}");
        assert!(
            reported.windows(2).all(|w| w[0].0 < w[1].0),
            "{what}: done only grows"
        );
    }
}

// ---------------------------------------------------------------- errors say which column they are about

#[test]
fn every_error_names_the_layer_the_laser_the_channel_and_the_operation() {
    let (_file, locator) = locator_over("engine-location", |g| {
        put(g, "x", &[1.0f32, 2.0, 3.0], None);
        put(g, "narrow", &[1u16, 2, 3], None);
        let table = g
            .new_dataset::<f32>()
            .shape([2, 2])
            .create("table")
            .expect("a dataset");
        table.write_raw(&[0.0f32; 4]).expect("its values");
    });
    let context = ReadContext::new();
    let asked = [
        ("a column the file does not have", request("Mount/missing")),
        ("a range outside", indices("Mount/x", 0..9)),
        (
            "a time range",
            request("Mount/x").with_range(ColumnRange::Time {
                from: TimeValue::from_file(0, None),
                to: TimeValue::from_file(1, None),
            }),
        ),
        ("a type that is not a column", request("Mount/narrow")),
        ("a dataset of two dimensions", request("Mount/table")),
    ];
    for (what, asked) in asked {
        let error = kind_of(read_column(&asked, &locator, &context));
        let location = error.location();
        assert_eq!(location.layer, Some(LAYER), "{what}: {error}");
        assert_eq!(location.laser, Some(laser()), "{what}: {error}");
        assert_eq!(
            location.channel.as_ref(),
            Some(&asked.channel),
            "{what}: {error}"
        );
        assert_eq!(
            location.operation.as_deref(),
            Some("read"),
            "{what}: {error}"
        );
    }
}

#[test]
fn what_is_not_a_column_is_refused_for_what_it_is() {
    let (_file, locator) = locator_over("engine-refusals", |g| {
        put(g, "narrow", &[1u16, 2, 3], None);
        let table = g
            .new_dataset::<f32>()
            .shape([2, 2])
            .create("table")
            .expect("a dataset");
        table.write_raw(&[0.0f32; 4]).expect("its values");
    });
    let context = ReadContext::new();
    // a type that a column does not have, named with the version
    match kind_of(read_column(&request("Mount/narrow"), &locator, &context)).kind() {
        ErrorKind::UnsupportedInVersion { version, feature } => {
            assert_eq!(version, VERSION);
            assert!(
                feature.contains("column stored as") && feature.contains("U2"),
                "{feature}"
            );
            // and it says what a column is, from the one list of what the engine reads
            assert!(
                feature.contains("`int16`") && feature.contains("`uint8`"),
                "{feature}"
            );
        }
        other => panic!("expected UnsupportedInVersion, got {other:?}"),
    }
    // a dataset that is not one-dimensional is not a column of a laser: the file is at fault
    match kind_of(read_column(&request("Mount/table"), &locator, &context)).kind() {
        ErrorKind::Corrupt { detail } => assert!(detail.contains("2 dimensions"), "{detail}"),
        other => panic!("expected Corrupt, got {other:?}"),
    }
    // a column the locator does not have is its NotPresent, passed on
    assert!(matches!(
        kind_of(read_column(&request("Mount/missing"), &locator, &context)).kind(),
        ErrorKind::NotPresent { .. }
    ));
}
