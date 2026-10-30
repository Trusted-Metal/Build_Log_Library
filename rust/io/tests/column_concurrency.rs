//! Readers at once (IP-06 phase 6D, task T06.4; decisions D06-8 and PS-03, PS-09): the cache and the engine under many threads.
//!
//! * `the_source_is_send_and_sync`: the sources, the reader and the cache can be shared between threads (a compile-time test).
//! * `concurrent_reads_of_different_layers_equal_the_serial_result`: every column of a real file with seven layers, read by eight threads sharing one source, equals the serial read, and
//!   **each block was decoded once**.
//! * `results_do_not_depend_on_the_number_of_threads`: the same requests from 1, 2 and 8 threads, in other orders, at several budgets, give the same columns (PS-09).
//! * `a_block_that_two_readers_need_is_decoded_once` and `cancelling_one_reader_does_not_fail_the_other`: the one-fill rule and PS-03 on a block that takes long enough to decode
//!   that readers meet at it. The protocol itself, in every order of events, is held by the unit tests of the cache.
//! * `a_reader_blocked_in_the_locator_does_not_block_the_cache`: a reader held inside a call into the file does not keep another from the cache.

mod common;

use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use build_log_io::adapters::v17::source::V17Source;
use build_log_io::columns::{Cache, ColumnReader, Located, Locator};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildSource, CancelToken, ChannelKey, ColumnData, ColumnRange, ColumnRequest, Error, ErrorKind,
    LaserKey, LayerId, Precision, Presence, ReadContext,
};
use build_log_testkit::{miniature, miniature_path};
use common::{pick_rows, temp_h5, FileLocator, Rng};
use hdf5::File;

const WAIT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------- a compile-time test

fn is_send_and_sync<T: Send + Sync>() {}

#[test]
fn the_source_is_send_and_sync() {
    is_send_and_sync::<V17Source>();
    is_send_and_sync::<ColumnReader<FileLocator>>();
    is_send_and_sync::<Cache>();
    is_send_and_sync::<Box<dyn BuildSource>>();
    is_send_and_sync::<ReadContext>();
    is_send_and_sync::<CancelToken>();
}

// ---------------------------------------------------------------- datasets made at test time

fn laser() -> LaserKey {
    LaserKey::new("Laser_2", 2)
}

fn request(path: &str) -> ColumnRequest {
    ColumnRequest::new(LayerId(3), Some(laser()), ChannelKey::new([path]))
}

fn indices(path: &str, range: std::ops::Range<u64>) -> ColumnRequest {
    request(path).with_range(ColumnRange::Indices(range))
}

fn put(group: &hdf5::Group, name: &str, values: &[f32], chunk: usize) {
    let dataset = group
        .new_dataset::<f32>()
        .shape(values.len())
        .chunk(chunk)
        .deflate(1)
        .create(name)
        .expect("a dataset");
    dataset.write_raw(values).expect("its values");
}

/// A column that takes long enough to decode that readers meet at it: four million `float32` values (16 MB) in one chunk, which compress well.
const BIG: usize = 4_000_000;

fn big_values() -> Vec<f32> {
    (0..BIG).map(|i| (i % 1_000) as f32).collect()
}

fn big_reader(label: &str) -> (common::TempH5, ColumnReader<FileLocator>) {
    let on_disk = temp_h5(label, |g| put(g, "big", &big_values(), BIG));
    let file = File::open(&on_disk.0).expect("the file opens");
    (on_disk, ColumnReader::new(FileLocator::new(file)))
}

fn floats(data: &ColumnData) -> &[f32] {
    data.as_f32().expect("a float32 column")
}

// ---------------------------------------------------------------- many layers, many threads

#[test]
fn concurrent_reads_of_different_layers_equal_the_serial_result() {
    // the file with seven layers and two lasers: its columns are read once, serially and with no cache, and then by eight threads that share one source
    let serial = open(production(), miniature_path(miniature::V17_SPECIAL_LAYERS)).expect("opens");
    let shared = open(production(), miniature_path(miniature::V17_SPECIAL_LAYERS)).expect("opens");
    let build = serial.source().build().expect("the metadata");
    let mut requests: Vec<ColumnRequest> = Vec::new();
    for laser in build.lasers() {
        for channel in &laser.channels {
            for (layer, presence) in channel.presence.iter() {
                if matches!(presence, Presence::Recorded { .. }) {
                    requests.push(ColumnRequest::new(
                        layer,
                        Some(laser.key.clone()),
                        channel.key.clone(),
                    ));
                }
            }
        }
    }
    assert!(requests.len() >= 90, "{} columns", requests.len());
    let plain = ReadContext::new();
    let reference: Vec<ColumnData> = requests
        .iter()
        .map(|r| serial.source().read(r, &plain).expect("the serial read"))
        .collect();

    let kept = ReadContext::new().with_cache_budget(256 << 20);
    let source = shared.source();
    thread::scope(|scope| {
        for t in 0..8 {
            let (requests, reference, kept) = (&requests, &reference, &kept);
            scope.spawn(move || {
                // every thread reads every column, each from another place in the list, so the threads meet at the same blocks at different times
                for i in 0..requests.len() {
                    let at = (i + t * requests.len() / 8) % requests.len();
                    let got = source.read(&requests[at], kept).expect("a concurrent read");
                    assert_eq!(got, reference[at], "thread {t}: column {at}");
                }
            });
        }
    });
    // each column is one block, and **each block was decoded once**: one miss for each, and every other read of it a hit
    let stats = source.cache_stats().expect("stats");
    let columns = requests.len() as u64;
    assert_eq!(
        (stats.misses, stats.hits, stats.evictions),
        (columns, 7 * columns, 0),
        "{stats:?}"
    );
}

#[test]
fn results_do_not_depend_on_the_number_of_threads() {
    let on_disk = temp_h5("concurrency-threads", |g| {
        let values: Vec<f32> = (0..25_000).map(|i| i as f32).collect();
        put(g, "x", &values, 10_000);
        put(
            g,
            "y",
            &values.iter().map(|v| v + 1.0e6).collect::<Vec<_>>(),
            10_000,
        );
    });
    let locator = FileLocator::new(File::open(&on_disk.0).expect("the file opens"));
    // the requests: ranges on and across the block boundaries of two columns, at every precision
    let mut rng = Rng(0xC2B2_AE3D_27D4_EB4F);
    let requests: Vec<ColumnRequest> = (0..60)
        .map(|_| {
            let rows = pick_rows(&mut rng, 25_000, 10_000);
            let precision = [Precision::Native, Precision::F32, Precision::F64][rng.up_to(2)];
            let column = ["Mount/x", "Mount/y"][rng.up_to(1)];
            indices(column, rows.start as u64..rows.end as u64).with_precision(precision)
        })
        .collect();
    let plain = ReadContext::new();
    let reference: Vec<ColumnData> = requests
        .iter()
        .map(|r| build_log_io::columns::read_column(r, &locator, &plain).expect("the serial read"))
        .collect();

    let reader = ColumnReader::new(locator);
    for budget in [0u64, 40_000, 1 << 20] {
        for threads in [1usize, 2, 8] {
            let context = ReadContext::new().with_cache_budget(budget);
            thread::scope(|scope| {
                for t in 0..threads {
                    let (requests, reference, reader, context) =
                        (&requests, &reference, &reader, &context);
                    scope.spawn(move || {
                        // the same list, from another place in it and, for the odd threads, backwards
                        let n = requests.len();
                        for i in 0..n {
                            let at = if t % 2 == 0 {
                                (i + t * 7) % n
                            } else {
                                n - 1 - (i + t * 5) % n
                            };
                            let got = reader.read(&requests[at], context).expect("a read");
                            assert_eq!(
                                got, reference[at],
                                "budget {budget}, {threads} threads, thread {t}, request {at}"
                            );
                        }
                    });
                }
            });
            // with no budget nothing was ever kept, whatever the number of threads
            if budget == 0 {
                assert_eq!(reader.stats().bytes_held, 0);
            }
        }
    }
}

// ---------------------------------------------------------------- readers that meet at one block

#[test]
fn a_block_that_two_readers_need_is_decoded_once() {
    for threads in [2usize, 8] {
        let (_disk, reader) = big_reader(&format!("concurrency-once-{threads}"));
        let context = ReadContext::new().with_cache_budget(64 << 20);
        let expected = big_values();
        let start = Arc::new(Barrier::new(threads));
        thread::scope(|scope| {
            for t in 0..threads {
                let (reader, context, start, expected) =
                    (&reader, &context, Arc::clone(&start), &expected);
                scope.spawn(move || {
                    start.wait();
                    // each reader wants a different range of the one block
                    let from = 100_000 + 1_000 * t as u64;
                    let got = reader
                        .read(&indices("Mount/big", from..from + 500), context)
                        .expect("a read");
                    assert_eq!(floats(&got), &expected[from as usize..from as usize + 500]);
                });
            }
        });
        // one reader decoded the block, and every other was served from the cache: without the rule each would have decoded it, one after the other
        let stats = reader.stats();
        assert_eq!(
            (stats.misses, stats.hits, stats.evictions),
            (1, threads as u64 - 1, 0),
            "{threads} readers: {stats:?}"
        );
        assert_eq!(stats.bytes_held, 4 * BIG as u64);
    }
}

#[test]
fn cancelling_one_reader_does_not_fail_the_other() {
    let expected = big_values();
    // a reader that waits for another's decode and is cancelled while it waits returns `Cancelled`; the decode goes on, and the reader that does it is not failed. The waiter must
    // arrive while the block is being decoded, which takes some milliseconds, so the attempt is made again (with a cache that is empty) if it arrived too late
    let mut waited_and_was_cancelled = false;
    for attempt in 0..6 {
        let (_disk, reader) = big_reader(&format!("concurrency-cancel-{attempt}"));
        let context = ReadContext::new().with_cache_budget(64 << 20);
        let token = CancelToken::new();
        let waiter_context = ReadContext::new()
            .with_cache_budget(64 << 20)
            .with_cancel(token.clone());
        let (filler_result, waiter_result) = thread::scope(|scope| {
            let filler = scope.spawn(|| reader.read(&indices("Mount/big", 1_000..1_500), &context));
            // the decode has begun by now, and takes tens of milliseconds
            thread::sleep(Duration::from_millis(15));
            let waiter =
                scope.spawn(|| reader.read(&indices("Mount/big", 2_000..2_500), &waiter_context));
            thread::sleep(Duration::from_millis(15));
            token.cancel();
            (
                filler.join().expect("the filler"),
                waiter.join().expect("the waiter"),
            )
        });
        // the reader that decodes is never failed by the other's cancellation, and what it returns is right
        let filler = filler_result.expect("the reader that decodes the block is not failed");
        assert_eq!(floats(&filler), &expected[1_000..1_500]);
        match waiter_result {
            Err(error) => {
                assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error}");
                waited_and_was_cancelled = true;
            }
            // it came after the block was kept: it was served, and what it returns is right
            Ok(got) => assert_eq!(floats(&got), &expected[2_000..2_500]),
        }
        // the block was decoded once whatever happened, and a reader that comes later is served from the cache
        let later = reader
            .read(&indices("Mount/big", 3_000..3_500), &context)
            .expect("a later reader");
        assert_eq!(floats(&later), &expected[3_000..3_500]);
        assert_eq!(
            reader.stats().misses,
            1,
            "attempt {attempt}: {:?}",
            reader.stats()
        );
        if waited_and_was_cancelled {
            break;
        }
    }
    assert!(
        waited_and_was_cancelled,
        "no attempt had a reader waiting for the decode when it was cancelled"
    );

    // the other way round: the reader that is decoding is cancelled, and the reader that waits for it is not failed (the decode is one call into the file and is not cut short,
    // so the block is kept and the waiter is served)
    let (_disk, reader) = big_reader("concurrency-cancel-filler");
    let token = CancelToken::new();
    let filler_context = ReadContext::new()
        .with_cache_budget(64 << 20)
        .with_cancel(token.clone());
    let waiter_context = ReadContext::new().with_cache_budget(64 << 20);
    thread::scope(|scope| {
        let filler =
            scope.spawn(|| reader.read(&indices("Mount/big", 1_000..1_500), &filler_context));
        thread::sleep(Duration::from_millis(15));
        let waiter =
            scope.spawn(|| reader.read(&indices("Mount/big", 2_000..2_500), &waiter_context));
        thread::sleep(Duration::from_millis(15));
        token.cancel();
        let waiter = waiter
            .join()
            .expect("the waiter")
            .expect("the waiter is not failed by the filler's cancellation");
        assert_eq!(floats(&waiter), &expected[2_000..2_500]);
        // the filler is either cancelled or finished; it is never anything else
        match filler.join().expect("the filler") {
            Ok(got) => assert_eq!(floats(&got), &expected[1_000..1_500]),
            Err(error) => assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error}"),
        }
    });
}

// ---------------------------------------------------------------- a reader held inside a call into the file

/// A locator that holds the reader that asks for one dataset until the test lets it go.
struct Gate {
    inner: FileLocator,
    held: &'static str,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Locator for Gate {
    fn version(&self) -> &str {
        self.inner.version()
    }

    fn locate(&self, request: &ColumnRequest) -> Result<Located, Error> {
        if request.channel.segments.join("/") == self.held {
            if let Some(entered) = self.entered.lock().expect("a lock").take() {
                entered.send(()).ok();
            }
            self.release.lock().expect("a lock").recv().ok();
        }
        self.inner.locate(request)
    }
}

#[test]
fn a_reader_blocked_in_the_locator_does_not_block_the_cache() {
    let on_disk = temp_h5("concurrency-gate", |g| {
        let values: Vec<f32> = (0..25_000).map(|i| i as f32).collect();
        put(g, "x", &values, 10_000);
        put(g, "z", &values, 10_000);
    });
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let reader = Arc::new(ColumnReader::new(Gate {
        inner: FileLocator::new(File::open(&on_disk.0).expect("the file opens")),
        held: "Mount/z",
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(release_rx),
    }));
    let context = ReadContext::new().with_cache_budget(1 << 20);

    // one reader is held inside the locator, which is a call into the file
    let held = {
        let (reader, context) = (Arc::clone(&reader), context.clone());
        thread::spawn(move || reader.read(&request("Mount/z"), &context))
    };
    entered_rx
        .recv_timeout(WAIT)
        .expect("the reader is inside the locator");

    // meanwhile another reader fills and then hits a block, and the statistics are read: none of it waits for the held one
    let (done_tx, done_rx) = mpsc::channel();
    {
        let (reader, context) = (Arc::clone(&reader), context.clone());
        thread::spawn(move || {
            let cold = reader.read(&indices("Mount/x", 0..100), &context);
            let warm = reader.read(&indices("Mount/x", 50..150), &context);
            let stats = reader.stats();
            done_tx
                .send(cold.is_ok() && warm.is_ok() && stats.hits == 1 && stats.misses == 1)
                .ok();
        });
    }
    let went_through = done_rx.recv_timeout(WAIT);
    // the held reader is let go whatever happened, so that a failure does not hang the test
    release_tx.send(()).ok();
    assert_eq!(
        went_through,
        Ok(true),
        "a reader inside the locator kept another from the cache"
    );
    let finished = held
        .join()
        .expect("the held reader")
        .expect("it reads once it is let go");
    assert_eq!(finished.len(), 25_000);
}
