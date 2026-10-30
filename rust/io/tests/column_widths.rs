//! Other sample widths (IP-06 phase 6I, task T06.9; decision D07-3): the column engine reads a stored 16-bit integer as stored, and the one list of what it reads is what it reads.
//!
//! tauri's XY2-100E writer stores its commanded positions as `int16`. The V17 adapter described them (`Numeric(I16)`), and a read answered `UnsupportedInVersion`, because the engine
//! read five types and `int16` was not one of them. Nothing is converted: a column read as stored is `I16`, and a precision is applied by the Model's own conversions.
//!
//! * `an_int16_dataset_reads_as_stored`, `an_int16_range_equals_a_slice_of_the_full_read`, `an_int16_column_converts_to_f32_and_f64_as_the_model_does`: the three ways a column is read,
//!   on datasets made at test time (chunks of 10,000, one chunk, and no chunking, the three shapes of the files we hold), with both ends of the type's range.
//! * `an_int16_block_is_cached_by_its_size_and_served_warm`: the cache holds a block of 16-bit values as two bytes a value.
//! * `the_readable_list_equals_what_the_engine_reads`: a dataset of every number type that the Model names, made at test time, is read, and **what is read is exactly what `can_read`
//!   says**. It is the test that keeps a report of availability (IP-07 phase 7D) from saying one thing while a read does another.
//! * `the_xy2_100_fixture_commanded_positions_read`: the owner's fixture of 2026-08-12 (local; skipped when absent), whose three commanded channels are `int16`, equals what the
//!   independent script computed from the file.

mod common;

use std::ops::Range;

use build_log_conformance::Fingerprint;
use build_log_io::adapters::v17::layout;
use build_log_io::columns::{can_read, read_column, ColumnReader};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    ChannelKey, ColumnData, ColumnRange, ColumnRequest, ErrorKind, LaserKey, LayerId, NumericType,
    Precision, ReadContext, Shared,
};
use build_log_testkit::skip_message;
use common::{locator_over, pick_rows, put, temp_h5, FileLocator, Rng};

const LAYER: LayerId = LayerId(1);

fn laser() -> LaserKey {
    LaserKey::new("Laser_1", 1)
}

fn request(path: &str) -> ColumnRequest {
    ColumnRequest::new(LAYER, Some(laser()), ChannelKey::new([path]))
}

fn indices(path: &str, range: Range<u64>) -> ColumnRequest {
    request(path).with_range(ColumnRange::Indices(range))
}

/// 25,000 values of the type, both ends of its range first, then values that spread over it, so that a value read as another width shows.
fn values() -> Vec<i16> {
    let mut values = vec![i16::MIN, -1, 0, 1, i16::MAX, i16::MIN + 1, i16::MAX - 1];
    values.extend((7..25_000i64).map(|i| ((i * 7_919) % 65_536 - 32_768) as i16));
    values
}

/// The three ways the files we hold are stored: chunks of 10,000, one chunk for all the values, and no chunking.
const SHAPES: [(&str, Option<usize>); 3] = [
    ("chunks", Some(10_000)),
    ("one_chunk", Some(25_000)),
    ("plain", None),
];

// ---------------------------------------------------------------- reading as stored

#[test]
fn an_int16_dataset_reads_as_stored() {
    let stored = values();
    let (_file, locator) = locator_over("widths-stored", |g| {
        for (name, chunk) in SHAPES {
            put(g, name, &stored, chunk);
        }
        put::<i16>(g, "none", &[], None);
    });
    let context = ReadContext::new();
    for (name, _) in SHAPES {
        let got = read_column(&request(&format!("Mount/{name}")), &locator, &context)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        // the variant is the stored type, and the values are the values: nothing is widened, narrowed or rescaled
        assert_eq!(got, ColumnData::I16(Shared::new(stored.clone())), "{name}");
    }
    // a column of no value is an empty column of the stored type, with or without a precision
    let none = read_column(&request("Mount/none"), &locator, &context).expect("an empty column");
    assert_eq!(none, ColumnData::I16(Shared::new(Vec::new())));
    let none = read_column(
        &request("Mount/none").with_precision(Precision::F32),
        &locator,
        &context,
    )
    .expect("an empty column");
    assert_eq!(none, ColumnData::F32(Shared::new(Vec::new())));
}

#[test]
fn an_int16_range_equals_a_slice_of_the_full_read() {
    let stored = values();
    let full = ColumnData::I16(Shared::new(stored.clone()));
    let (_file, locator) = locator_over("widths-ranges", |g| {
        for (name, chunk) in SHAPES {
            put(g, name, &stored, chunk);
        }
    });
    let context = ReadContext::new();
    let mut rng = Rng(0x16B1_75A5_0F16_0001);
    for (name, chunk) in SHAPES {
        let size = chunk.unwrap_or(10_000);
        for _ in 0..100 {
            let range = pick_rows(&mut rng, stored.len(), size);
            for precision in [Precision::Native, Precision::F32, Precision::F64] {
                let got = read_column(
                    &indices(
                        &format!("Mount/{name}"),
                        range.start as u64..range.end as u64,
                    )
                    .with_precision(precision),
                    &locator,
                    &context,
                )
                .unwrap_or_else(|e| panic!("{name} {range:?} {precision:?}: {e}"));
                let want = full
                    .slice(range.clone())
                    .expect("a slice")
                    .with_precision(precision)
                    .expect("numbers");
                assert_eq!(got, want, "{name} {range:?} {precision:?}");
            }
        }
    }
}

#[test]
fn an_int16_column_converts_to_f32_and_f64_as_the_model_does() {
    let stored = values();
    let native = ColumnData::I16(Shared::new(stored.clone()));
    let (_file, locator) = locator_over("widths-precision", |g| {
        put(g, "x", &stored, Some(10_000));
    });
    let context = ReadContext::new();
    for precision in [Precision::F32, Precision::F64] {
        let got = read_column(
            &request("Mount/x").with_precision(precision),
            &locator,
            &context,
        )
        .expect("converted");
        // every 16-bit integer is exact in both, so the value is the same number
        match (&got, precision) {
            (ColumnData::F32(v), Precision::F32) => {
                assert!(v
                    .as_slice()
                    .iter()
                    .zip(&stored)
                    .all(|(&f, &i)| f == f32::from(i)))
            }
            (ColumnData::F64(v), Precision::F64) => {
                assert!(v
                    .as_slice()
                    .iter()
                    .zip(&stored)
                    .all(|(&f, &i)| f == f64::from(i)))
            }
            other => panic!("{other:?}"),
        }
        // block by block is the whole column converted by the Model, and so is a range that crosses two block boundaries
        let want = native.clone().with_precision(precision).expect("numbers");
        assert_eq!(got, want, "{precision:?}");
        let part = read_column(
            &indices("Mount/x", 9_990..20_010).with_precision(precision),
            &locator,
            &context,
        )
        .expect("a converted range");
        assert_eq!(
            part,
            want.slice(9_990..20_010).expect("a slice"),
            "{precision:?}"
        );
    }
}

#[test]
fn an_int16_block_is_cached_by_its_size_and_served_warm() {
    let stored = values();
    let on_disk = temp_h5("widths-cache", |g| put(g, "x", &stored, Some(10_000)));
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let reader = ColumnReader::new(FileLocator::new(file));
    let context = ReadContext::new().with_cache_budget(1 << 20);
    let cold = reader
        .read(&request("Mount/x"), &context)
        .expect("a cold read");
    let after_cold = reader.stats();
    // three blocks of the column, each decoded once and held as two bytes a value
    assert_eq!(
        (after_cold.hits, after_cold.misses),
        (0, 3),
        "{after_cold:?}"
    );
    assert_eq!(
        after_cold.bytes_held,
        2 * stored.len() as u64,
        "{after_cold:?}"
    );
    let warm = reader
        .read(&request("Mount/x"), &context)
        .expect("a warm read");
    let after_warm = reader.stats();
    assert_eq!(cold, warm);
    assert_eq!(cold, ColumnData::I16(Shared::new(stored)));
    assert_eq!(
        (after_warm.hits, after_warm.misses, after_warm.bytes_held),
        (3, 3, after_cold.bytes_held),
        "{after_warm:?}"
    );
}

// ---------------------------------------------------------------- the one list

/// One dataset of each number type the Model names, made at test time: three values each.
fn every_type_in_one_file(label: &str) -> (common::TempH5, FileLocator) {
    locator_over(label, |g| {
        put(g, "I8", &[-1i8, 2, 3], None);
        put(g, "I16", &[-1i16, 2, 3], None);
        put(g, "I32", &[-1i32, 2, 3], None);
        put(g, "I64", &[-1i64, 2, 3], None);
        put(g, "U8", &[1u8, 2, 3], None);
        put(g, "U16", &[1u16, 2, 3], None);
        put(g, "U32", &[1u32, 2, 3], None);
        put(g, "U64", &[1u64, 2, 3], None);
        put(g, "F32", &[1.5f32, 2.0, 3.0], None);
        put(g, "F64", &[1.5f64, 2.0, 3.0], None);
    })
}

/// The Model's name for what a column holds.
fn named(data: &ColumnData) -> &'static str {
    match data {
        ColumnData::I8(_) => "I8",
        ColumnData::I16(_) => "I16",
        ColumnData::I32(_) => "I32",
        ColumnData::I64(_) => "I64",
        ColumnData::U8(_) => "U8",
        ColumnData::U16(_) => "U16",
        ColumnData::U32(_) => "U32",
        ColumnData::U64(_) => "U64",
        ColumnData::F32(_) => "F32",
        ColumnData::F64(_) => "F64",
        _ => "another",
    }
}

#[test]
fn the_readable_list_equals_what_the_engine_reads() {
    let (_file, locator) = every_type_in_one_file("widths-list");
    let context = ReadContext::new();
    let types = [
        ("I8", NumericType::I8),
        ("I16", NumericType::I16),
        ("I32", NumericType::I32),
        ("I64", NumericType::I64),
        ("U8", NumericType::U8),
        ("U16", NumericType::U16),
        ("U32", NumericType::U32),
        ("U64", NumericType::U64),
        ("F32", NumericType::F32),
        ("F64", NumericType::F64),
    ];
    let mut readable = Vec::new();
    for (name, dtype) in types {
        match read_column(&request(&format!("Mount/{name}")), &locator, &context) {
            Ok(data) => {
                // what is read is typed by what is stored, and `can_read` says so
                assert_eq!(named(&data), name, "{name}");
                assert!(
                    can_read(dtype),
                    "{name} was read and `can_read` says it is not"
                );
                readable.push(name);
            }
            Err(error) => {
                assert!(
                    matches!(error.kind(), ErrorKind::UnsupportedInVersion { .. }),
                    "{name}: {error}"
                );
                assert!(
                    !can_read(dtype),
                    "{name} was refused and `can_read` says it is read"
                );
            }
        }
    }
    // the list as it stands, written out: a type that is added or dropped is a decision, and this test is where it is seen
    assert_eq!(
        readable,
        ["I16", "I32", "U8", "U64", "F32", "F64"],
        "what the engine reads"
    );
}

// ---------------------------------------------------------------- the owner's fixture of 2026-08-12 (local)

/// The independent script's fingerprint of a commanded channel of the file (`tools/fixtures`, `expectations.fingerprint`, from `h5py`): the count, the extremes, the first and the last
/// value (each run of three is the same value), and the SHA-256 of the little-endian bytes.
struct Independent {
    channel: &'static str,
    min: f64,
    max: f64,
    first: f64,
    last: f64,
    sha256: &'static str,
}

const COMMANDED: [Independent; 3] = [
    Independent {
        channel: layout::COMMANDED_X,
        min: -18_511.0,
        max: 16_963.0,
        first: -18_511.0,
        last: 13_289.0,
        sha256: "1468da51bc9d58f23596075ae86789321a707fe1801986de4b8302e90535f003",
    },
    Independent {
        channel: layout::COMMANDED_Y,
        min: -2_199.0,
        max: 17_300.0,
        first: 12_441.0,
        last: 16_664.0,
        sha256: "6b76e2acf850f2f6978048813ad3dbac5173c395bc9b352e091183a253cddb76",
    },
    Independent {
        channel: layout::COMMANDED_Z,
        min: -32_765.0,
        max: -32_012.0,
        first: -32_061.0,
        last: -32_093.0,
        sha256: "282dc3a34cd41db2337c652435a85a03e7687a2ddf2d6afc0ced50d47dfa211b",
    },
];

#[test]
fn the_xy2_100_fixture_commanded_positions_read() {
    // the owner's file, kept under `tools/edge_cases/files/` (git-excluded): the test is skipped where it is not
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/edge_cases/files/BuildData_fixture.h5");
    if !path.exists() {
        eprintln!(
            "{}",
            skip_message(&format!(
                "the owner's XY2-100 fixture is not at {}",
                path.display()
            ))
        );
        return;
    }
    let opened = open(production(), &path).expect("the fixture opens");
    let source = opened.source();
    let context = ReadContext::new();
    for expected in &COMMANDED {
        let got = source
            .read(&request(expected.channel), &context)
            .unwrap_or_else(|e| panic!("{}: {e}", expected.channel));
        // stored as 16-bit integers, and returned as such
        assert!(
            matches!(got, ColumnData::I16(_)),
            "{}: {}",
            expected.channel,
            named(&got)
        );
        let fingerprint = Fingerprint::of(&got).expect("numbers");
        assert_eq!(fingerprint.count, 141_624, "{}", expected.channel);
        assert_eq!(fingerprint.stored_as, "I16", "{}", expected.channel);
        assert_eq!(
            (fingerprint.min, fingerprint.max),
            (Some(expected.min), Some(expected.max)),
            "{}",
            expected.channel
        );
        assert_eq!(
            (fingerprint.first[0], fingerprint.last[2]),
            (expected.first, expected.last),
            "{}",
            expected.channel
        );
        assert_eq!(fingerprint.sha256, expected.sha256, "{}", expected.channel);
    }
}
