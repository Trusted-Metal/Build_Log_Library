//! The walk over the blocks of a range, with the cache (IP-06 phases 6B and 6C, decisions D06-4 to D06-7).
//!
//! A range of a column is read **one block at a time**, in order. For each block the walk looks in the cache first (**a hit is served whatever the budget**), and on a miss
//! decodes the block. **If the block fits the budget of the read it is decoded whole and kept**, and the rows that were asked for are a slice of it; **if it does not, only the
//! rows that were asked for are decoded** and nothing is kept, so a read with no budget decodes exactly what it returns. The token is checked before each block and progress
//! is reported after it, so a cached read and one that is not cached report alike.
//!
//! The values come out in one of three ways: **as stored** (the first block is the start of the result, and a range inside one cached block is that block's buffer, shared, with
//! no copy), **converted** (each block is converted as it arrives into a result of the converted type), or **as stored but of one block that was not kept** (its vector).
//! A conversion is made from the native block and is never kept (decision D06-5).

use std::ops::Range;

use build_log_model::{ColumnData, Error, Precision, ReadContext, Shared};
use hdf5::dataset::Dataset;

use super::cache::{Cache, Key};
use super::convert::{reserve, Native};
use crate::hdf5::{read_blocks, Region};

/// What the walk reads from: the dataset, the cache if there is one, and the key of the column (its block is set for each block).
pub(super) struct Source<'a> {
    pub dataset: &'a Dataset,
    pub cache: Option<&'a Cache>,
    pub key: Key,
    /// The values of one block, and the number of values of the column.
    pub size: usize,
    pub length: usize,
}

/// The values of one block that a read needs.
enum Part<T> {
    /// A slice of a block that the cache holds (or has just been given).
    Cached(Shared<T>),
    /// Values decoded for this read alone.
    Fresh(Vec<T>),
}

impl<T> Part<T> {
    fn as_slice(&self) -> &[T] {
        match self {
            Part::Cached(shared) => shared.as_slice(),
            Part::Fresh(values) => values,
        }
    }
}

/// The values `rows` of the dataset, read in one block (`rows` lies inside one block of the walk, so the reader's blocks are one).
fn decode<T: Native>(dataset: &Dataset, rows: Range<usize>) -> Result<Vec<T>, Error> {
    let mut values: Vec<T> = Vec::new();
    // the block is read whole and by itself: the context here is the one that never cancels and reports nowhere, because the walk checks the token and reports the progress
    read_blocks::<T>(dataset, &Region::rows(rows), &ReadContext::new(), |block| {
        if values.is_empty() {
            values = block.values;
        } else {
            values.extend(block.values);
        }
        Ok(())
    })?;
    Ok(values)
}

/// The rows `need` of the block `block`, which holds the rows `block_rows` of the column.
///
/// With a cache the block comes from it, **whoever decodes it** (decision D06-8): a hit, or the block that another reader is decoding and this one waited for, or, if nobody has it
/// and it fits the budget, this reader's own decode, which it keeps for the others. A block that does not fit the budget is not kept, and then only the rows needed are decoded.
fn part<T: Native>(
    source: &mut Source<'_>,
    block: usize,
    block_rows: Range<usize>,
    need: Range<usize>,
    context: &ReadContext,
) -> Result<Part<T>, Error> {
    let inside = need.start - block_rows.start..need.end - block_rows.start;
    if let Some(cache) = source.cache {
        source.key.block = block;
        let bytes = (block_rows.len() * std::mem::size_of::<T>()) as u64;
        let dataset = source.dataset;
        let whole = block_rows.clone();
        let held = cache.block(&source.key, context, context.cache_budget(), bytes, || {
            Ok(T::hold(Shared::new(decode::<T>(dataset, whole)?)))
        })?;
        if let Some(shared) = held.as_ref().and_then(T::held) {
            return Ok(Part::Cached(
                shared.slice(inside).expect("the rows are inside the block"),
            ));
        }
    }
    Ok(Part::Fresh(decode::<T>(source.dataset, need)?))
}

/// Walks the blocks of `range` in order, handing each block's rows to `take`.
fn walk<T: Native>(
    source: &mut Source<'_>,
    range: Range<usize>,
    context: &ReadContext,
    mut take: impl FnMut(Part<T>) -> Result<(), Error>,
) -> Result<(), Error> {
    let total = range.len() as u64;
    let (size, length) = (source.size, source.length);
    let mut done = 0u64;
    let mut at = range.start;
    while at < range.end {
        context.check_cancelled()?;
        let block = at / size;
        let block_rows = block * size..((block + 1) * size).min(length);
        let upto = block_rows.end.min(range.end);
        take(part::<T>(source, block, block_rows, at..upto, context)?)?;
        done += (upto - at) as u64;
        context.report(done, total);
        at = upto;
    }
    Ok(())
}

/// What a read has gathered of the blocks so far, as stored.
enum Gathered<T> {
    Nothing,
    One(Part<T>),
    Many(Vec<T>),
}

impl<T: Native> Gathered<T> {
    /// Adds the rows of the next block. The first block is kept as it is, so that a range inside one block is not copied; a second block starts the result's own vector,
    /// from the first block's if it was decoded for this read alone.
    fn push(&mut self, next: Part<T>, total: usize) -> Result<(), Error> {
        *self = match std::mem::replace(self, Gathered::Nothing) {
            Gathered::Nothing => Gathered::One(next),
            Gathered::One(first) => {
                let mut values = match first {
                    Part::Fresh(values) => values,
                    Part::Cached(shared) => {
                        let mut values = Vec::new();
                        reserve(&mut values, total)?;
                        values.extend_from_slice(shared.as_slice());
                        values
                    }
                };
                let more = total.saturating_sub(values.len());
                reserve(&mut values, more)?;
                values.extend_from_slice(next.as_slice());
                Gathered::Many(values)
            }
            Gathered::Many(mut values) => {
                values.extend_from_slice(next.as_slice());
                Gathered::Many(values)
            }
        };
        Ok(())
    }

    fn finish(self) -> ColumnData {
        match self {
            Gathered::Nothing => T::column(Vec::new()),
            Gathered::One(Part::Cached(shared)) => T::shared(shared),
            Gathered::One(Part::Fresh(values)) | Gathered::Many(values) => T::column(values),
        }
    }
}

/// As stored: the values of `range`, at the precision they are stored in.
fn gather_native<T: Native>(
    source: &mut Source<'_>,
    range: Range<usize>,
    context: &ReadContext,
) -> Result<ColumnData, Error> {
    let total = range.len();
    let mut gathered = Gathered::<T>::Nothing;
    walk::<T>(source, range, context, |next| gathered.push(next, total))?;
    Ok(gathered.finish())
}

/// Each block converted as it arrives into a result of the converted type.
pub(super) fn gather_converted<T: Native, O>(
    source: &mut Source<'_>,
    range: Range<usize>,
    context: &ReadContext,
    convert: fn(T) -> O,
    wrap: fn(Vec<O>) -> ColumnData,
) -> Result<ColumnData, Error> {
    let mut values: Vec<O> = Vec::new();
    reserve(&mut values, range.len())?;
    walk::<T>(source, range, context, |next| {
        values.extend(next.as_slice().iter().copied().map(convert));
        Ok(())
    })?;
    Ok(wrap(values))
}

/// Reads the values `range` of a column stored as `T`, at `precision`.
pub(super) fn collect<T: Native>(
    source: &mut Source<'_>,
    range: Range<usize>,
    precision: Precision,
    context: &ReadContext,
) -> Result<ColumnData, Error> {
    match precision {
        Precision::F32 if !T::IS_F32 => {
            gather_converted::<T, f32>(source, range, context, T::to_f32, |v| {
                ColumnData::F32(Shared::new(v))
            })
        }
        Precision::F64 if !T::IS_F64 => {
            gather_converted::<T, f64>(source, range, context, T::to_f64, |v| {
                ColumnData::F64(Shared::new(v))
            })
        }
        // `Native`, a precision that is the stored one, and any that a later version of the Model adds and this engine does not know: what is stored
        _ => gather_native::<T>(source, range, context),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use build_log_model::{ChannelKey, LayerId};

    use super::*;

    /// How many values `counting` has converted: only the test below uses it.
    static CONVERTED: AtomicUsize = AtomicUsize::new(0);

    fn counting(value: i32) -> f64 {
        CONVERTED.fetch_add(1, Ordering::SeqCst);
        f64::from(value)
    }

    #[test]
    fn each_block_is_converted_as_it_arrives_and_not_after_the_whole_read() {
        // the progress sink is told after each block, and by then that block, and no later one, has been converted
        let path =
            std::env::temp_dir().join(format!("build-log-io-blocks-{}.h5", std::process::id()));
        let file = hdf5::File::create(&path).expect("a file");
        let values: Vec<i32> = (0..25_000).collect();
        let dataset = file
            .new_dataset::<i32>()
            .shape(values.len())
            .chunk(10_000)
            .create("x")
            .expect("a dataset");
        dataset.write_raw(&values).expect("its values");

        let converted_when_told: Arc<Mutex<Vec<usize>>> = Arc::default();
        let sink = Arc::clone(&converted_when_told);
        let context = ReadContext::new().with_progress(Arc::new(move |_done, _total| {
            sink.lock()
                .expect("a lock")
                .push(CONVERTED.load(Ordering::SeqCst));
        }));
        let mut source = Source {
            dataset: &dataset,
            cache: None,
            key: Key {
                layer: LayerId(1),
                laser: None,
                channel: ChannelKey::new(["x"]),
                block: 0,
                generation: 0,
            },
            size: 10_000,
            length: values.len(),
        };
        let column =
            gather_converted::<i32, f64>(&mut source, 0..values.len(), &context, counting, |v| {
                ColumnData::F64(Shared::new(v))
            })
            .expect("a read");
        drop(file);
        std::fs::remove_file(&path).ok();
        assert_eq!(column.len(), 25_000);
        assert_eq!(
            *converted_when_told.lock().expect("a lock"),
            [10_000, 20_000, 25_000]
        );
    }
}
