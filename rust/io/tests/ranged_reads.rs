//! The ranged block reader (IP-03 phase 3F, task T03.6, decision D03-12), held to the eight miniatures and to the oracle `testdata/primitives.json`.
//!
//! * `ranged_reads_equal_slices_of_the_full_read`: every numeric dataset of every miniature, of the five types and one to three dimensions, is read whole and by
//!   seeded ranges (across and on block boundaries, and as rectangles), and each read equals the slice of the binding's own full read.
//! * `the_first_values_equal_the_oracle`: the first values of every numeric kind of the oracle.
//! * `cancel_returns_cancelled_at_a_block_boundary`, `progress_is_reported_monotonically` and `a_range_outside_the_dataset_is_invalid_index`: the three
//!   contracts of `ReadContext` and of a range, on datasets made at test time so that the blocks are known.
//! * The block rule of D03-12 (`blocks_are_a_multiple_of_the_chunk_and_aligned`), and the refusals: a type that is not the stored one is not converted, and a
//!   dataset of no dimension or of four is not read.

mod common;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use build_log_io::hdf5::{block_rows, read_blocks, region_shape, Block, Element, Region};
use build_log_model::{CancelToken, Error, ErrorKind, ReadContext};
use build_log_testkit::miniature_path;
use common::{datasets, oracle, pick_any, pick_rows, temp_h5, Rng, MINIATURES};
use hdf5::types::{FloatSize, IntSize, TypeDescriptor};
use hdf5::{Dataset, File};
use serde_json::Value as Json;

// ---------------------------------------------------------------- small helpers

/// An element that can be compared bit for bit, so that a `NaN` equals itself.
trait Bits: Element + Debug {
    fn bits(&self) -> u64;
}
impl Bits for i32 {
    fn bits(&self) -> u64 {
        u64::from(*self as u32)
    }
}
impl Bits for u64 {
    fn bits(&self) -> u64 {
        *self
    }
}
impl Bits for f32 {
    fn bits(&self) -> u64 {
        u64::from(self.to_bits())
    }
}
impl Bits for f64 {
    fn bits(&self) -> u64 {
        self.to_bits()
    }
}
impl Bits for u8 {
    fn bits(&self) -> u64 {
        u64::from(*self)
    }
}

fn same<T: Bits>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.bits() == y.bits())
}

fn blocks_of<T: Element>(dataset: &Dataset, region: &Region) -> Result<Vec<Block<T>>, Error> {
    let mut blocks = Vec::new();
    read_blocks::<T>(dataset, region, &ReadContext::new(), |block| {
        blocks.push(block);
        Ok(())
    })?;
    Ok(blocks)
}

fn flatten<T: Element>(blocks: &[Block<T>]) -> Vec<T> {
    blocks
        .iter()
        .flat_map(|b| b.values.iter().copied())
        .collect()
}

/// The blocks cover `rows` without a gap, the first starts where the read starts and each later one on a multiple of the block size, and none is larger than
/// the block size.
fn assert_aligned<T: Element>(label: &str, blocks: &[Block<T>], size: usize, rows: Range<usize>) {
    let mut next = rows.start;
    for (i, block) in blocks.iter().enumerate() {
        assert_eq!(
            block.first, next,
            "{label}: block {i} starts where the last ended"
        );
        assert!(
            block.rows >= 1 && block.rows <= size,
            "{label}: block {i} has {} rows, the size is {size}",
            block.rows
        );
        if i > 0 {
            assert_eq!(
                block.first % size,
                0,
                "{label}: block {i} starts on a boundary"
            );
        }
        if i + 1 < blocks.len() {
            assert_eq!(
                (block.first + block.rows) % size,
                0,
                "{label}: block {i} ends on a boundary"
            );
        }
        next += block.rows;
    }
    assert_eq!(
        next, rows.end,
        "{label}: the blocks end where the read ends"
    );
}

// ---------------------------------------------------------------- the property test over the miniatures

fn check_dataset<T: Bits>(label: &str, dataset: &Dataset, rng: &mut Rng) {
    let shape = dataset.shape();
    let full = dataset
        .read_raw::<T>()
        .expect("the binding's own full read");
    let row_values: usize = shape[1..].iter().product();

    // the whole region, in blocks, is the full read
    let whole = Region::whole();
    let size = block_rows(dataset, &whole).expect("the block size");
    let blocks = blocks_of::<T>(dataset, &whole).expect("the whole dataset reads");
    assert!(
        same(&flatten(&blocks), &full),
        "{label}: the whole region in blocks differs from the full read"
    );
    assert_eq!(
        blocks.len(),
        shape[0].div_ceil(size),
        "{label}: the number of blocks"
    );
    assert_aligned(label, &blocks, size, 0..shape[0]);

    for _ in 0..6 {
        // a range of rows, and the rest of the dimensions whole
        let rows = pick_rows(rng, shape[0], size);
        let blocks = blocks_of::<T>(dataset, &Region::rows(rows.clone()))
            .unwrap_or_else(|e| panic!("{label}: rows {rows:?}: {e}"));
        assert!(
            same(
                &flatten(&blocks),
                &full[rows.start * row_values..rows.end * row_values]
            ),
            "{label}: rows {rows:?} differ from the slice of the full read"
        );
        assert_aligned(label, &blocks, size, rows.clone());

        // a rectangle, in two or three dimensions
        if shape.len() >= 2 {
            let mut ranges = vec![rows.clone(), pick_any(rng, shape[1])];
            if shape.len() == 3 {
                ranges.push(pick_any(rng, shape[2]));
            }
            let region = Region::new(ranges.clone());
            let mut want = Vec::new();
            for r in ranges[0].clone() {
                for i in ranges[1].clone() {
                    if shape.len() == 2 {
                        want.push(full[r * shape[1] + i]);
                    } else {
                        for j in ranges[2].clone() {
                            want.push(full[(r * shape[1] + i) * shape[2] + j]);
                        }
                    }
                }
            }
            let rect_size = block_rows(dataset, &region).expect("the block size of a rectangle");
            let blocks = blocks_of::<T>(dataset, &region)
                .unwrap_or_else(|e| panic!("{label}: region {ranges:?}: {e}"));
            assert!(
                same(&flatten(&blocks), &want),
                "{label}: the region {ranges:?} differs from the slice of the full read"
            );
            assert_aligned(label, &blocks, rect_size, ranges[0].clone());
            let selected = region_shape(dataset, &region).expect("the shape of the region");
            assert_eq!(
                selected,
                ranges.iter().map(|r| r.len()).collect::<Vec<_>>(),
                "{label}"
            );
        }
    }
}

fn element(dataset: &Dataset) -> Option<&'static str> {
    match dataset.dtype().and_then(|d| d.to_descriptor()) {
        Ok(TypeDescriptor::Integer(IntSize::U4)) => Some("int32"),
        Ok(TypeDescriptor::Unsigned(IntSize::U8)) => Some("uint64"),
        Ok(TypeDescriptor::Float(FloatSize::U4)) => Some("float32"),
        Ok(TypeDescriptor::Float(FloatSize::U8)) => Some("float64"),
        Ok(TypeDescriptor::Unsigned(IntSize::U1)) => Some("uint8"),
        _ => None,
    }
}

#[test]
fn ranged_reads_equal_slices_of_the_full_read() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // how many datasets of each type and number of dimensions were checked: the test must not be empty for any of them
    let mut seen: BTreeMap<(&str, usize), usize> = BTreeMap::new();
    for name in MINIATURES {
        let file = File::open(miniature_path(name)).expect("a miniature opens");
        for (path, dataset) in datasets(&file) {
            let Some(kind) = element(&dataset) else {
                continue;
            };
            let label = format!("{name} {path}");
            match kind {
                "int32" => check_dataset::<i32>(&label, &dataset, &mut rng),
                "uint64" => check_dataset::<u64>(&label, &dataset, &mut rng),
                "float32" => check_dataset::<f32>(&label, &dataset, &mut rng),
                "float64" => check_dataset::<f64>(&label, &dataset, &mut rng),
                _ => check_dataset::<u8>(&label, &dataset, &mut rng),
            }
            *seen.entry((kind, dataset.ndim())).or_default() += 1;
        }
    }
    for wanted in [
        ("int32", 1),
        ("int32", 3),
        ("uint64", 1),
        ("float32", 1),
        ("float64", 1),
        ("float64", 3),
        ("uint8", 1),
        ("uint8", 2),
        ("uint8", 3),
    ] {
        assert!(
            seen.contains_key(&wanted),
            "no {wanted:?} dataset was checked: {seen:?}"
        );
    }
}

// ---------------------------------------------------------------- the first values of the oracle

fn open_instance(instance: &Json) -> (String, Dataset) {
    let file_name = instance["file"].as_str().expect("a file");
    let path = instance["path"].as_str().expect("a path");
    let file = File::open(miniature_path(file_name)).expect("a miniature opens");
    let dataset = file.dataset(path).expect("the dataset of the oracle");
    (format!("{file_name} {path}"), dataset)
}

/// The first `n` values of a dataset in storage order, read as rows from the first.
fn first_values<T: Element>(dataset: &Dataset, n: usize) -> Vec<T> {
    let shape = dataset.shape();
    let row_values: usize = shape[1..].iter().product();
    let rows = n.div_ceil(row_values).min(shape[0]);
    let blocks = blocks_of::<T>(dataset, &Region::rows(0..rows)).expect("the first rows read");
    flatten(&blocks).into_iter().take(n).collect()
}

fn agree<T: Element>(
    label: &str,
    dataset: &Dataset,
    want: &[Json],
    matches: impl Fn(&T, &Json) -> bool,
) {
    let got = first_values::<T>(dataset, want.len());
    assert_eq!(got.len(), want.len(), "{label}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(
            matches(g, w),
            "{label}: value {i} is {w}, read something else"
        );
    }
}

#[test]
fn the_first_values_equal_the_oracle() {
    let oracle = oracle();
    let mut checked = BTreeMap::new();
    for kind in oracle["dataset_encodings"]
        .as_array()
        .expect("dataset kinds")
    {
        let name = kind["kind"].as_str().expect("a kind");
        let instance = &kind["instance"];
        let (label, dataset) = open_instance(instance);
        let want = instance["first_values"]
            .as_array()
            .expect("the first values");
        // a float is its text in the oracle, or `nan`
        let float = |v: f64, w: &Json| match w.as_str() {
            Some("nan") => v.is_nan(),
            Some(other) => panic!("{label}: the oracle holds {other}"),
            None => w.as_f64() == Some(v),
        };
        match name.split(',').next().expect("a type") {
            "int32" => agree::<i32>(&label, &dataset, want, |v, w| {
                w.as_i64() == Some(i64::from(*v))
            }),
            "uint64" => agree::<u64>(&label, &dataset, want, |v, w| w.as_u64() == Some(*v)),
            "uint8" => agree::<u8>(&label, &dataset, want, |v, w| {
                w.as_u64() == Some(u64::from(*v))
            }),
            "float32" => agree::<f32>(&label, &dataset, want, |v, w| float(f64::from(*v), w)),
            "float64" => agree::<f64>(&label, &dataset, want, |v, w| float(*v, w)),
            // the strings and the compounds are read by phase 3E
            _ => continue,
        }
        *checked.entry(name.to_string()).or_insert(0usize) += 1;
    }
    assert!(
        checked.len() >= 8,
        "the numeric kinds of the oracle were read: {checked:?}"
    );
}

// ---------------------------------------------------------------- datasets made at test time

/// A file with the datasets that `write` makes in its one group, opened for reading.
fn with_datasets(label: &str, write: impl FnOnce(&hdf5::Group)) -> (common::TempH5, File) {
    let on_disk = temp_h5(label, write);
    let file = File::open(&on_disk.0).expect("the file opens");
    (on_disk, file)
}

/// A `float32` dataset of `n` values (the value is its index), chunked and compressed if `chunk` is given.
fn floats(group: &hdf5::Group, name: &str, n: usize, chunk: Option<usize>) {
    let values: Vec<f32> = (0..n).map(|i| i as f32).collect();
    let builder = group.new_dataset::<f32>().shape(n);
    let dataset = match chunk {
        Some(c) => builder.chunk(c).deflate(6).create(name),
        None => builder.create(name),
    }
    .expect("a dataset");
    dataset.write_raw(&values).expect("its values");
}

#[test]
fn cancel_returns_cancelled_at_a_block_boundary() {
    let (_file, file) = with_datasets("ranged-cancel", |g| floats(g, "x", 25_000, Some(10_000)));
    let dataset = file.dataset("Mount/x").expect("the dataset");

    // not cancelled: three blocks of 10,000, 10,000 and 5,000
    let blocks = blocks_of::<f32>(&dataset, &Region::whole()).expect("a read");
    assert_eq!(
        blocks.iter().map(|b| b.rows).collect::<Vec<_>>(),
        [10_000, 10_000, 5_000]
    );

    // cancelled by the sink after the first block: the next block is not read
    let token = CancelToken::new();
    let context = ReadContext::new().with_cancel(token.clone());
    let mut delivered = 0;
    let outcome = read_blocks::<f32>(&dataset, &Region::whole(), &context, |_| {
        delivered += 1;
        token.cancel();
        Ok(())
    });
    assert!(matches!(
        outcome.expect_err("cancelled").kind(),
        ErrorKind::Cancelled
    ));
    assert_eq!(delivered, 1, "no block is read after the token fires");

    // cancelled before the first block: nothing is read
    let context = ReadContext::new().with_cancel(token);
    let mut delivered = 0;
    let outcome = read_blocks::<f32>(&dataset, &Region::whole(), &context, |_| {
        delivered += 1;
        Ok(())
    });
    assert!(matches!(
        outcome.expect_err("cancelled").kind(),
        ErrorKind::Cancelled
    ));
    assert_eq!(delivered, 0);
}

#[test]
fn progress_is_reported_monotonically() {
    let (_file, file) = with_datasets("ranged-progress", |g| {
        floats(g, "x", 25_000, Some(10_000));
        let table = g
            .new_dataset::<f32>()
            .shape([300, 100])
            .create("table")
            .expect("a dataset");
        table.write_raw(&vec![0.5f32; 30_000]).expect("its values");
    });
    let dataset = file.dataset("Mount/x").expect("the dataset");
    let seen: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let context = ReadContext::new().with_progress(Arc::new(move |done, total| {
        sink.lock().expect("a lock").push((done, total));
    }));

    read_blocks::<f32>(&dataset, &Region::whole(), &context, |_| Ok(())).expect("a read");
    assert_eq!(
        *seen.lock().expect("a lock"),
        [(10_000, 25_000), (20_000, 25_000), (25_000, 25_000)]
    );

    // a read that starts in the middle of a block: the total is the values asked for, and the first block ends on the boundary
    seen.lock().expect("a lock").clear();
    read_blocks::<f32>(&dataset, &Region::rows(5_000..22_000), &context, |_| Ok(()))
        .expect("a read");
    let reported = seen.lock().expect("a lock").clone();
    assert_eq!(
        reported,
        [(5_000, 17_000), (15_000, 17_000), (17_000, 17_000)]
    );
    assert!(
        reported.windows(2).all(|w| w[0].0 < w[1].0),
        "done only grows"
    );

    // progress counts values and not rows: a table of 300 rows of 100 values, in blocks of 100 rows
    seen.lock().expect("a lock").clear();
    let table = file.dataset("Mount/table").expect("the dataset");
    read_blocks::<f32>(&table, &Region::whole(), &context, |_| Ok(())).expect("a read");
    assert_eq!(
        *seen.lock().expect("a lock"),
        [(10_000, 30_000), (20_000, 30_000), (30_000, 30_000)]
    );

    // an empty range delivers no block and reports nothing
    seen.lock().expect("a lock").clear();
    let mut delivered = 0;
    read_blocks::<f32>(&dataset, &Region::rows(7..7), &context, |_| {
        delivered += 1;
        Ok(())
    })
    .expect("an empty range is not an error");
    assert_eq!(delivered, 0);
    assert!(seen.lock().expect("a lock").is_empty());
}

#[test]
#[allow(clippy::reversed_empty_ranges)]
fn a_range_outside_the_dataset_is_invalid_index() {
    let (_file, file) = with_datasets("ranged-outside", |g| {
        floats(g, "x", 25_000, Some(10_000));
        let image = g
            .new_dataset::<u8>()
            .shape([6, 8])
            .create("image")
            .expect("a dataset");
        image.write_raw(&[0u8; 48]).expect("its values");
    });
    let x = file.dataset("Mount/x").expect("the dataset");
    let image = file.dataset("Mount/image").expect("the dataset");
    let context = ReadContext::new();
    let invalid = |dataset: &Dataset, region: Region| {
        let mut delivered = 0;
        let outcome = read_blocks::<f32>(dataset, &region, &context, |_| {
            delivered += 1;
            Ok(())
        });
        assert_eq!(delivered, 0, "a bad range is said before any block is read");
        outcome.expect_err("an error")
    };
    for (what, error) in [
        ("one value too many", invalid(&x, Region::rows(0..25_001))),
        ("past the end", invalid(&x, Region::rows(30_000..30_001))),
        ("start after end", invalid(&x, Region::rows(10..5))),
        (
            "two ranges for one dimension",
            invalid(&x, Region::new(vec![0..1, 0..1])),
        ),
    ] {
        assert!(
            matches!(error.kind(), ErrorKind::InvalidIndex { .. }),
            "{what}: {error}"
        );
    }
    // along the second dimension of an image
    let mut delivered = 0;
    let outcome = read_blocks::<u8>(&image, &Region::new(vec![0..6, 0..9]), &context, |_| {
        delivered += 1;
        Ok(())
    });
    assert_eq!(delivered, 0);
    assert!(matches!(
        outcome.expect_err("a column too many").kind(),
        ErrorKind::InvalidIndex { .. }
    ));
    assert!(region_shape(&image, &Region::new(vec![2..4, 1..8])).is_ok());
    assert!(region_shape(&image, &Region::rows(4..9)).is_err());
}

// ---------------------------------------------------------------- the block rule and the refusals

#[test]
fn blocks_are_a_multiple_of_the_chunk_and_aligned() {
    let (_file, file) = with_datasets("ranged-blocks", |g| {
        floats(g, "chunked", 25_000, Some(10_000));
        floats(g, "one_chunk", 25_000, Some(25_000));
        floats(g, "contiguous", 25_000, None);
        let frame = g
            .new_dataset::<u8>()
            .shape([128, 128])
            .chunk([128, 128])
            .create("frame")
            .expect("a dataset");
        frame.write_raw(&vec![7u8; 128 * 128]).expect("its values");
        let grid = g
            .new_dataset::<u8>()
            .shape([20, 100, 5])
            .create("grid")
            .expect("a dataset");
        grid.write_raw(&vec![1u8; 20 * 100 * 5])
            .expect("its values");
    });
    let at = |name: &str| file.dataset(&format!("Mount/{name}")).expect("the dataset");
    let size = |name: &str, region: Region| block_rows(&at(name), &region).expect("the block size");

    // a chunk is the block, and never less; one chunk for all the values is one block (G17 is IP-06's)
    assert_eq!(size("chunked", Region::whole()), 10_000);
    assert_eq!(size("one_chunk", Region::whole()), 25_000);
    // not chunked: as many rows as keep a block at 10,000 values, and at least one row
    assert_eq!(size("contiguous", Region::whole()), 10_000);
    assert_eq!(size("grid", Region::whole()), 20);
    // the size follows the width of what is selected: a narrower region has more rows in a block
    assert_eq!(size("grid", Region::new(vec![0..20, 0..100, 0..1])), 100);
    // an image of one chunk: the frame is the block
    assert_eq!(size("frame", Region::whole()), 128);

    // a read that starts inside a block ends its first block on the boundary
    let blocks = blocks_of::<f32>(&at("chunked"), &Region::rows(3_000..25_000)).expect("a read");
    assert_eq!(
        blocks.iter().map(|b| (b.first, b.rows)).collect::<Vec<_>>(),
        [(3_000, 7_000), (10_000, 10_000), (20_000, 5_000)]
    );
    // inside one chunk there is one block of what was asked for, and the whole dataset is one block
    let blocks = blocks_of::<f32>(&at("one_chunk"), &Region::rows(100..200)).expect("a read");
    assert_eq!(
        blocks.iter().map(|b| (b.first, b.rows)).collect::<Vec<_>>(),
        [(100, 100)]
    );
    let blocks = blocks_of::<f32>(&at("one_chunk"), &Region::whole()).expect("a read");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].values.len(), 25_000);
    assert_eq!(blocks[0].values[24_999], 24_999.0);
}

#[test]
fn a_type_that_is_not_the_stored_type_is_not_converted() {
    let (_file, file) = with_datasets("ranged-types", |g| {
        floats(g, "x", 100, None);
        let counts = g
            .new_dataset::<i32>()
            .shape(10)
            .create("counts")
            .expect("a dataset");
        counts.write_raw(&[1i32; 10]).expect("its values");
    });
    let x = file.dataset("Mount/x").expect("the dataset");
    let counts = file.dataset("Mount/counts").expect("the dataset");
    let context = ReadContext::new();
    let refused = |outcome: Result<(), Error>| {
        let error = outcome.expect_err("refused");
        assert!(
            matches!(error.kind(), ErrorKind::UnsupportedInVersion { .. }),
            "{error}"
        );
    };
    // widening and narrowing, and a change of kind, are all refused
    refused(read_blocks::<f64>(&x, &Region::whole(), &context, |_| {
        Ok(())
    }));
    refused(read_blocks::<u8>(
        &x,
        &Region::whole(),
        &context,
        |_| Ok(()),
    ));
    refused(read_blocks::<u64>(
        &counts,
        &Region::whole(),
        &context,
        |_| Ok(()),
    ));
    refused(read_blocks::<f32>(
        &counts,
        &Region::whole(),
        &context,
        |_| Ok(()),
    ));
    // and the type that is stored reads
    assert!(read_blocks::<i32>(&counts, &Region::whole(), &context, |_| Ok(())).is_ok());
}

#[test]
fn a_dataset_of_no_dimension_or_of_four_is_not_read() {
    let (_file, file) = with_datasets("ranged-dimensions", |g| {
        let scalar = g
            .new_dataset::<f32>()
            .shape(())
            .create("scalar")
            .expect("a dataset");
        scalar.write_scalar(&1.5f32).expect("its value");
        let four = g
            .new_dataset::<f32>()
            .shape([2, 2, 2, 2])
            .create("four")
            .expect("a dataset");
        four.write_raw(&[0.0f32; 16]).expect("its values");
    });
    let context = ReadContext::new();
    for name in ["scalar", "four"] {
        let dataset = file.dataset(&format!("Mount/{name}")).expect("the dataset");
        let error = read_blocks::<f32>(&dataset, &Region::whole(), &context, |_| Ok(()))
            .expect_err("not read");
        assert!(
            matches!(error.kind(), ErrorKind::UnsupportedInVersion { .. }),
            "{name}: {error}"
        );
    }
}

#[test]
fn an_error_from_the_sink_stops_the_read_and_is_returned() {
    let (_file, file) = with_datasets("ranged-sink-error", |g| {
        floats(g, "x", 25_000, Some(10_000))
    });
    let dataset = file.dataset("Mount/x").expect("the dataset");
    let mut delivered = 0;
    let outcome = read_blocks::<f32>(&dataset, &Region::whole(), &ReadContext::new(), |_| {
        delivered += 1;
        Err(Error::new(ErrorKind::Closed))
    });
    assert!(matches!(
        outcome.expect_err("the sink's error").kind(),
        ErrorKind::Closed
    ));
    assert_eq!(delivered, 1);
}
