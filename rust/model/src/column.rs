//! Column access: the requests, the typed, shared results, cancellation and progress, the `ColumnSource` trait and the in-memory source. Bulk data is reached only through here (decision D02-13).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** caller-options (`ColumnRequest`, `ReadContext`); library (`Shared`, `CancelToken`, `Revision`, `InMemorySource`); the enums `ColumnData`, `ColumnRange`, `Precision`; the traits `ColumnSource`, `BuildSource` and `ProgressSink`. `Shared` and `ColumnData` are the types that hold bulk data, and say so with `// bulk-container:`.
//! * **Requirement:** BL-CD-03, BL-FA-02, BL-FA-08.
//!
//! # What this module claims, and what it does not
//!
//! * **Bulk data is never part of the metadata.** A descriptor says what a column is; a [`ColumnSource`] hands its values over when asked, for the range
//!   asked, and nothing else holds them (BL-CD-03).
//! * **A read is synchronous and can be stopped** (decision G7). The caller passes a [`ReadContext`] with a [`CancelToken`] and an optional
//!   [`ProgressSink`]. A source that does long work checks the token and reports progress at least every [`CANCEL_CHECK_INTERVAL`] values; a caller that does
//!   not care passes [`ReadContext::new`], whose token is never cancelled. The job layer (IP-15) is built on these signatures and does not change them.
//! * **A result is typed, immutable and shared.** [`ColumnData`] holds a [`Shared`] buffer: a slice of it points into the same allocation, so a cached
//!   column can serve several callers without copying.
//! * **A request names a range of indices or a range of time.** A time range is resolved through the segment table of the laser in that layer, which is
//!   itself a column (BL-CD-10): the samples inside the range, with the gaps between segments skipped.
//! * **Precision is the caller's choice and is never silent.** `Native` returns what is stored. `F32` and `F64` convert numbers as asked, rounding to the
//!   nearest where a value cannot be exact; a caller that needs the exact stored value asks for `Native`.
//! * **Discovery reads no samples.** [`BuildSource`] says what a build holds (its metadata, layers, lasers, channels) and carries a [`Revision`] that changes when a
//!   live source grows. Every file adapter and the in-memory source implement it, and it is part of the read surface that was frozen after Vision's trial read V16 and V17
//!   through it (decision D24-20): an additive change is logged, and a change to what it already offers needs a recorded decision.
//! * **The in-memory source** serves columns that are already in memory, in blocks, with the same checks and reports as a file-backed source would make.
//!   It is how the traits are proved without HDF5, and how tests and non-file sources work (BL-FA-01).

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::availability::{availability_of, AvailabilityReport};
use crate::channel::ChannelDescriptor;
use crate::error::{Error, ErrorKind, Location};
use crate::event_columns::{EventColumns, EventPlace, EventRequest};
use crate::events::EventKind;
use crate::grids::{GridData, GridRequest};
use crate::identity::{ChannelKey, LaserKey, LayerId};
use crate::image_regions::{ImageData, ImageRegion, ImageRequest};
use crate::meta::BuildMeta;
use crate::time::{index_range, Segment, TimeValue};
use crate::validation::{validate_build, ValidationReport};

/// A long read checks its cancel token, and reports its progress, at least this often: every 10,000 values.
pub const CANCEL_CHECK_INTERVAL: usize = 10_000;

// ---------------------------------------------------------------- the shared buffer and the typed column

/// A run of values in a buffer that many readers share. A slice of it points into the same allocation: nothing is copied.
///
/// It is immutable, cheap to clone, and `Send + Sync` when the values are.
// constructed-by: library
// bulk-container: the shared, immutable buffer that holds the values of one column
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Shared<T> {
    data: Arc<[T]>,
    start: usize,
    len: usize,
}

impl<T> Shared<T> {
    pub fn new(values: Vec<T>) -> Self {
        let len = values.len();
        Shared {
            data: values.into(),
            start: 0,
            len,
        }
    }

    pub fn as_slice(&self) -> &[T] {
        &self.data[self.start..self.start + self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The values `range` of this run, in the same allocation. `None` if the range is outside it.
    pub fn slice(&self, range: Range<usize>) -> Option<Shared<T>> {
        (range.start <= range.end && range.end <= self.len).then(|| Shared {
            data: Arc::clone(&self.data),
            start: self.start + range.start,
            len: range.end - range.start,
        })
    }

    /// How many `Shared` values hold this allocation (this one included).
    pub fn holders(&self) -> usize {
        Arc::strong_count(&self.data)
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

/// The values of a column, typed. Which variant a source returns follows the stored type, unless a precision was asked for.
// bulk-container: the typed values of one column read
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ColumnData {
    I8(Shared<i8>),
    I16(Shared<i16>),
    I32(Shared<i32>),
    I64(Shared<i64>),
    U8(Shared<u8>),
    U16(Shared<u16>),
    U32(Shared<u32>),
    U64(Shared<u64>),
    F32(Shared<f32>),
    F64(Shared<f64>),
    Bool(Shared<bool>),
    Text(Shared<String>),
    /// The rows of a segment table (BL-CD-10): a column like any other.
    Segments(Shared<Segment>),
}

/// Runs `$body` on the buffer inside whichever variant `$data` is.
macro_rules! inspect {
    ($data:expr, $v:ident => $body:expr) => {
        match $data {
            ColumnData::I8($v) => $body,
            ColumnData::I16($v) => $body,
            ColumnData::I32($v) => $body,
            ColumnData::I64($v) => $body,
            ColumnData::U8($v) => $body,
            ColumnData::U16($v) => $body,
            ColumnData::U32($v) => $body,
            ColumnData::U64($v) => $body,
            ColumnData::F32($v) => $body,
            ColumnData::F64($v) => $body,
            ColumnData::Bool($v) => $body,
            ColumnData::Text($v) => $body,
            ColumnData::Segments($v) => $body,
        }
    };
}

/// Runs `$body` on the buffer inside whichever variant `$data` is, and wraps what it returns in the same variant.
macro_rules! rewrap {
    ($data:expr, $v:ident => $body:expr) => {
        match $data {
            ColumnData::I8($v) => ColumnData::I8($body),
            ColumnData::I16($v) => ColumnData::I16($body),
            ColumnData::I32($v) => ColumnData::I32($body),
            ColumnData::I64($v) => ColumnData::I64($body),
            ColumnData::U8($v) => ColumnData::U8($body),
            ColumnData::U16($v) => ColumnData::U16($body),
            ColumnData::U32($v) => ColumnData::U32($body),
            ColumnData::U64($v) => ColumnData::U64($body),
            ColumnData::F32($v) => ColumnData::F32($body),
            ColumnData::F64($v) => ColumnData::F64($body),
            ColumnData::Bool($v) => ColumnData::Bool($body),
            ColumnData::Text($v) => ColumnData::Text($body),
            ColumnData::Segments($v) => ColumnData::Segments($body),
        }
    };
}

impl ColumnData {
    /// How many values the column holds.
    pub fn len(&self) -> usize {
        inspect!(self, v => v.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The values `range`, sharing this column's allocation. `None` if the range is outside the column.
    pub fn slice(&self, range: Range<usize>) -> Option<ColumnData> {
        Some(rewrap!(self, v => v.slice(range.clone())?))
    }

    /// The values of `ranges`, in order, as a new buffer (a copy): the rows of a rectangle of a frame are such ranges. `None` if a range is outside the column.
    pub(crate) fn gather(&self, ranges: &[Range<usize>]) -> Option<ColumnData> {
        Some(rewrap!(self, v => {
            let mut out = Vec::with_capacity(ranges.iter().map(|r| r.len()).sum());
            for range in ranges {
                out.extend_from_slice(v.as_slice().get(range.clone())?);
            }
            Shared::new(out)
        }))
    }

    pub fn as_i32(&self) -> Option<&[i32]> {
        match self {
            ColumnData::I32(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<&[f32]> {
        match self {
            ColumnData::F32(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<&[f64]> {
        match self {
            ColumnData::F64(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_segments(&self) -> Option<&[Segment]> {
        match self {
            ColumnData::Segments(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// A copy of the values `range`, made in blocks of [`CANCEL_CHECK_INTERVAL`]: the token is checked before each block and the progress is reported
    /// after it, as a file-backed source does while it decodes. The range must be inside the column.
    fn copy_range(&self, range: Range<usize>, context: &ReadContext) -> Result<ColumnData, Error> {
        Ok(rewrap!(self, v => copy_in_blocks(&v.as_slice()[range.clone()], context)?))
    }

    /// The column at the precision asked for. `Native` and a number that is already of that type are returned as they are, sharing their buffer. Any other
    /// number is converted, rounded to the nearest where it cannot be exact. A column that is not numbers has no precision, and asking for one is an error.
    pub fn with_precision(self, precision: Precision) -> Result<ColumnData, Error> {
        fn convert<T, U>(v: &Shared<T>, f: impl Fn(&T) -> U) -> Shared<U> {
            Shared::new(v.as_slice().iter().map(f).collect())
        }
        let not_numeric = || {
            Error::new(ErrorKind::InvalidIndex {
                what: "a precision was asked for a column that is not numbers".to_string(),
            })
        };
        Ok(match (precision, &self) {
            (Precision::Native, _)
            | (Precision::F32, ColumnData::F32(_))
            | (Precision::F64, ColumnData::F64(_)) => self,
            (Precision::F32, ColumnData::I8(v)) => ColumnData::F32(convert(v, |x| f32::from(*x))),
            (Precision::F32, ColumnData::I16(v)) => ColumnData::F32(convert(v, |x| f32::from(*x))),
            (Precision::F32, ColumnData::I32(v)) => ColumnData::F32(convert(v, |x| *x as f32)),
            (Precision::F32, ColumnData::I64(v)) => ColumnData::F32(convert(v, |x| *x as f32)),
            (Precision::F32, ColumnData::U8(v)) => ColumnData::F32(convert(v, |x| f32::from(*x))),
            (Precision::F32, ColumnData::U16(v)) => ColumnData::F32(convert(v, |x| f32::from(*x))),
            (Precision::F32, ColumnData::U32(v)) => ColumnData::F32(convert(v, |x| *x as f32)),
            (Precision::F32, ColumnData::U64(v)) => ColumnData::F32(convert(v, |x| *x as f32)),
            (Precision::F32, ColumnData::F64(v)) => ColumnData::F32(convert(v, |x| *x as f32)),
            (Precision::F64, ColumnData::I8(v)) => ColumnData::F64(convert(v, |x| f64::from(*x))),
            (Precision::F64, ColumnData::I16(v)) => ColumnData::F64(convert(v, |x| f64::from(*x))),
            (Precision::F64, ColumnData::I32(v)) => ColumnData::F64(convert(v, |x| f64::from(*x))),
            (Precision::F64, ColumnData::I64(v)) => ColumnData::F64(convert(v, |x| *x as f64)),
            (Precision::F64, ColumnData::U8(v)) => ColumnData::F64(convert(v, |x| f64::from(*x))),
            (Precision::F64, ColumnData::U16(v)) => ColumnData::F64(convert(v, |x| f64::from(*x))),
            (Precision::F64, ColumnData::U32(v)) => ColumnData::F64(convert(v, |x| f64::from(*x))),
            (Precision::F64, ColumnData::U64(v)) => ColumnData::F64(convert(v, |x| *x as f64)),
            (Precision::F64, ColumnData::F32(v)) => ColumnData::F64(convert(v, |x| f64::from(*x))),
            _ => return Err(not_numeric()),
        })
    }
}

/// Copies `source` in blocks, checking the token before each and reporting progress after each.
fn copy_in_blocks<T: Clone>(source: &[T], context: &ReadContext) -> Result<Shared<T>, Error> {
    let total = source.len() as u64;
    let mut out = Vec::with_capacity(source.len());
    for block in source.chunks(CANCEL_CHECK_INTERVAL) {
        context.check_cancelled()?;
        out.extend_from_slice(block);
        context.report(out.len() as u64, total);
    }
    Ok(Shared::new(out))
}

// ---------------------------------------------------------------- the request

/// The numbers a caller wants a column in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Precision {
    /// As stored.
    #[default]
    Native,
    F32,
    F64,
}

/// Which values of a column a request wants.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum ColumnRange {
    /// Every value.
    #[default]
    All,
    /// The values at these sample indices.
    Indices(Range<u64>),
    /// The values whose time lies in this range, both ends included, in the clock of the segment table. The time range is resolved through that table,
    /// and the gaps between segments have no values.
    Time { from: TimeValue, to: TimeValue },
}

/// A request for one column: the layer, the laser (none for a build-level channel such as a sensor), the channel, the range and the precision.
// constructed-by: caller-options
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ColumnRequest {
    pub layer: LayerId,
    pub laser: Option<LaserKey>,
    pub channel: ChannelKey,
    pub range: ColumnRange,
    pub precision: Precision,
}

impl ColumnRequest {
    /// A request for the whole column, as stored.
    pub fn new(layer: LayerId, laser: Option<LaserKey>, channel: ChannelKey) -> Self {
        ColumnRequest {
            layer,
            laser,
            channel,
            range: ColumnRange::All,
            precision: Precision::Native,
        }
    }

    pub fn with_range(mut self, range: ColumnRange) -> Self {
        self.range = range;
        self
    }

    pub fn with_precision(mut self, precision: Precision) -> Self {
        self.precision = precision;
        self
    }

    /// Where this request points, for an error that must say which column it was about: the layer, the laser if there is one, the channel and the operation `read`.
    pub fn location(&self) -> Location {
        let mut location = Location::new()
            .with_layer(self.layer)
            .with_channel(self.channel.clone())
            .with_operation("read");
        if let Some(laser) = &self.laser {
            location = location.with_laser(laser.clone());
        }
        location
    }
}

// ---------------------------------------------------------------- cancellation and progress

/// A way for a caller to stop a long read. Clones share one flag, so the caller keeps one and passes the other.
///
/// [`CancelToken::new`] makes a token that can be cancelled; [`CancelToken::never`] (also the default) makes one that cannot, for a caller that does not
/// care.
// constructed-by: library
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CancelToken {
    flag: Option<Arc<AtomicBool>>,
}

impl CancelToken {
    /// A token that can be cancelled, by `cancel` on it or on any clone of it.
    pub fn new() -> Self {
        CancelToken {
            flag: Some(Arc::new(AtomicBool::new(false))),
        }
    }

    /// A token that is never cancelled, and that `cancel` does nothing to.
    pub fn never() -> Self {
        CancelToken { flag: None }
    }

    pub fn cancel(&self) {
        if let Some(flag) = &self.flag {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        CancelToken::never()
    }
}

/// Where a long read reports how far it has come: `done` values of `total`. It is called from the thread that reads, and must be quick.
///
/// Any `Fn(u64, u64)` that is `Send + Sync` is a sink.
pub trait ProgressSink: Send + Sync {
    /// Reports that `done` of `total` values have been read so far.
    fn report(&self, done: u64, total: u64);
}

impl<F> ProgressSink for F
where
    F: Fn(u64, u64) + Send + Sync,
{
    fn report(&self, done: u64, total: u64) {
        self(done, total)
    }
}

/// What a read is given besides the request: the token that can stop it, where to report progress, and how much memory the source may keep for later reads.
// constructed-by: caller-options
#[derive(Clone)]
#[non_exhaustive]
pub struct ReadContext {
    pub cancel: CancelToken,
    pub progress: Option<Arc<dyn ProgressSink>>,
    cache_budget: u64,
}

impl ReadContext {
    /// A context that never cancels, reports nowhere and **lets the source keep nothing**: a library that is not told how much memory it may hold holds none (decision
    /// D06-6, PS-07).
    pub fn new() -> Self {
        ReadContext {
            cancel: CancelToken::never(),
            progress: None,
            cache_budget: 0,
        }
    }

    /// How many bytes of decoded columns the source may keep, after this read, for later reads (decisions G17 and D06-6). **It bounds what is retained and not what is
    /// returned**: a read is served in full whatever the budget. A source that keeps nothing (memory, or one that has no cache) ignores it. A source applies the budget of
    /// each read as that read finishes, so readers that share a source should share a budget.
    pub fn with_cache_budget(mut self, bytes: u64) -> Self {
        self.cache_budget = bytes;
        self
    }

    /// The budget of this read: `0`, the default, keeps nothing.
    pub fn cache_budget(&self) -> u64 {
        self.cache_budget
    }

    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn with_progress(mut self, progress: Arc<dyn ProgressSink>) -> Self {
        self.progress = Some(progress);
        self
    }

    /// `Err(Cancelled)` if the token has been cancelled. A source calls this at least every [`CANCEL_CHECK_INTERVAL`] values.
    pub fn check_cancelled(&self) -> Result<(), Error> {
        if self.cancel.is_cancelled() {
            Err(Error::new(ErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    /// Reports progress to the sink, if there is one.
    pub fn report(&self, done: u64, total: u64) {
        if let Some(sink) = &self.progress {
            sink.report(done, total);
        }
    }
}

impl Default for ReadContext {
    fn default() -> Self {
        ReadContext::new()
    }
}

impl fmt::Debug for ReadContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadContext")
            .field("cancel", &self.cancel)
            .field("progress", &self.progress.as_ref().map(|_| "a sink"))
            .field("cache_budget", &self.cache_budget)
            .finish()
    }
}

// ---------------------------------------------------------------- the trait and the in-memory source

/// Something that serves columns: a file, a live capture, memory. A reader is `Send + Sync` and is used through a shared reference, so many threads can
/// read at once; a source that cannot (an HDF5 handle behind a lock) serialises inside.
pub trait ColumnSource: Send + Sync {
    /// Reads the column a request names, for the range and at the precision it asks for. A long read checks `context` as described in the module
    /// documentation.
    ///
    /// Errors: `NotPresent` for a column the source does not have, `InvalidIndex` for a range outside it or a precision it cannot give, `Cancelled`
    /// when the token fires. Every error carries the layer, laser, channel and the operation.
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error>;

    /// Reads the columns of one place of an OPC UA event series: the source times, the values as stored (text) with the typed view of them when the series declares a type this
    /// library names, and for a task its start and its stop, for the records the request names (decision D06-12). **Bulk data stays behind this one door**: a source is
    /// opened once and read through here.
    ///
    /// Defaulted: a source that holds no event columns answers `UnsupportedInVersion`, so that a source written before events were readable still compiles and says so; one that
    /// has them declares `Feature::EventColumns`. Errors: `NotPresent` for a series or a place that the source does not have, `InvalidIndex` for a range outside the records,
    /// `UnsupportedInVersion` for a time range, `Cancelled` when the token fires. Every error carries the layer, the series as a channel and the operation `read_events`.
    fn read_events(
        &self,
        request: &EventRequest,
        context: &ReadContext,
    ) -> Result<EventColumns, Error> {
        let _ = context;
        Err(Error::new(ErrorKind::UnsupportedInVersion {
            version: "this source".to_string(),
            feature: "reading the columns of event series".to_string(),
        })
        .at(request.location()))
    }

    /// Reads the pixels of a region of one camera frame, as stored: the layer, the phase and the rectangle of rows and columns (the whole frame if the request names none), row-major
    /// (decision D06-13). **Bulk data stays behind this one door.**
    ///
    /// Defaulted: a source that holds no frames answers `UnsupportedInVersion`, so that a source written before frames were readable still compiles and says so; one that has them declares
    /// `Feature::ImageRegions`. Errors: `NotPresent` for a phase or a layer that the source has no frame for, `InvalidIndex` for a region outside the frame (the error says the frame's dimensions),
    /// `Cancelled` when the token fires. Every error carries the layer, the phase as a channel and the operation `read_image`.
    fn read_image(
        &self,
        request: &ImageRequest,
        context: &ReadContext,
    ) -> Result<ImageData, Error> {
        let _ = context;
        Err(Error::new(ErrorKind::UnsupportedInVersion {
            version: "this source".to_string(),
            feature: "reading regions of camera frames".to_string(),
        })
        .at(request.location()))
    }

    /// Reads the values of one correction grid of a laser, as stored: the laser and the grid by the name the file spells, the whole grid, **in the type the file holds and row-major in the extents as
    /// the file stores them** (decision D24-11). **Bulk data stays behind this one door.** The library does not interpret a grid: its domain and its axes are stated by no file, and a value that is not
    /// a number comes back as it is.
    ///
    /// Defaulted: a source that holds no grids answers `UnsupportedInVersion`, so that a source written before grids were readable still compiles and says so; one that has them declares
    /// `Feature::CorrectionGrids`. Errors: `NotPresent` for a laser or a grid that the source does not have, `Cancelled` when the token fires. Every error carries the laser, the grid as a channel
    /// and the operation `read_grid`.
    fn read_grid(&self, request: &GridRequest, context: &ReadContext) -> Result<GridData, Error> {
        let _ = context;
        Err(Error::new(ErrorKind::UnsupportedInVersion {
            version: "this source".to_string(),
            feature: "reading correction grids".to_string(),
        })
        .at(request.location()))
    }

    /// What the source's cache of decoded columns has done so far, or `None` if it has none (PS-12). A source that keeps nothing answers `None`; a caller that wants to
    /// know whether a budget helps reads it before and after.
    fn cache_stats(&self) -> Option<CacheStats> {
        None
    }
}

/// What a source's cache of decoded columns has done since the source was opened, and what it holds now.
// constructed-by: library
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct CacheStats {
    /// Blocks that a read found in the cache.
    pub hits: u64,
    /// Blocks that a read looked for and did not find.
    pub misses: u64,
    /// Blocks that were dropped to make room for another.
    pub evictions: u64,
    /// The bytes of decoded values that the cache holds now.
    pub bytes_held: u64,
    /// The budget of the read that last changed the cache.
    pub budget: u64,
}

/// A token that changes when a source grows: two equal tokens mean that nothing was added in between, and a later token is never smaller. A live capture
/// bumps it as each layer is committed, so a viewer can ask "has anything changed?" without reading the metadata again.
// constructed-by: library
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub struct Revision(u64);

impl Revision {
    pub fn new(value: u64) -> Self {
        Revision(value)
    }

    pub fn value(&self) -> u64 {
        self.0
    }

    /// The token after one more change.
    pub fn next(&self) -> Revision {
        Revision(self.0.saturating_add(1))
    }
}

/// Discovery: what a build holds, without reading a sample. Every file adapter and [`InMemorySource`] implement it. It is part of the read surface that was frozen on
/// 2026-11-05 (decision D24-20): an additive change is logged, and a change to what it already offers needs a recorded decision.
///
/// A source that can be discovered can also be read, so this extends [`ColumnSource`]. An implementation needs only [`BuildSource::build`] and
/// [`BuildSource::revision`]; the rest are derived from the metadata. **None of these methods takes a range or a context**, so none of them can read samples.
pub trait BuildSource: ColumnSource {
    /// The metadata of the build as of now: a shared, immutable snapshot, cheap to hold and to clone.
    fn build(&self) -> Result<Arc<BuildMeta>, Error>;

    /// The token of the build as of now. It differs from an earlier one if the source has grown since.
    fn revision(&self) -> Result<Revision, Error>;

    /// The ids of the layers the build holds, in order. Gaps are allowed.
    fn layers(&self) -> Result<Vec<LayerId>, Error> {
        Ok(self.build()?.layers.ids().collect())
    }

    /// The lasers that have something in a layer, in numeric order. A layer the build does not hold is an `InvalidIndex` error.
    fn lasers_in_layer(&self, layer: LayerId) -> Result<Vec<LaserKey>, Error> {
        let build = self.build()?;
        let located =
            |error: Error| error.at(Location::new().with_layer(layer).with_operation("discover"));
        if !build.layers.contains(layer) {
            return Err(located(Error::new(ErrorKind::InvalidIndex {
                what: "a layer that the build does not hold".to_string(),
            })));
        }
        Ok(build
            .lasers()
            .iter()
            .filter(|laser| laser.layers.contains(layer))
            .map(|laser| laser.key.clone())
            .collect())
    }

    /// What is wrong or odd about the build, found from its metadata **without reading a sample** (IP-07 phase 7C): findings with a code, a severity and the layer, laser and channel they are
    /// about, and counts of each severity. It is the same function of the metadata for every source (`validate_build`), so a source that is not a file has it with nothing of its own; a source
    /// that keeps its build still may keep the report too, to give the same one each time.
    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        let build = self.build()?;
        Ok(Arc::new(validate_build(&build)))
    }

    /// What can be done with each layer, laser in a layer and channel in a layer, **without reading a sample** (IP-07 phase 7D): one of the five states of BL-FA-09 for any item, and counts of each
    /// state for each kind of item. **This default says that everything can be read**; a source that cannot read some kind of channel overrides it and says which (`availability_of`, whose last
    /// argument is what the source cannot read).
    fn availability(&self) -> Result<Arc<AvailabilityReport>, Error> {
        let build = self.build()?;
        let validation = self.validate()?;
        Ok(Arc::new(availability_of(&build, &validation, &|_| None)))
    }

    /// The channels of a laser, described: what each is, how it is clocked, its unit, and in which layers it was recorded. A laser the build does not have
    /// is an `InvalidIndex` error.
    fn channels(&self, laser: &LaserKey) -> Result<Vec<ChannelDescriptor>, Error> {
        let build = self.build()?;
        match build.laser(laser) {
            Some(found) => Ok(found.channels.clone()),
            None => Err(Error::new(ErrorKind::InvalidIndex {
                what: "a laser that the build does not have".to_string(),
            })
            .at(Location::new()
                .with_laser(laser.clone())
                .with_operation("discover"))),
        }
    }
}

/// A source whose columns and metadata are in memory. Reads copy in blocks and check the token and report progress exactly as a file-backed source does, so it is
/// the reference that the file-backed sources are tested against.
// constructed-by: library
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct InMemorySource {
    columns: BTreeMap<(LayerId, Option<LaserKey>, ChannelKey), ColumnData>,
    segment_tables: BTreeMap<(LayerId, LaserKey), ChannelKey>,
    events: BTreeMap<(EventKind, Vec<String>, EventPlace), EventColumns>,
    images: BTreeMap<(LayerId, ChannelKey), ImageData>,
    grids: BTreeMap<(LaserKey, String), GridData>,
    build: Option<Arc<BuildMeta>>,
    revision: Revision,
}

impl InMemorySource {
    pub fn new() -> Self {
        InMemorySource::default()
    }

    /// Adds the values of a laser's correction grid, by the name the file spells, and returns the ones it replaced.
    pub fn insert_grid(
        &mut self,
        laser: LaserKey,
        name: impl Into<String>,
        data: GridData,
    ) -> Option<GridData> {
        self.revision = self.revision.next();
        self.grids.insert((laser, name.into()), data)
    }

    /// Adds a column, and returns the one it replaced.
    pub fn insert(
        &mut self,
        layer: LayerId,
        laser: Option<LaserKey>,
        channel: ChannelKey,
        data: ColumnData,
    ) -> Option<ColumnData> {
        self.revision = self.revision.next();
        self.columns.insert((layer, laser, channel), data)
    }

    /// Adds the columns of one place of an event series, and returns the ones it replaced.
    pub fn insert_events(
        &mut self,
        kind: EventKind,
        path: Vec<String>,
        place: EventPlace,
        columns: EventColumns,
    ) -> Option<EventColumns> {
        self.revision = self.revision.next();
        self.events.insert((kind, path, place), columns)
    }

    /// Adds the whole frame of a phase in a layer (`frame.region` is the whole frame, from its first row and column), and returns the one it replaced. A region of it is cropped from it.
    pub fn insert_image(
        &mut self,
        layer: LayerId,
        channel: ChannelKey,
        frame: ImageData,
    ) -> Option<ImageData> {
        self.revision = self.revision.next();
        self.images.insert((layer, channel), frame)
    }

    /// Sets the metadata of the build, which discovery reports. The source has grown, so its revision changes.
    pub fn set_build(&mut self, build: BuildMeta) {
        self.revision = self.revision.next();
        self.build = Some(Arc::new(build));
    }

    /// Says which channel of this laser in this layer holds its segment table. The column itself is added with [`InMemorySource::insert`].
    pub fn set_segment_table(&mut self, layer: LayerId, laser: LaserKey, channel: ChannelKey) {
        self.revision = self.revision.next();
        self.segment_tables.insert((layer, laser), channel);
    }

    /// The sample indices a request selects in a column of `len` values.
    fn resolve(&self, request: &ColumnRequest, len: usize) -> Result<Range<usize>, Error> {
        let invalid = |what: String| Error::new(ErrorKind::InvalidIndex { what });
        let to_usize =
            |n: u64| usize::try_from(n).map_err(|_| invalid(format!("the index {n} is too large")));
        match &request.range {
            ColumnRange::All => Ok(0..len),
            ColumnRange::Indices(range) => {
                if range.start > range.end || range.end > len as u64 {
                    return Err(invalid(format!(
                        "the indices {}..{} are outside a column of {len} values",
                        range.start, range.end
                    )));
                }
                Ok(to_usize(range.start)?..to_usize(range.end)?)
            }
            ColumnRange::Time { from, to } => {
                let laser = request.laser.as_ref().ok_or_else(|| {
                    invalid(
                        "a time range needs a laser: only a laser has a segment table".to_string(),
                    )
                })?;
                let table_key = self
                    .segment_tables
                    .get(&(request.layer, laser.clone()))
                    .ok_or_else(|| {
                        Error::new(ErrorKind::NotPresent {
                            what: "the segment table of this laser in this layer".to_string(),
                        })
                    })?;
                let table = self
                    .columns
                    .get(&(request.layer, Some(laser.clone()), table_key.clone()))
                    .ok_or_else(|| {
                        Error::new(ErrorKind::NotPresent {
                            what: "the column that holds the segment table".to_string(),
                        })
                    })?;
                let segments = table.as_segments().ok_or_else(|| {
                    Error::new(ErrorKind::Corrupt {
                        detail: "the column named as the segment table does not hold segments"
                            .to_string(),
                    })
                })?;
                match index_range(segments, from, to)? {
                    None => Ok(0..0),
                    Some(range) if range.end <= len as u64 => {
                        Ok(to_usize(range.start)?..to_usize(range.end)?)
                    }
                    Some(range) => Err(invalid(format!(
                        "the segment table selects {}..{}, outside a column of {len} values",
                        range.start, range.end
                    ))),
                }
            }
        }
    }
}

impl ColumnSource for InMemorySource {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        let located = |error: Error| error.at(request.location());
        context.check_cancelled().map_err(located)?;
        let column = self
            .columns
            .get(&(
                request.layer,
                request.laser.clone(),
                request.channel.clone(),
            ))
            .ok_or_else(|| {
                located(Error::new(ErrorKind::NotPresent {
                    what: "the column".to_string(),
                }))
            })?;
        let range = self.resolve(request, column.len()).map_err(located)?;
        column
            .copy_range(range, context)
            .and_then(|copied| copied.with_precision(request.precision))
            .map_err(located)
    }

    fn read_events(
        &self,
        request: &EventRequest,
        context: &ReadContext,
    ) -> Result<EventColumns, Error> {
        let located = |error: Error| error.at(request.location());
        context.check_cancelled().map_err(located)?;
        let columns = self
            .events
            .get(&(request.kind.clone(), request.path.clone(), request.place))
            .ok_or_else(|| {
                located(Error::new(ErrorKind::NotPresent {
                    what: "the series in this place".to_string(),
                }))
            })?;
        let range = self
            .resolve_events(request, columns.len())
            .map_err(located)?;
        context.report(columns.len() as u64, columns.len() as u64);
        columns.range(range).ok_or_else(|| {
            located(Error::new(ErrorKind::InvalidIndex {
                what: "the records are outside the series".to_string(),
            }))
        })
    }

    fn read_grid(&self, request: &GridRequest, context: &ReadContext) -> Result<GridData, Error> {
        let located = |error: Error| error.at(request.location());
        context.check_cancelled().map_err(located)?;
        let grid = self
            .grids
            .get(&(request.laser.clone(), request.grid.clone()))
            .ok_or_else(|| {
                located(Error::new(ErrorKind::NotPresent {
                    what: "the grid".to_string(),
                }))
            })?;
        let values = grid.len() as u64;
        context.report(values, values);
        Ok(grid.clone())
    }

    fn read_image(
        &self,
        request: &ImageRequest,
        context: &ReadContext,
    ) -> Result<ImageData, Error> {
        let located = |error: Error| error.at(request.location());
        context.check_cancelled().map_err(located)?;
        let frame = self
            .images
            .get(&(request.layer, request.channel.clone()))
            .ok_or_else(|| {
                located(Error::new(ErrorKind::NotPresent {
                    what: "the frame".to_string(),
                }))
            })?;
        let wanted: ImageRegion = request
            .region
            .clone()
            .unwrap_or_else(|| frame.region.clone());
        let pixels = frame.pixels.len() as u64;
        context.report(pixels, pixels);
        frame.crop(&wanted).ok_or_else(|| {
            located(Error::new(ErrorKind::InvalidIndex {
                what: format!(
                    "the region of rows {}..{} and columns {}..{} is outside a frame of {} rows by {} columns",
                    wanted.rows.start,
                    wanted.rows.end,
                    wanted.columns.start,
                    wanted.columns.end,
                    frame.region.rows.end,
                    frame.region.columns.end
                ),
            }))
        })
    }
}

impl InMemorySource {
    /// The records an event request selects among `len`.
    fn resolve_events(&self, request: &EventRequest, len: usize) -> Result<Range<usize>, Error> {
        match &request.range {
            ColumnRange::All => Ok(0..len),
            ColumnRange::Indices(range) => {
                if range.start > range.end || range.end > len as u64 {
                    return Err(Error::new(ErrorKind::InvalidIndex {
                        what: format!(
                            "the indices {}..{} are outside a series of {len} records",
                            range.start, range.end
                        ),
                    }));
                }
                Ok(range.start as usize..range.end as usize)
            }
            _ => Err(Error::new(ErrorKind::UnsupportedInVersion {
                version: "the in-memory source".to_string(),
                feature: "a time range of event records".to_string(),
            })),
        }
    }
}

impl BuildSource for InMemorySource {
    fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        self.build.clone().ok_or_else(|| {
            Error::new(ErrorKind::NotPresent {
                what: "the metadata of the build".to_string(),
            })
            .at(Location::new().with_operation("discover"))
        })
    }

    fn revision(&self) -> Result<Revision, Error> {
        Ok(self.revision)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    use super::*;
    use crate::channel::{ChannelDType, ChannelKind, ClockClass, NumericType};
    use crate::error::ErrorClass;
    use crate::identity::Presence;
    use crate::meta::{BuildIdentity, LaserLayerMeta, LaserMeta};

    const LAYER: LayerId = LayerId(1);

    #[test]
    fn a_context_keeps_nothing_until_it_is_given_a_budget() {
        assert_eq!(ReadContext::new().cache_budget(), 0);
        assert_eq!(ReadContext::default().cache_budget(), 0);
        let context = ReadContext::new().with_cache_budget(64 << 20);
        assert_eq!(context.cache_budget(), 64 << 20);
        // the budget travels with a clone, and shows in the debug text
        assert_eq!(context.clone().cache_budget(), 64 << 20);
        assert!(format!("{context:?}").contains("cache_budget: 67108864"));
    }

    #[test]
    fn a_source_with_no_cache_has_no_stats() {
        assert_eq!(InMemorySource::new().cache_stats(), None);
        assert_eq!(
            CacheStats::default().hits + CacheStats::default().bytes_held,
            0
        );
    }

    fn laser() -> LaserKey {
        LaserKey::new("1", 1)
    }

    fn key(name: &str) -> ChannelKey {
        ChannelKey::new([name])
    }

    fn request(name: &str) -> ColumnRequest {
        ColumnRequest::new(LAYER, Some(laser()), key(name))
    }

    /// A source with one laser, a column of `n` `i32` values counting up from zero, and another of the same values as `f32`.
    fn counting_source(n: i32) -> InMemorySource {
        let mut source = InMemorySource::new();
        source.insert(
            LAYER,
            Some(laser()),
            key("counts"),
            ColumnData::I32(Shared::new((0..n).collect())),
        );
        source.insert(
            LAYER,
            Some(laser()),
            key("floats"),
            ColumnData::F32(Shared::new((0..n).map(|i| i as f32 * 0.1).collect())),
        );
        source
    }

    #[test]
    fn column_source_is_object_safe_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        // it can be a trait object, boxed or behind an Arc, and shared between threads
        let boxed: Box<dyn ColumnSource> = Box::new(counting_source(3));
        let shared: Arc<dyn ColumnSource> = Arc::new(counting_source(3));
        assert_send_sync::<Box<dyn ColumnSource>>();
        assert_send_sync::<Arc<dyn ColumnSource>>();
        assert_send_sync::<InMemorySource>();
        assert_send_sync::<ColumnData>();
        assert_send_sync::<Shared<f32>>();
        assert_send_sync::<ReadContext>();
        assert_send_sync::<CancelToken>();
        assert_eq!(
            boxed
                .read(&request("counts"), &ReadContext::new())
                .unwrap()
                .len(),
            3
        );
        // and one read from several threads at once gives each thread the whole column
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let source = Arc::clone(&shared);
                std::thread::spawn(move || {
                    source
                        .read(&request("counts"), &ReadContext::new())
                        .unwrap()
                        .len()
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), 3);
        }
    }

    #[test]
    fn shared_buffer_slices_do_not_copy() {
        let whole = Shared::new((0..100_u32).collect());
        assert_eq!(whole.holders(), 1);
        let part = whole.slice(10..20).expect("inside");
        // the slice points into the same allocation, at the right place, and counts as one more holder
        assert_eq!(part.as_slice().as_ptr(), whole.as_slice()[10..].as_ptr());
        assert_eq!(whole.holders(), 2);
        assert_eq!(part.as_slice(), &(10..20).collect::<Vec<u32>>()[..]);
        // a slice of a slice is still the same allocation
        let inner = part.slice(2..5).unwrap();
        assert_eq!(inner.as_slice().as_ptr(), whole.as_slice()[12..].as_ptr());
        assert_eq!(whole.holders(), 3);
        // outside the run is `None`, not a panic, and an empty slice is fine
        let (high, low) = (5, 4);
        assert!(whole.slice(90..101).is_none() && whole.slice(high..low).is_none());
        assert!(whole.slice(100..100).unwrap().is_empty());
        // the same for a typed column
        let column = ColumnData::I32(Shared::new((0..50).collect()));
        let piece = column.slice(5..10).unwrap();
        assert_eq!(piece.as_i32(), Some(&[5, 6, 7, 8, 9][..]));
        assert!(column.slice(0..51).is_none());
    }

    #[test]
    fn a_never_cancelled_token_never_cancels() {
        let never = CancelToken::never();
        assert!(!never.is_cancelled());
        // cancelling it does nothing, and so does cancelling a clone, and so do a great many checks
        never.cancel();
        never.clone().cancel();
        assert!((0..100_000).all(|_| !never.is_cancelled()));
        assert!(!CancelToken::default().is_cancelled());
        assert!(ReadContext::new().check_cancelled().is_ok());
        // a token that can be cancelled is cancelled for every clone
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
    }

    /// A small deterministic generator, so that the property test needs no dependency and fails the same way every time.
    struct Lcg(u64);
    impl Lcg {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % bound
        }
    }

    #[test]
    fn ranged_reads_equal_slices_of_the_full_column() {
        // 25,000 values: more than two blocks, so reads cross the block boundaries of 10,000
        let source = counting_source(25_000);
        let context = ReadContext::new();
        let full = source.read(&request("counts"), &context).unwrap();
        let full = full.as_i32().unwrap();
        assert_eq!(full.len(), 25_000);
        let mut random = Lcg(7);
        for _ in 0..2_000 {
            let a = random.below(25_001);
            let b = random.below(25_001);
            let (start, end) = (a.min(b), a.max(b));
            let ranged = source
                .read(
                    &request("counts").with_range(ColumnRange::Indices(start..end)),
                    &context,
                )
                .unwrap();
            assert_eq!(
                ranged.as_i32().unwrap(),
                &full[start as usize..end as usize],
                "{start}..{end}"
            );
        }
        // the ends and the empty ranges
        for (start, end) in [
            (0, 0),
            (0, 1),
            (24_999, 25_000),
            (25_000, 25_000),
            (9_999, 10_001),
            (0, 25_000),
        ] {
            let ranged = source
                .read(
                    &request("counts").with_range(ColumnRange::Indices(start..end)),
                    &context,
                )
                .unwrap();
            assert_eq!(
                ranged.as_i32().unwrap(),
                &full[start as usize..end as usize]
            );
        }
    }

    #[test]
    fn cancel_returns_cancelled_mid_read() {
        let source = counting_source(35_000);
        let token = CancelToken::new();
        // a sink that cancels once 20,000 values have been copied: the third block is then never read
        let stopper = token.clone();
        let context = ReadContext::new()
            .with_cancel(token)
            .with_progress(Arc::new(move |done: u64, _total: u64| {
                if done >= 20_000 {
                    stopper.cancel();
                }
            }));
        let error = source.read(&request("counts"), &context).unwrap_err();
        assert_eq!(error.kind(), &ErrorKind::Cancelled);
        assert_eq!(error.class(), ErrorClass::Caller);
        assert!(error.is_retryable());
        // the error says where it happened
        assert_eq!(error.location().layer, Some(LAYER));
        assert_eq!(error.location().channel, Some(key("counts")));
        assert_eq!(error.location().operation.as_deref(), Some("read"));
        // a token cancelled before the read stops it before it starts
        let early = CancelToken::new();
        early.cancel();
        let error = source
            .read(&request("counts"), &ReadContext::new().with_cancel(early))
            .unwrap_err();
        assert_eq!(error.kind(), &ErrorKind::Cancelled);
    }

    #[test]
    fn time_range_resolves_through_the_segment_table() {
        // 152 values: sample i is at time 1000 + 10 i in the first segment (0..=100) and 5000 + 10 (i - 101) in the second (101..=151)
        let table = [
            Segment {
                first_sample: 0,
                samples: 101,
                start: TimeValue::from_file(1000, None),
                end: TimeValue::from_file(2000, None),
            },
            Segment {
                first_sample: 101,
                samples: 51,
                start: TimeValue::from_file(5000, None),
                end: TimeValue::from_file(5500, None),
            },
        ];
        let mut source = InMemorySource::new();
        source.insert(
            LAYER,
            Some(laser()),
            key("power"),
            ColumnData::F32(Shared::new((0..152).map(|i| i as f32).collect())),
        );
        source.insert(
            LAYER,
            Some(laser()),
            key("segments"),
            ColumnData::Segments(Shared::new(table.to_vec())),
        );
        source.set_segment_table(LAYER, laser(), key("segments"));
        let time = |from: i64, to: i64| ColumnRange::Time {
            from: TimeValue::from_file(from, None),
            to: TimeValue::from_file(to, None),
        };
        let read = |from: i64, to: i64| {
            source
                .read(
                    &request("power").with_range(time(from, to)),
                    &ReadContext::new(),
                )
                .unwrap()
        };
        // across the gap: samples 50 to 121, the same as the indices the table gives
        let across = read(1500, 5200);
        let expected: Vec<f32> = (50..122).map(|i| i as f32).collect();
        assert_eq!(across.as_f32().unwrap(), &expected[..]);
        let by_index = source
            .read(
                &request("power").with_range(ColumnRange::Indices(50..122)),
                &ReadContext::new(),
            )
            .unwrap();
        assert_eq!(across, by_index);
        // wholly in the gap: an empty column, not an error
        assert!(read(2500, 4500).is_empty());
        // the table is read like any other column
        let rows = source
            .read(&request("segments"), &ReadContext::new())
            .unwrap();
        assert_eq!(rows.as_segments().unwrap(), &table[..]);
        // a time range for a laser with no segment table, or for a column of no laser, says so
        let mut bare = InMemorySource::new();
        bare.insert(
            LAYER,
            Some(laser()),
            key("power"),
            ColumnData::F32(Shared::new(vec![0.0; 5])),
        );
        let error = bare
            .read(
                &request("power").with_range(time(0, 10)),
                &ReadContext::new(),
            )
            .unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{error}"
        );
        bare.insert(
            LAYER,
            None,
            key("sensor"),
            ColumnData::F32(Shared::new(vec![0.0; 5])),
        );
        let no_laser = ColumnRequest::new(LAYER, None, key("sensor")).with_range(time(0, 10));
        assert!(matches!(
            bare.read(&no_laser, &ReadContext::new())
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidIndex { .. }
        ));
    }

    #[test]
    fn requested_precision_f32_equals_widening_back() {
        let values = [0.1_f32, -3.5, 1.0e-30, 16_777_217.0, f32::MAX];
        let mut source = InMemorySource::new();
        source.insert(
            LAYER,
            Some(laser()),
            key("f32"),
            ColumnData::F32(Shared::new(values.to_vec())),
        );
        let context = ReadContext::new();
        // f32 asked as f64: each value is the widened one, and narrowing it back gives the stored value exactly
        let wide = source
            .read(&request("f32").with_precision(Precision::F64), &context)
            .unwrap();
        for (stored, widened) in values.iter().zip(wide.as_f64().unwrap()) {
            assert_eq!(*widened, f64::from(*stored));
            assert_eq!(*widened as f32, *stored);
        }
        // f32 asked as f32, or as stored, is the stored column: the same values
        let same = source
            .read(&request("f32").with_precision(Precision::F32), &context)
            .unwrap();
        assert_eq!(same.as_f32().unwrap(), &values[..]);
        let native = source.read(&request("f32"), &context).unwrap();
        assert_eq!(native, same);
        // f64 narrowed to f32 is rounded to the nearest, as asked
        let mut doubles = InMemorySource::new();
        doubles.insert(
            LAYER,
            Some(laser()),
            key("f64"),
            ColumnData::F64(Shared::new(vec![0.1, 2.5])),
        );
        let narrow = doubles
            .read(&request("f64").with_precision(Precision::F32), &context)
            .unwrap();
        assert_eq!(narrow.as_f32().unwrap(), &[0.1_f64 as f32, 2.5_f32][..]);
        // integers become numbers of the type asked for, and the stored type is left alone when native
        let counts = counting_source(4);
        let as_f64 = counts
            .read(&request("counts").with_precision(Precision::F64), &context)
            .unwrap();
        assert_eq!(as_f64.as_f64().unwrap(), &[0.0, 1.0, 2.0, 3.0][..]);
        assert!(counts
            .read(&request("counts"), &context)
            .unwrap()
            .as_i32()
            .is_some());
        // a column that is not numbers has no precision
        let mut text = InMemorySource::new();
        text.insert(
            LAYER,
            None,
            key("names"),
            ColumnData::Text(Shared::new(vec!["a".to_string()])),
        );
        let asked = ColumnRequest::new(LAYER, None, key("names")).with_precision(Precision::F32);
        assert!(matches!(
            text.read(&asked, &context).unwrap_err().kind(),
            ErrorKind::InvalidIndex { .. }
        ));
    }

    #[test]
    fn progress_is_reported_monotonically() {
        let source = counting_source(35_000);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let context = ReadContext::new().with_progress(Arc::new(move |done: u64, total: u64| {
            sink.lock().unwrap().push((done, total))
        }));
        source.read(&request("counts"), &context).unwrap();
        let seen = seen.lock().unwrap();
        // one report per block of 10,000, ending at the total, with a total that does not change
        assert_eq!(
            *seen,
            [
                (10_000, 35_000),
                (20_000, 35_000),
                (30_000, 35_000),
                (35_000, 35_000)
            ]
        );
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));
        // a range reports against its own length
        let sink = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&sink);
        let context = ReadContext::new().with_progress(Arc::new(move |done: u64, total: u64| {
            recorder.lock().unwrap().push((done, total))
        }));
        source
            .read(
                &request("counts").with_range(ColumnRange::Indices(100..110)),
                &context,
            )
            .unwrap();
        assert_eq!(*sink.lock().unwrap(), [(10, 10)]);
    }

    #[test]
    fn a_request_for_something_absent_or_outside_is_a_typed_error_with_its_location() {
        let source = counting_source(100);
        let context = ReadContext::new();
        let absent = source.read(&request("missing"), &context).unwrap_err();
        assert!(matches!(absent.kind(), ErrorKind::NotPresent { .. }));
        assert_eq!(absent.location().channel, Some(key("missing")));
        // another layer, or another laser, is a column that is not there
        let other_layer = ColumnRequest::new(LayerId(2), Some(laser()), key("counts"));
        assert!(matches!(
            source.read(&other_layer, &context).unwrap_err().kind(),
            ErrorKind::NotPresent { .. }
        ));
        // a range outside the column, or one that ends before it starts, is an invalid index
        for (start, end) in [(0, 101), (50, 40), (101, 101)] {
            let outside = source
                .read(
                    &request("counts").with_range(ColumnRange::Indices(start..end)),
                    &context,
                )
                .unwrap_err();
            assert!(
                matches!(outside.kind(), ErrorKind::InvalidIndex { .. }),
                "{outside}"
            );
            assert_eq!(outside.location().laser, Some(laser()));
        }
    }

    // ------------------------------------------------------------ discovery (task T02.28)

    /// A build with two lasers, layers 1 and 3 (a gap), and one channel for the first laser.
    fn discoverable() -> InMemorySource {
        let mut build = BuildMeta::new(BuildIdentity {
            build_id: Some("build-1".into()),
            date: None,
            username: None,
            alloy: None,
            part_count: None,
            project_id: None,
            source_mcf_name: None,
            source_mcf_hash: None,
        });
        for (number, layers) in [(1, vec![1, 3]), (2, vec![1])] {
            let mut laser = LaserMeta::new(LaserKey::new(number.to_string(), number));
            for layer in layers {
                laser.layers.insert(
                    LayerId(layer),
                    LaserLayerMeta::new(Presence::from_count(25_000)),
                );
            }
            if number == 1 {
                laser.channels.push(ChannelDescriptor::new(
                    key("counts"),
                    ChannelKind::Sensor,
                    ClockClass::LaserSynchronous,
                    ChannelDType::Numeric(NumericType::I32),
                ));
            }
            build.add_laser(laser);
        }
        build.synthesise_missing_layers();
        let mut source = counting_source(100);
        source.set_build(build);
        source
    }

    #[test]
    fn in_memory_source_implements_build_source() {
        // it is a trait object, and the one object discovers and reads
        let source: Box<dyn BuildSource> = Box::new(discoverable());
        assert_eq!(
            source.build().unwrap().identity.build_id.as_deref(),
            Some("build-1")
        );
        assert_eq!(source.layers().unwrap(), [LayerId(1), LayerId(3)]);
        assert_eq!(source.lasers_in_layer(LayerId(1)).unwrap().len(), 2);
        assert_eq!(source.lasers_in_layer(LayerId(3)).unwrap(), [laser()]);
        let channels = source.channels(&laser()).unwrap();
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].key, key("counts"));
        assert!(source.channels(&LaserKey::new("2", 2)).unwrap().is_empty());
        assert_eq!(
            source
                .read(&request("counts"), &ReadContext::new())
                .unwrap()
                .len(),
            100
        );
        // a layer or a laser that is not there is an invalid index with its location, not an empty answer
        let no_layer = source.lasers_in_layer(LayerId(2)).unwrap_err();
        assert!(matches!(no_layer.kind(), ErrorKind::InvalidIndex { .. }));
        assert_eq!(no_layer.location().layer, Some(LayerId(2)));
        let no_laser = source.channels(&LaserKey::new("9", 9)).unwrap_err();
        assert!(matches!(no_laser.kind(), ErrorKind::InvalidIndex { .. }));
        // a source with no metadata says so
        assert!(matches!(
            InMemorySource::new().build().unwrap_err().kind(),
            ErrorKind::NotPresent { .. }
        ));
    }

    /// Counts the reads that reach the columns, and passes everything else to the source it wraps.
    struct Counting {
        inner: InMemorySource,
        reads: AtomicUsize,
    }

    impl ColumnSource for Counting {
        fn read(
            &self,
            request: &ColumnRequest,
            context: &ReadContext,
        ) -> Result<ColumnData, Error> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.inner.read(request, context)
        }
    }

    impl BuildSource for Counting {
        fn build(&self) -> Result<Arc<BuildMeta>, Error> {
            self.inner.build()
        }
        fn revision(&self) -> Result<Revision, Error> {
            self.inner.revision()
        }
    }

    #[test]
    fn discovery_reads_no_samples() {
        // every discovery method, on a source that counts the reads that reach its columns
        let counting = Counting {
            inner: discoverable(),
            reads: AtomicUsize::new(0),
        };
        counting.build().unwrap();
        counting.revision().unwrap();
        counting.layers().unwrap();
        counting.lasers_in_layer(LayerId(1)).unwrap();
        counting.channels(&laser()).unwrap();
        assert_eq!(
            counting.reads.load(Ordering::SeqCst),
            0,
            "discovery must not read a column"
        );
        // and a source that holds metadata and no column at all can be discovered completely: discovery needs no sample
        let mut metadata_only = InMemorySource::new();
        metadata_only.set_build((*discoverable().build().unwrap()).clone());
        assert_eq!(metadata_only.layers().unwrap(), [LayerId(1), LayerId(3)]);
        assert_eq!(metadata_only.channels(&laser()).unwrap().len(), 1);
        assert!(matches!(
            metadata_only
                .read(&request("counts"), &ReadContext::new())
                .unwrap_err()
                .kind(),
            ErrorKind::NotPresent { .. }
        ));
    }

    #[test]
    fn revision_token_changes_when_the_source_grows() {
        let mut source = InMemorySource::new();
        let first = source.revision().unwrap();
        assert_eq!(first, Revision::default());
        // asking again does not change it
        assert_eq!(first, source.revision().unwrap());
        source.insert(
            LAYER,
            Some(laser()),
            key("counts"),
            ColumnData::I32(Shared::new(vec![1, 2, 3])),
        );
        let after_insert = source.revision().unwrap();
        assert!(after_insert > first);
        source
            .read(&request("counts"), &ReadContext::new())
            .unwrap();
        assert_eq!(
            source.revision().unwrap(),
            after_insert,
            "a read is not growth"
        );
        // adding the metadata, or another column, is growth, and a token never goes back
        source.set_build((*discoverable().build().unwrap()).clone());
        let after_build = source.revision().unwrap();
        assert!(after_build > after_insert);
        source.insert(
            LAYER,
            Some(laser()),
            key("more"),
            ColumnData::I32(Shared::new(vec![4])),
        );
        assert!(source.revision().unwrap() > after_build);
        assert_eq!(Revision::new(7).next().value(), 8);
    }
}
