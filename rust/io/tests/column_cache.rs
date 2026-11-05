//! The decoded-chunk cache (IP-06 phase 6C, task T06.3; decisions G17, D06-5, D06-6 and D06-7), proved on datasets made at test time so that the blocks are known: a dataset of
//! 25,000 `float32` values in chunks of 10,000 is three blocks of 40,000, 40,000 and 20,000 bytes, and the value of each is its index.
//!
//! * `a_range_inside_one_block_is_a_slice_of_the_cached_buffer`: the point of the cache (G17): a range inside a block is the block's own shared buffer, not a copy.
//! * `a_context_with_no_budget_retains_nothing`, `the_cache_never_holds_more_than_the_budget`, `the_least_recently_used_block_goes_first` and
//!   `a_block_larger_than_the_budget_is_served_and_not_cached`: the budget is the context's, and it bounds what is retained and not what is returned.
//! * `a_range_across_blocks_equals_the_slice_of_the_full_read` and `a_precision_request_is_converted_from_the_cached_native_block`: what a cached read returns.
//! * `the_stats_count_hits_misses_and_bytes` and `a_warm_read_does_not_touch_the_file`: PS-12, and the proof that a hit reads no block from the file.
//! * The same holds through the V17 source, and for the real March layer when the large local fixture is there (`the_local_march_layer_is_served_warm_from_the_cache`).

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use build_log_io::columns::ColumnReader;
use build_log_model::{
    CacheStats, CancelToken, ChannelKey, ColumnData, ColumnRange, ColumnRequest, ErrorKind,
    LaserKey, LayerId, Precision, ReadContext,
};
use common::{pick_rows, temp_h5, FileLocator, Rng};
use hdf5::File;

const VALUES: usize = 25_000;
/// The bytes of a block of 10,000 `float32` values, and of the last block of 5,000.
const BLOCK: u64 = 40_000;
const LAST: u64 = 20_000;

fn laser() -> LaserKey {
    LaserKey::new("Laser_2", 2)
}

fn request(path: &str) -> ColumnRequest {
    ColumnRequest::new(LayerId(3), Some(laser()), ChannelKey::new([path]))
}

fn indices(path: &str, range: std::ops::Range<u64>) -> ColumnRequest {
    request(path).with_range(ColumnRange::Indices(range))
}

fn expected() -> Vec<f32> {
    (0..VALUES).map(|i| i as f32).collect()
}

fn put(group: &hdf5::Group, name: &str, values: &[f32]) {
    let dataset = group
        .new_dataset::<f32>()
        .shape(values.len())
        .chunk(10_000)
        .deflate(6)
        .create(name)
        .expect("a dataset");
    dataset.write_raw(values).expect("its values");
}

/// A reader over a file with the dataset `x` (the value is the index) and `y` (the value is the index plus a million).
fn reader_over(label: &str) -> (common::TempH5, ColumnReader<FileLocator>) {
    let on_disk = temp_h5(label, |g| {
        put(g, "x", &expected());
        put(
            g,
            "y",
            &expected().iter().map(|v| v + 1.0e6).collect::<Vec<_>>(),
        );
    });
    let file = File::open(&on_disk.0).expect("the file opens");
    (on_disk, ColumnReader::new(FileLocator::new(file)))
}

fn floats(data: &ColumnData) -> &[f32] {
    data.as_f32().expect("a float32 column")
}

fn budget(bytes: u64) -> ReadContext {
    ReadContext::new().with_cache_budget(bytes)
}

fn stats(reader: &ColumnReader<FileLocator>) -> CacheStats {
    reader.stats()
}

#[test]
fn a_range_inside_one_block_is_a_slice_of_the_cached_buffer() {
    let (_disk, reader) = reader_over("cache-slice");
    let context = budget(1 << 20);
    // the first read decodes the block and keeps it; the range is a slice of the kept block, and so is the next one: one buffer, with no copy
    let first = reader
        .read(&indices("Mount/x", 100..200), &context)
        .expect("a miss");
    let second = reader
        .read(&indices("Mount/x", 150..250), &context)
        .expect("a hit");
    let (a, b) = (floats(&first), floats(&second));
    assert_eq!(a, &expected()[100..200]);
    assert_eq!(b, &expected()[150..250]);
    assert_eq!(a.as_ptr().wrapping_add(50), b.as_ptr(), "one buffer");
    // and the whole of the block is the buffer itself
    let block = reader
        .read(&indices("Mount/x", 0..10_000), &context)
        .expect("a hit");
    assert_eq!(floats(&block).as_ptr().wrapping_add(100), a.as_ptr());
    assert_eq!(floats(&block).len(), 10_000);
    // the stored type, at the precision it is stored in, shares the buffer too
    let same = reader
        .read(
            &indices("Mount/x", 100..200).with_precision(Precision::F32),
            &context,
        )
        .expect("a hit");
    assert_eq!(floats(&same).as_ptr(), a.as_ptr());
}

#[test]
fn a_range_across_blocks_equals_the_slice_of_the_full_read() {
    let want = ColumnData::F32(build_log_model::Shared::new(expected()));
    let mut rng = Rng(0xA076_1D64_78BD_642F);
    for bytes in [0, BLOCK - 1, BLOCK, 2 * BLOCK, 1 << 20] {
        let (_disk, reader) = reader_over(&format!("cache-across-{bytes}"));
        let context = budget(bytes);
        // cold and then warm, at every precision, on and across the boundaries
        for _ in 0..24 {
            let rows = pick_rows(&mut rng, VALUES, 10_000);
            let precision = [Precision::Native, Precision::F32, Precision::F64][rng.up_to(2)];
            let got = reader
                .read(
                    &indices("Mount/x", rows.start as u64..rows.end as u64)
                        .with_precision(precision),
                    &context,
                )
                .unwrap_or_else(|e| panic!("budget {bytes}, rows {rows:?}: {e}"));
            let slice = want
                .slice(rows.clone())
                .expect("a slice")
                .with_precision(precision)
                .expect("numbers");
            assert_eq!(got, slice, "budget {bytes}: rows {rows:?} at {precision:?}");
        }
        // the whole column, once more
        assert_eq!(
            reader.read(&request("Mount/x"), &context).expect("all"),
            want,
            "budget {bytes}"
        );
    }
}

#[test]
fn a_context_with_no_budget_retains_nothing() {
    let (_disk, reader) = reader_over("cache-none");
    for _ in 0..2 {
        let got = reader
            .read(&request("Mount/x"), &ReadContext::new())
            .expect("all");
        assert_eq!(floats(&got), expected());
    }
    // every block was looked for, none was kept, and so none was found
    let s = stats(&reader);
    assert_eq!((s.bytes_held, s.hits, s.misses, s.evictions), (0, 0, 6, 0));
}

#[test]
fn the_cache_never_holds_more_than_the_budget() {
    let (_disk, reader) = reader_over("cache-bounded");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // room for one block of 40,000 bytes and not for two
    let context = budget(50_000);
    for _ in 0..40 {
        let rows = pick_rows(&mut rng, VALUES, 10_000);
        reader
            .read(
                &indices("Mount/x", rows.start as u64..rows.end as u64),
                &context,
            )
            .expect("a read");
        assert!(
            stats(&reader).bytes_held <= 50_000,
            "{} bytes after rows {rows:?}",
            stats(&reader).bytes_held
        );
    }
    let s = stats(&reader);
    assert!(s.evictions > 0 && s.budget == 50_000, "{s:?}");
}

#[test]
fn the_least_recently_used_block_goes_first() {
    let (_disk, reader) = reader_over("cache-lru");
    // room for two blocks of 40,000 bytes
    let context = budget(2 * BLOCK);
    let read = |rows: std::ops::Range<u64>| {
        reader
            .read(&indices("Mount/x", rows), &context)
            .expect("a read");
    };
    read(0..10); // block 0: a miss, kept
    read(10_000..10_010); // block 1: a miss, kept; the cache is full
    read(0..10); // block 0: a hit, and now the most recent
    let s = stats(&reader);
    assert_eq!(
        (s.hits, s.misses, s.evictions, s.bytes_held),
        (1, 2, 0, 2 * BLOCK)
    );
    read(20_000..20_010); // block 2 (20,000 bytes): block 1 is the least recent, and goes
    let s = stats(&reader);
    assert_eq!((s.misses, s.evictions, s.bytes_held), (3, 1, BLOCK + LAST));
    read(0..10); // block 0 is still there
    assert_eq!(stats(&reader).hits, 2);
    read(10_000..10_010); // block 1 is not
    assert_eq!(stats(&reader).misses, 4);
}

#[test]
fn a_block_larger_than_the_budget_is_served_and_not_cached() {
    let (_disk, reader) = reader_over("cache-too-big");
    let context = budget(1_000);
    let part = reader
        .read(&indices("Mount/x", 9_990..10_010), &context)
        .expect("a read that crosses two blocks");
    assert_eq!(floats(&part), &expected()[9_990..10_010]);
    let all = reader.read(&request("Mount/x"), &context).expect("all");
    assert_eq!(floats(&all), expected());
    // nothing was kept, nothing was pushed out, and nothing was found
    let s = stats(&reader);
    assert_eq!((s.bytes_held, s.evictions, s.hits), (0, 0, 0));
}

#[test]
fn a_precision_request_is_converted_from_the_cached_native_block() {
    let (_disk, reader) = reader_over("cache-precision");
    let context = budget(1 << 20);
    reader
        .read(&indices("Mount/x", 0..10_000), &context)
        .expect("a miss that keeps the block");
    let before = stats(&reader);
    // another precision is a hit on the native block, converted for this read, and the cache keeps no converted copy
    let doubles = reader
        .read(
            &indices("Mount/x", 100..200).with_precision(Precision::F64),
            &context,
        )
        .expect("a hit");
    let after = stats(&reader);
    assert_eq!(after.hits, before.hits + 1);
    assert_eq!(after.misses, before.misses);
    assert_eq!(
        after.bytes_held, before.bytes_held,
        "the block is kept as stored and only that"
    );
    let want: Vec<f64> = expected()[100..200].iter().map(|&v| f64::from(v)).collect();
    assert_eq!(doubles.as_f64().expect("float64"), want);
}

#[test]
fn the_stats_count_hits_misses_and_bytes() {
    let (_disk, reader) = reader_over("cache-stats");
    assert_eq!(stats(&reader), CacheStats::default());
    let context = budget(1 << 20);
    reader.read(&request("Mount/x"), &context).expect("all");
    let s = stats(&reader);
    assert_eq!(
        (s.hits, s.misses, s.evictions, s.bytes_held, s.budget),
        (0, 3, 0, 2 * BLOCK + LAST, 1 << 20)
    );
    reader
        .read(&request("Mount/x"), &context)
        .expect("all again");
    let s = stats(&reader);
    assert_eq!((s.hits, s.misses, s.bytes_held), (3, 3, 2 * BLOCK + LAST));
    reader
        .read(&indices("Mount/x", 0..5), &context)
        .expect("a range");
    assert_eq!(stats(&reader).hits, 4);
    // another column is other blocks: they are not found, and they are kept beside the first
    reader
        .read(&request("Mount/y"), &context)
        .expect("another column");
    let s = stats(&reader);
    assert_eq!(
        (s.hits, s.misses, s.bytes_held),
        (4, 6, 2 * (2 * BLOCK + LAST))
    );
}

#[test]
fn different_columns_do_not_share_blocks() {
    let (_disk, reader) = reader_over("cache-keys");
    let context = budget(1 << 20);
    let x = reader.read(&indices("Mount/x", 5..8), &context).expect("x");
    let y = reader.read(&indices("Mount/y", 5..8), &context).expect("y");
    assert_eq!(floats(&x), [5.0, 6.0, 7.0]);
    assert_eq!(floats(&y), [1_000_005.0, 1_000_006.0, 1_000_007.0]);
    // the same dataset under another layer or laser is another column too
    let other_layer = ColumnRequest::new(LayerId(4), Some(laser()), ChannelKey::new(["Mount/x"]))
        .with_range(ColumnRange::Indices(5..8));
    let before = stats(&reader).misses;
    reader.read(&other_layer, &context).expect("another layer");
    assert_eq!(stats(&reader).misses, before + 1, "another layer is a miss");
}

#[test]
fn a_warm_read_does_not_touch_the_file() {
    // a file that is opened for writing, so that the values can be changed under the reader
    let on_disk = temp_h5("cache-warm", |g| put(g, "x", &expected()));
    let file = File::open_rw(&on_disk.0).expect("the file opens");
    let dataset = file.dataset("Mount/x").expect("the dataset");
    let reader = ColumnReader::new(FileLocator::new(file));
    let context = budget(1 << 20);

    let cold = reader.read(&request("Mount/x"), &context).expect("cold");
    assert_eq!(floats(&cold), expected());
    // the file now holds other values: a read that went to the file would show them
    dataset
        .write_raw(&vec![-1.0f32; VALUES])
        .expect("new values");
    let warm = reader.read(&request("Mount/x"), &context).expect("warm");
    assert_eq!(
        floats(&warm),
        expected(),
        "the blocks were served from the cache"
    );
    // and a column the cache does not hold is read from the file as it is
    let other = reader
        .read(
            &ColumnRequest::new(LayerId(9), Some(laser()), ChannelKey::new(["Mount/x"])),
            &context,
        )
        .expect("another column");
    assert_eq!(floats(&other), vec![-1.0f32; VALUES]);
}

#[test]
fn progress_and_cancel_are_the_same_with_a_cache() {
    let (_disk, reader) = reader_over("cache-progress");
    let seen: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let context = budget(1 << 20).with_progress(Arc::new(move |done, total| {
        sink.lock().expect("a lock").push((done, total));
    }));
    // cold and then warm: the reports are those of a read with no cache, and they end at the total
    for _ in 0..2 {
        seen.lock().expect("a lock").clear();
        reader
            .read(&indices("Mount/x", 3_000..22_000), &context)
            .expect("a read");
        assert_eq!(
            *seen.lock().expect("a lock"),
            [(7_000, 19_000), (17_000, 19_000), (19_000, 19_000)]
        );
    }
    // cancelled by the first report: nothing is returned, before or after the cache holds the blocks
    for _ in 0..2 {
        let token = CancelToken::new();
        let reports = Arc::new(AtomicUsize::new(0));
        let (cancel, count) = (token.clone(), Arc::clone(&reports));
        let context = budget(1 << 20)
            .with_cancel(token)
            .with_progress(Arc::new(move |_, _| {
                count.fetch_add(1, Ordering::SeqCst);
                cancel.cancel();
            }));
        let outcome = reader.read(&request("Mount/x"), &context);
        assert!(matches!(
            outcome.expect_err("cancelled").kind(),
            ErrorKind::Cancelled
        ));
        assert_eq!(reports.load(Ordering::SeqCst), 1);
    }
}
