//! Ranged reads of numeric datasets in blocks, with the cancel token and the progress sink of `ReadContext` (IP-03 phase 3F, task T03.6, decision D03-12).
//!
//! * **[`read_blocks`]** reads a [`Region`] of a dataset of `int16`, `int32`, `uint64`, `float32`, `float64` or `uint8` in one to three dimensions and **hands each block to
//!   its caller as it is read**, so that the caller can convert it, place it in its output or keep it in a cache without a second pass. A block is a run of whole
//!   rows (the extent along the first dimension), in row-major order, owned by the caller.
//! * **A block is a multiple of the dataset's chunk, and is never smaller than one chunk** (D03-12): a chunk is decoded as a whole, so a read of less than a chunk
//!   would decode it again for the next block. Where the dataset is not chunked a block holds at most [`CANCEL_CHECK_INTERVAL`] values (and at least one row).
//!   The blocks are aligned to multiples of the block size counted from the first row of the dataset, so that a read that starts in the middle of a block ends its
//!   first block at the next boundary, and every later block is a whole one (see [`block_rows`]). **A dataset with one chunk for all its values (V16, V17 March) is
//!   one block**: it is read correctly and not specially treated here; what to do about its cost is decision G17 (IP-06).
//! * **The token is checked before each block and progress is reported after it** (`values done`, `values in all`), in the order the blocks are read. A cancelled
//!   read returns `Cancelled`; the blocks already handed over stay with the caller, which discards them if it wants no partial result.
//! * **Nothing is converted.** The type that is asked for must be the type that is stored (a different byte order is the same type): a `float32` dataset read as
//!   `float64`, or an `int32` read as `uint64`, is refused and not widened or narrowed (BL-ST-07, D03-3). The library decides what a value means, and not this module.
//! * **A range outside the dataset is `InvalidIndex`**, said before any block is read, and an empty range delivers no block.
//!
//! No group, dataset or attribute name of any version appears here (D03-11). A dataset of more than three dimensions, or of none, is `UnsupportedInVersion`.

use std::ops::Range;

use build_log_model::{Error, ErrorKind, ReadContext, CANCEL_CHECK_INTERVAL};
use hdf5::dataset::Dataset;
use hdf5::{H5Type, Hyperslab, Selection, SliceOrIndex};

use super::error::{from_binding, unsupported};

mod sealed {
    pub trait Sealed {}
    impl Sealed for i16 {}
    impl Sealed for i32 {}
    impl Sealed for u64 {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
    impl Sealed for u8 {}
}

/// A type that a numeric dataset is read as. Only the six that the files hold; another is added here, with its test, when a file needs it (`int16` was, in IP-06 phase 6I:
/// tauri's XY2-100E writes its commanded positions as `int16`). **The column engine keeps the list of what it reads, once** (`columns::can_read`), and adding a type is a variant there.
pub trait Element: H5Type + Copy + sealed::Sealed {}
impl Element for i16 {}
impl Element for i32 {}
impl Element for u64 {}
impl Element for f32 {}
impl Element for f64 {}
impl Element for u8 {}

/// Which values of a dataset to read: a range of the values along each dimension, starting with the first.
///
/// A dimension that has no range is read whole, so the region of a one-dimensional dataset is one range, and the region of an image can be a range of rows
/// alone or a rectangle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Region {
    ranges: Vec<Range<usize>>,
}

impl Region {
    /// Every value of the dataset.
    pub fn whole() -> Self {
        Region { ranges: Vec::new() }
    }

    /// The values `range` along the first dimension, and everything along the others.
    pub fn rows(range: Range<usize>) -> Self {
        Region {
            ranges: vec![range],
        }
    }

    /// A range along each of the first dimensions, in order; a dimension that has none is read whole.
    pub fn new(ranges: Vec<Range<usize>>) -> Self {
        Region { ranges }
    }
}

/// A run of whole rows of a region, read from the file and now the caller's.
#[derive(Debug, Clone, PartialEq)]
pub struct Block<T> {
    /// The index, along the first dimension of the dataset, of the first row of this block.
    pub first: usize,
    /// How many rows the block holds.
    pub rows: usize,
    /// The values, row-major: the rows, then the selected extents of the other dimensions.
    pub values: Vec<T>,
}

/// The dimensions a dataset may have to be read here.
const MAX_DIMENSIONS: usize = 3;

/// The extents of the dataset that a region selects, one for each dimension, as a range of the values along it.
fn extents(dataset: &Dataset, region: &Region) -> Result<Vec<Range<usize>>, Error> {
    let shape = dataset.shape();
    if shape.is_empty() || shape.len() > MAX_DIMENSIONS {
        return Err(unsupported(format!(
            "a numeric dataset of {} dimensions (one to {MAX_DIMENSIONS} are read)",
            shape.len()
        )));
    }
    if region.ranges.len() > shape.len() {
        return Err(Error::new(ErrorKind::InvalidIndex {
            what: format!(
                "a region of {} ranges for a dataset of {} dimensions",
                region.ranges.len(),
                shape.len()
            ),
        }));
    }
    shape
        .iter()
        .enumerate()
        .map(|(dimension, &length)| {
            let range = region
                .ranges
                .get(dimension)
                .cloned()
                .unwrap_or(0..length);
            if range.start > range.end || range.end > length {
                Err(Error::new(ErrorKind::InvalidIndex {
                    what: format!(
                        "the values {}..{} along dimension {dimension} of a dataset whose extent there is {length}",
                        range.start, range.end
                    ),
                }))
            } else {
                Ok(range)
            }
        })
        .collect()
}

/// How many values one row of the region holds: the product of the selected extents of the dimensions after the first.
fn row_values(extents: &[Range<usize>]) -> usize {
    extents[1..]
        .iter()
        .fold(1usize, |all, e| all.saturating_mul(e.len()))
}

/// The shape of what a region selects: its extent along each dimension of the dataset. Fails as a read would for a region outside the dataset or a dataset that
/// is not read.
pub fn region_shape(dataset: &Dataset, region: &Region) -> Result<Vec<usize>, Error> {
    Ok(extents(dataset, region)?.iter().map(|e| e.len()).collect())
}

/// How many rows a block holds when this region of this dataset is read (decision D03-12).
///
/// A multiple of the chunk's extent along the first dimension, at least one chunk, and as many as keep the values of a block at or below
/// [`CANCEL_CHECK_INTERVAL`]; where the dataset is not chunked, as many rows as keep the values of a block at or below it, and at least one row.
pub fn block_rows(dataset: &Dataset, region: &Region) -> Result<usize, Error> {
    let selected = extents(dataset, region)?;
    Ok(block_rows_of(
        dataset.chunk().as_deref().and_then(|c| c.first().copied()),
        row_values(&selected),
    ))
}

fn block_rows_of(chunk_rows: Option<usize>, row_values: usize) -> usize {
    let row_values = row_values.max(1);
    match chunk_rows {
        Some(chunk_rows) if chunk_rows > 0 => {
            let chunks = (CANCEL_CHECK_INTERVAL / chunk_rows.saturating_mul(row_values)).max(1);
            chunk_rows.saturating_mul(chunks)
        }
        _ => (CANCEL_CHECK_INTERVAL / row_values).max(1),
    }
}

/// The stored type must be the type asked for: nothing is converted.
fn check_type<T: Element>(dataset: &Dataset) -> Result<(), Error> {
    let stored = dataset
        .dtype()
        .and_then(|d| d.to_descriptor())
        .map_err(|e| unsupported(format!("a dataset of a datatype that is not read ({e})")))?;
    let wanted = T::type_descriptor();
    if stored == wanted {
        Ok(())
    } else {
        Err(unsupported(format!(
            "a dataset of {stored:?} read as {wanted:?} (the primitives never convert numbers)"
        )))
    }
}

fn slice(range: &Range<usize>) -> SliceOrIndex {
    SliceOrIndex::SliceTo {
        start: range.start,
        step: 1,
        end: range.end,
        block: 1,
    }
}

fn selection(parts: Vec<SliceOrIndex>) -> Selection {
    Hyperslab::from(parts).into()
}

/// The rows `rows` of the selected region, row-major.
fn read_rows<T: Element>(
    dataset: &Dataset,
    selected: &[Range<usize>],
    rows: Range<usize>,
) -> Result<Vec<T>, Error> {
    let what = "reading a block of a dataset";
    let whole = rows.len() == dataset.shape()[0]
        && selected
            .iter()
            .zip(dataset.shape())
            .all(|(range, length)| range.start == 0 && range.end == length);
    if whole {
        // the one block that is the whole dataset (one chunk for all its values) is read straight into its vector, with no copy
        return dataset.read_raw::<T>().map_err(|e| from_binding(what, e));
    }
    match selected.len() {
        1 => Ok(dataset
            .read_slice_1d::<T, _>(selection(vec![slice(&rows)]))
            .map_err(|e| from_binding(what, e))?
            .to_vec()),
        2 => Ok(dataset
            .read_slice_2d::<T, _>(selection(vec![slice(&rows), slice(&selected[1])]))
            .map_err(|e| from_binding(what, e))?
            .iter()
            .copied()
            .collect()),
        _ => {
            // three dimensions: one plane for each row, so that the binding's two-dimensional read serves it
            let mut values = Vec::with_capacity(
                rows.len()
                    .saturating_mul(selected[1].len())
                    .saturating_mul(selected[2].len()),
            );
            for row in rows {
                let plane = dataset
                    .read_slice_2d::<T, _>(selection(vec![
                        SliceOrIndex::Index(row),
                        slice(&selected[1]),
                        slice(&selected[2]),
                    ]))
                    .map_err(|e| from_binding(what, e))?;
                values.extend(plane.iter().copied());
            }
            Ok(values)
        }
    }
}

/// Reads `region` of `dataset` as `T`, block by block, in order, handing each block to `sink`.
///
/// `context` is checked before each block (`Cancelled` if its token has fired) and told how many values are done of how many after each. An error from `sink`
/// stops the read and is returned. A region outside the dataset, a dataset that is not read, or a type that is not the stored type is an error before any block.
pub fn read_blocks<T: Element>(
    dataset: &Dataset,
    region: &Region,
    context: &ReadContext,
    mut sink: impl FnMut(Block<T>) -> Result<(), Error>,
) -> Result<(), Error> {
    check_type::<T>(dataset)?;
    let selected = extents(dataset, region)?;
    let per_row = row_values(&selected);
    let size = block_rows_of(
        dataset.chunk().as_deref().and_then(|c| c.first().copied()),
        per_row,
    );
    let rows = &selected[0];
    let total = rows.len().saturating_mul(per_row) as u64;
    let mut done = 0u64;
    let mut next = rows.start;
    while next < rows.end {
        context.check_cancelled()?;
        // blocks end on multiples of the block size, counted from the first row of the dataset
        let end = ((next / size + 1).saturating_mul(size)).min(rows.end);
        let values = read_rows::<T>(dataset, &selected, next..end)?;
        done += values.len() as u64;
        sink(Block {
            first: next,
            rows: end - next,
            values,
        })?;
        context.report(done, total);
        next = end;
    }
    Ok(())
}
