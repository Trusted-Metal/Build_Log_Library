//! The decoded-chunk cache (IP-06 phases 6C and 6D, tasks T06.3 and T06.4; decisions G17, D06-5 to D06-8).
//!
//! * **The unit is a block** (the engine's unit of reading: one chunk, or the whole dataset where it has one chunk for all its values), **decoded and native**: a conversion to
//!   another precision is made from it and is never kept. The value is a [`Shared`] buffer, so that a range inside one block is handed out as a slice of it, **with no copy**
//!   (the reason G17 gives for the shared buffer).
//! * **The key** is the column's identity as the request gives it (layer, laser, channel), the index of the block, and a **generation**. The generation is `0` for a file that is
//!   opened for reading and does not change (D06-7); a source that can grow changes it when it does, so that no block of the old generation is ever served again.
//! * **Least recently used, by bytes.** A hit makes a block the most recent. A block is inserted only if it fits the budget of the read that decoded it, after evicting the least
//!   recently used blocks until it does. A block larger than the budget is not inserted. **A hit is served whatever the budget**: the budget bounds what is retained, not what
//!   is read. A block that a caller still holds stays alive for as long as the caller holds it, and no longer counts here.
//! * **One mutex guards the index, and it is never held while a block is read from the file** (decision D06-8): a miss is decoded by its reader outside the lock and inserted
//!   after.
//! * **A block is decoded once** (decision D06-8, phase 6D). The reader that misses a block **registers a fill** and decodes it; a reader that needs the same block meanwhile
//!   **waits** for that fill and is then served from the cache. **Cancelling one reader never fails another**: a waiter that is cancelled returns `Cancelled` and the fill goes on,
//!   and a filler that is cancelled, fails or panics **hands the block on**: its fill is withdrawn, a waiter wakes and takes the fill over. A waiter checks its token every
//!   [`POLL`] (a token has no way to wake it), and is woken at once when a fill ends.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use build_log_model::{CacheStats, ChannelKey, Error, LaserKey, LayerId, ReadContext, Shared};

/// How often a reader that waits for another's fill looks at its own token.
const POLL: Duration = Duration::from_millis(20);

/// A decoded block, of whichever stored type the column has.
#[derive(Debug, Clone)]
pub(super) enum Held {
    I16(Shared<i16>),
    I32(Shared<i32>),
    U64(Shared<u64>),
    F32(Shared<f32>),
    F64(Shared<f64>),
    U8(Shared<u8>),
}

impl Held {
    fn bytes(&self) -> u64 {
        match self {
            Held::I16(v) => std::mem::size_of_val(v.as_slice()) as u64,
            Held::I32(v) => std::mem::size_of_val(v.as_slice()) as u64,
            Held::U64(v) => std::mem::size_of_val(v.as_slice()) as u64,
            Held::F32(v) => std::mem::size_of_val(v.as_slice()) as u64,
            Held::F64(v) => std::mem::size_of_val(v.as_slice()) as u64,
            Held::U8(v) => std::mem::size_of_val(v.as_slice()) as u64,
        }
    }
}

/// Which block of which column of which generation of a source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Key {
    pub layer: LayerId,
    pub laser: Option<LaserKey>,
    pub channel: ChannelKey,
    pub block: usize,
    pub generation: u64,
}

struct Entry {
    held: Held,
    bytes: u64,
    /// When the block was last used: a later use has a larger number.
    tick: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Key, Entry>,
    /// The blocks in the order of their last use, the least recent first.
    order: BTreeMap<u64, Key>,
    tick: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    bytes_held: u64,
    budget: u64,
    /// The blocks that a reader is decoding now, for others to wait for.
    filling: HashSet<Key>,
    /// How many readers wait for a fill now.
    waiting: usize,
}

/// The cache of one source.
pub struct Cache {
    inner: Mutex<Inner>,
    /// Told when a fill ends, whether it kept its block or gave it up.
    ended: Condvar,
    poll: Duration,
}

impl Default for Cache {
    fn default() -> Self {
        Cache::new()
    }
}

/// The block under `key` made the most recent, counted as a hit; `None` if it is not there (nothing is counted).
fn hit(inner: &mut Inner, key: &Key) -> Option<Held> {
    inner.tick += 1;
    let tick = inner.tick;
    let (held, before) = inner
        .entries
        .get_mut(key)
        .map(|entry| (entry.held.clone(), std::mem::replace(&mut entry.tick, tick)))?;
    inner.order.remove(&before);
    inner.order.insert(tick, key.clone());
    inner.hits += 1;
    Some(held)
}

/// Keeps `held` under `key` if it fits `budget`, after dropping the least recently used blocks that stand in the way. Returns whether it was kept.
fn insert_locked(inner: &mut Inner, key: Key, held: Held, budget: u64) -> bool {
    let bytes = held.bytes();
    if bytes > budget {
        return false;
    }
    inner.budget = budget;
    // a block of the same key is replaced
    if let Some(old) = inner.entries.remove(&key) {
        inner.order.remove(&old.tick);
        inner.bytes_held -= old.bytes;
    }
    while inner.bytes_held + bytes > budget {
        let Some((_, oldest)) = inner.order.pop_first() else {
            break;
        };
        if let Some(gone) = inner.entries.remove(&oldest) {
            inner.bytes_held -= gone.bytes;
            inner.evictions += 1;
        }
    }
    inner.tick += 1;
    let tick = inner.tick;
    inner.order.insert(tick, key.clone());
    inner.bytes_held += bytes;
    inner.entries.insert(key, Entry { held, bytes, tick });
    true
}

/// A fill that a reader has registered. If the reader does not complete it (it is cancelled, fails, or panics) it is withdrawn when this is dropped, and the readers that wait
/// for it are woken to take it over.
struct Fill<'a> {
    cache: &'a Cache,
    key: Key,
    done: bool,
}

impl Fill<'_> {
    /// Keeps the block (if it fits `budget`), ends the fill and wakes the readers that wait for it.
    fn complete(mut self, held: Held, budget: u64) {
        let mut inner = self.cache.lock();
        inner.filling.remove(&self.key);
        insert_locked(&mut inner, self.key.clone(), held, budget);
        self.done = true;
        drop(inner);
        self.cache.ended.notify_all();
    }
}

impl Drop for Fill<'_> {
    fn drop(&mut self) {
        if !self.done {
            let mut inner = self.cache.lock();
            inner.filling.remove(&self.key);
            drop(inner);
            self.cache.ended.notify_all();
        }
    }
}

impl Cache {
    pub fn new() -> Self {
        Cache {
            inner: Mutex::new(Inner::default()),
            ended: Condvar::new(),
            poll: POLL,
        }
    }

    /// A cache whose waiters look at their token only every `poll`: a test that wants to show that a waiter is woken, and not merely noticing, makes it long.
    #[cfg(test)]
    fn with_poll(poll: Duration) -> Self {
        Cache {
            poll,
            ..Cache::new()
        }
    }

    /// The lock, even if a reader panicked while holding it: the index is only ever left whole.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The block under `key`, which then is the most recently used, or `None`. Counts a hit or a miss.
    #[cfg(test)]
    fn get(&self, key: &Key) -> Option<Held> {
        let mut inner = self.lock();
        let found = hit(&mut inner, key);
        if found.is_none() {
            inner.misses += 1;
        }
        found
    }

    /// Keeps `held` under `key` if it fits `budget`, after dropping the least recently used blocks that stand in the way. Returns whether it was kept.
    #[cfg(test)]
    fn insert(&self, key: Key, held: Held, budget: u64) -> bool {
        insert_locked(&mut self.lock(), key, held, budget)
    }

    /// How many readers wait for another's fill now.
    #[cfg(test)]
    fn waiting(&self) -> usize {
        self.lock().waiting
    }

    /// The block `key`, of `bytes` bytes, for a read whose budget is `budget`.
    ///
    /// * **In the cache:** it is returned (a hit), whatever the budget.
    /// * **Another reader is decoding it:** this reader waits for that fill and is then served (a hit), or, if the fill was given up, takes it over. A waiter that is cancelled
    ///   returns `Cancelled`.
    /// * **Nobody has it and it fits the budget:** this reader registers a fill, **decodes it with `decode` outside the lock**, keeps it and wakes the waiters (a miss).
    /// * **Nobody has it and it does not fit the budget:** `None` (a miss), and the reader decodes what it needs for itself.
    pub(super) fn block(
        &self,
        key: &Key,
        context: &ReadContext,
        budget: u64,
        bytes: u64,
        decode: impl FnOnce() -> Result<Held, Error>,
    ) -> Result<Option<Held>, Error> {
        let mut inner = self.lock();
        loop {
            if let Some(held) = hit(&mut inner, key) {
                return Ok(Some(held));
            }
            if !inner.filling.contains(key) {
                inner.misses += 1;
                if bytes > budget {
                    return Ok(None);
                }
                inner.filling.insert(key.clone());
                break;
            }
            // another reader is decoding this block: wait for it, but never past our own cancellation
            context.check_cancelled()?;
            inner.waiting += 1;
            let (woken, _) = self
                .ended
                .wait_timeout(inner, self.poll)
                .unwrap_or_else(|e| e.into_inner());
            inner = woken;
            inner.waiting -= 1;
        }
        // the lock is not held while the block is read from the file (decision D06-8)
        drop(inner);
        let fill = Fill {
            cache: self,
            key: key.clone(),
            done: false,
        };
        // a reader cancelled before its read starts hands the block on
        context.check_cancelled()?;
        let held = decode()?;
        fill.complete(held.clone(), budget);
        Ok(Some(held))
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.lock();
        let mut stats = CacheStats::default();
        stats.hits = inner.hits;
        stats.misses = inner.misses;
        stats.evictions = inner.evictions;
        stats.bytes_held = inner.bytes_held;
        stats.budget = inner.budget;
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(block: usize, generation: u64) -> Key {
        Key {
            layer: LayerId(1),
            laser: Some(LaserKey::new("first", 1)),
            channel: ChannelKey::new(["x"]),
            block,
            generation,
        }
    }

    fn block(values: usize) -> Held {
        Held::F32(Shared::new(vec![0.5f32; values]))
    }

    #[test]
    fn a_block_of_another_generation_is_not_served() {
        let cache = Cache::new();
        assert!(cache.insert(key(0, 0), block(10), 1_000));
        assert!(cache.get(&key(0, 0)).is_some());
        // the same block of the same column, after the source has changed: a miss, and the old one stays until it is pushed out
        assert!(cache.get(&key(0, 1)).is_none());
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn a_block_that_does_not_fit_is_not_kept_and_a_replaced_one_is_counted_once() {
        let cache = Cache::new();
        // 100 values of 4 bytes against a budget of 399
        assert!(!cache.insert(key(0, 0), block(100), 399));
        assert_eq!(cache.stats().bytes_held, 0);
        assert!(cache.insert(key(0, 0), block(100), 400));
        assert!(cache.insert(key(0, 0), block(100), 400));
        assert_eq!(cache.stats().bytes_held, 400);
        assert_eq!(cache.stats().evictions, 0);
    }

    #[test]
    fn a_smaller_budget_pushes_out_what_the_cache_holds_to_make_room() {
        let cache = Cache::new();
        assert!(cache.insert(key(0, 0), block(100), 10_000));
        assert!(cache.insert(key(1, 0), block(100), 10_000));
        assert_eq!(cache.stats().bytes_held, 800);
        // a read with a budget of one block keeps only itself
        assert!(cache.insert(key(2, 0), block(100), 400));
        let stats = cache.stats();
        assert_eq!(
            (stats.bytes_held, stats.evictions, stats.budget),
            (400, 2, 400)
        );
        assert!(cache.get(&key(0, 0)).is_none() && cache.get(&key(2, 0)).is_some());
    }

    // ---------------------------------------------------------------- readers at once (phase 6D): the fills are driven by hand, so that every order of events is exact

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;

    use build_log_model::{CancelToken, ErrorKind};

    const WAIT: Duration = Duration::from_secs(5);

    /// Waits until `condition` holds, for five seconds at most.
    fn until(condition: impl Fn() -> bool) -> bool {
        for _ in 0..500 {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    type Outcome = thread::JoinHandle<Result<Option<Held>, Error>>;

    /// A reader that is decoding a block: it has registered the fill and is held in its decode until `release` is sent (`started` says it got there), and then ends it with `ending`.
    fn decoding(
        cache: &Arc<Cache>,
        key: Key,
        ending: fn() -> Result<Held, Error>,
    ) -> (Outcome, mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let cache = Arc::clone(cache);
        let reader = thread::spawn(move || {
            cache.block(&key, &ReadContext::new(), 1_000, 400, || {
                started_tx.send(()).ok();
                release_rx.recv().ok();
                ending()
            })
        });
        (reader, release_tx, started_rx)
    }

    #[test]
    fn the_index_lock_is_not_held_while_a_block_is_being_filled() {
        let cache = Arc::new(Cache::new());
        let (filler, release, started) = decoding(&cache, key(0, 0), || Ok(block(100)));
        started.recv_timeout(WAIT).expect("the fill started");
        // while that block is being read from the file, the statistics and another block's fill go through
        let (done_tx, done_rx) = mpsc::channel();
        let other = Arc::clone(&cache);
        thread::spawn(move || {
            let _ = other.stats();
            let got = other.block(&key(1, 0), &ReadContext::new(), 1_000, 400, || {
                Ok(block(100))
            });
            done_tx
                .send(got.map(|held| held.is_some()).unwrap_or(false))
                .ok();
        });
        let went_through = done_rx.recv_timeout(WAIT);
        // the filler is released whatever happened, so that a failure does not hang the test
        release.send(()).ok();
        assert_eq!(
            went_through,
            Ok(true),
            "the index was locked while a block was read from the file"
        );
        assert!(filler
            .join()
            .expect("the filler")
            .expect("a block")
            .is_some());
    }

    #[test]
    fn a_block_that_two_readers_need_is_decoded_once() {
        let cache = Arc::new(Cache::new());
        let (filler, release, started) = decoding(&cache, key(0, 0), || Ok(block(100)));
        started.recv_timeout(WAIT).expect("the fill started");
        // the second reader needs the same block while the first is decoding it, and has a decode of its own that must not run
        let second_decodes = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let (cache, decodes) = (Arc::clone(&cache), Arc::clone(&second_decodes));
            thread::spawn(move || {
                cache.block(&key(0, 0), &ReadContext::new(), 1_000, 400, || {
                    decodes.fetch_add(1, Ordering::SeqCst);
                    Ok(block(100))
                })
            })
        };
        assert!(
            until(|| cache.waiting() == 1),
            "the second reader waits for the first"
        );
        release.send(()).ok();
        assert!(filler
            .join()
            .expect("the filler")
            .expect("a block")
            .is_some());
        assert!(waiter
            .join()
            .expect("the waiter")
            .expect("a block")
            .is_some());
        assert_eq!(
            second_decodes.load(Ordering::SeqCst),
            0,
            "the waiter decoded the block too"
        );
        // one miss (the filler's) and one hit (the waiter's, once the block was there)
        let stats = cache.stats();
        assert_eq!((stats.misses, stats.hits, stats.bytes_held), (1, 1, 400));
    }

    #[test]
    fn a_waiter_is_woken_when_the_fill_ends() {
        // a poll of a minute: only the notice that the fill ended can wake the waiter in time
        let cache = Arc::new(Cache::with_poll(Duration::from_secs(60)));
        let (filler, release, started) = decoding(&cache, key(0, 0), || Ok(block(100)));
        started.recv_timeout(WAIT).expect("the fill started");
        let (done_tx, done_rx) = mpsc::channel();
        let waiting = Arc::clone(&cache);
        thread::spawn(move || {
            let got = waiting.block(&key(0, 0), &ReadContext::new(), 1_000, 400, || {
                Ok(block(100))
            });
            done_tx
                .send(got.map(|held| held.is_some()).unwrap_or(false))
                .ok();
        });
        assert!(
            until(|| cache.waiting() == 1),
            "the reader waits for the fill"
        );
        release.send(()).ok();
        assert_eq!(
            done_rx.recv_timeout(WAIT),
            Ok(true),
            "the waiter was not woken when the fill ended"
        );
        filler.join().expect("the filler").expect("a block");
    }

    #[test]
    fn a_waiter_takes_over_a_fill_whose_reader_was_cancelled() {
        let cache = Arc::new(Cache::new());
        // the first reader is cancelled in its decode: it ends in `Cancelled` and its fill is withdrawn
        let (filler, release, started) =
            decoding(&cache, key(0, 0), || Err(Error::new(ErrorKind::Cancelled)));
        started.recv_timeout(WAIT).expect("the fill started");
        let takes_over = Arc::new(AtomicUsize::new(0));
        let (done_tx, done_rx) = mpsc::channel();
        {
            let (cache, decodes) = (Arc::clone(&cache), Arc::clone(&takes_over));
            thread::spawn(move || {
                let got = cache.block(&key(0, 0), &ReadContext::new(), 1_000, 400, || {
                    decodes.fetch_add(1, Ordering::SeqCst);
                    Ok(block(100))
                });
                done_tx
                    .send(got.map(|held| held.is_some()).unwrap_or(false))
                    .ok();
            });
        }
        assert!(
            until(|| cache.waiting() == 1),
            "the second reader waits for the first"
        );
        release.send(()).ok();
        let ended = filler.join().expect("the filler").expect_err("cancelled");
        assert!(matches!(ended.kind(), ErrorKind::Cancelled));
        assert_eq!(
            done_rx.recv_timeout(WAIT),
            Ok(true),
            "the waiter did not take the fill over: it was left marked as in progress"
        );
        assert_eq!(
            takes_over.load(Ordering::SeqCst),
            1,
            "the waiter decoded the block itself"
        );
        // and a third reader finds the block the waiter kept
        assert!(cache
            .block(&key(0, 0), &ReadContext::new(), 1_000, 400, || panic!(
                "no decode"
            ))
            .expect("a hit")
            .is_some());
    }

    #[test]
    fn a_reader_cancelled_before_its_decode_starts_leaves_no_fill_behind() {
        let cache = Arc::new(Cache::new());
        let token = CancelToken::new();
        token.cancel();
        let decodes = AtomicUsize::new(0);
        let got = cache.block(
            &key(0, 0),
            &ReadContext::new().with_cancel(token),
            1_000,
            400,
            || {
                decodes.fetch_add(1, Ordering::SeqCst);
                Ok(block(100))
            },
        );
        assert!(matches!(
            got.expect_err("cancelled").kind(),
            ErrorKind::Cancelled
        ));
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            0,
            "a cancelled reader decoded the block"
        );
        // no fill is left marked as in progress: the next reader fills the block at once, and nobody waits
        let next = cache
            .block(&key(0, 0), &ReadContext::new(), 1_000, 400, || {
                Ok(block(100))
            })
            .expect("the next reader");
        assert!(next.is_some());
        assert_eq!(cache.waiting(), 0);
        assert_eq!(cache.stats().bytes_held, 400);
    }

    #[test]
    fn a_filler_that_panics_hands_the_block_on_too() {
        let cache = Arc::new(Cache::new());
        let doomed = {
            let cache = Arc::clone(&cache);
            thread::spawn(move || {
                let _ = cache.block(&key(0, 0), &ReadContext::new(), 1_000, 400, || {
                    panic!("a decode that panics")
                });
            })
        };
        assert!(doomed.join().is_err(), "the reader panicked");
        // its fill was withdrawn as it unwound: the next reader fills the block, and the index is whole
        let got = cache
            .block(&key(0, 0), &ReadContext::new(), 1_000, 400, || {
                Ok(block(100))
            })
            .expect("a reader after the panic");
        assert!(got.is_some());
        assert_eq!(cache.stats().bytes_held, 400);
    }

    #[test]
    fn a_cancelled_waiter_returns_cancelled_and_the_fill_goes_on() {
        let cache = Arc::new(Cache::new());
        let (filler, release, started) = decoding(&cache, key(0, 0), || Ok(block(100)));
        started.recv_timeout(WAIT).expect("the fill started");
        let token = CancelToken::new();
        let (done_tx, done_rx) = mpsc::channel();
        {
            let (cache, token) = (Arc::clone(&cache), token.clone());
            thread::spawn(move || {
                let context = ReadContext::new().with_cancel(token);
                let got = cache.block(&key(0, 0), &context, 1_000, 400, || {
                    panic!("a waiter does not decode")
                });
                done_tx
                    .send(got.map(|_| ()).map_err(|e| e.kind().clone()))
                    .ok();
            });
        }
        assert!(
            until(|| cache.waiting() == 1),
            "the reader waits for the fill"
        );
        token.cancel();
        // the waiter notices within a poll, and the fill it was waiting for is not touched
        let ended = done_rx.recv_timeout(WAIT).expect("the waiter returned");
        assert_eq!(ended, Err(ErrorKind::Cancelled));
        assert_eq!(cache.waiting(), 0);
        release.send(()).ok();
        assert!(filler
            .join()
            .expect("the filler")
            .expect("a block")
            .is_some());
        // the block is kept, and a reader that comes later is served
        assert!(cache
            .block(&key(0, 0), &ReadContext::new(), 1_000, 400, || panic!(
                "no decode"
            ))
            .expect("a hit")
            .is_some());
        assert_eq!(cache.stats().bytes_held, 400);
    }
}
