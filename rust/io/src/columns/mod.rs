//! The column engine (IP-06 phases 6B and 6C, tasks T06.2 and T06.3, decisions D06-1, D06-3 to D06-7 and D06-9): the shared code that turns a request for a column into values.
//!
//! * **Layer:** the IO layer, above the primitives (`hdf5`) and below the adapters. **An adapter only says where a column is stored**: it provides a [`Locator`], which turns
//!   a [`ColumnRequest`] into the dataset to read. **Everything else is here, once, for every version**: the range rules, the blocks, the cancel and progress rules, the
//!   precision, the typing of the result by what is stored, and the errors with their location.
//! * **Layout-free (BL-ST-13):** no group, dataset or attribute name of any version appears in this folder. The guard of the primitives (`tests/primitives_layout_guard.rs`,
//!   decision D03-11) scans it too.
//! * **Internal:** the adapters of this crate and the integration tests use it. A consumer reaches columns through the facade's `ColumnSource`.
//!
//! # What a read does ([`read_column`])
//!
//! 1. **The token is checked**, then the locator is asked for the dataset. A column the locator does not have is its `NotPresent`.
//! 2. **The result is typed by what is stored**, never by what the metadata says (read as recorded, D13-16): `int16` gives `I16`, `int32` `I32`, `uint64` `U64`, `float32`
//!    `F32`, `float64` `F64`, `uint8` `U8` (**that list is one, `Stored` in `engine.rs`, and [`can_read`] answers from it: IP-06 phase 6I, D07-3**). A dataset of another type is
//!    `UnsupportedInVersion`, naming the type and the version. **A dataset that is not one-dimensional is
//!    `Corrupt`**: it is not a column of a laser (D06-9).
//! 3. **The range is the dataset's own** (D06-3): `All` is every value, `Indices(a..b)` the values `a..b` and `a..a` an empty column, never an error. A range past the stored
//!    count, or one that starts after it ends, is `InvalidIndex` and **says the stored count**. **A time range is `UnsupportedInVersion`**: it needs a segment table, and the
//!    files that this engine reads store none.
//! 4. **The values are read in blocks** by the ranged reader of IP-03 (`hdf5::read_blocks`, decision D03-14): the token is checked before each block, progress (values done of
//!    values in all) is reported after it, and **a cancelled read returns `Cancelled` and no partial column**.
//! 5. **Precision is applied block by block**, into the output: `Native` returns the stored width, `F32` and `F64` convert as the Model's `ColumnData::with_precision` does (a test
//!    holds the two to each other), and a native read of one block moves the block's vector into the result without a copy.
//! 5a. **With a cache (a [`ColumnReader`]) the blocks are looked for first** (decision G17, D06-5): a hit is served whatever the budget, a miss that fits the budget of the read is decoded
//!    whole and kept, and a range inside one block is that block's shared buffer, with no copy; a block that does not fit is not kept, and then only the rows asked for are decoded. The
//!    budget is the context's (`ReadContext::with_cache_budget`, default none, D06-6). A conversion is made from the native block and is never kept.
//! 6. **A result that cannot be allocated is a typed error, not an abort**: `IoError`, transient, naming the bytes (decision D06-17).
//! 7. **Every error carries the layer, the laser, the channel and the operation** of the request.
//!
//! # Regions of frames ([`read_frame`], phase 6G)
//!
//! The pixels of a rectangle of a two-dimensional dataset, read as stored through the same ranged reader (`hdf5::read_blocks` with a range for each dimension), with the same cancel and progress
//! rules, the same typing by what is stored and the same typed allocation failure. A region is checked against the frame before any pixel is read, and the error says the frame's dimensions. **A frame
//! is not cached.**
//!
//! There is no thread here (phase 6D) and no knowledge of any version's layout.

mod blocks;
mod cache;
mod convert;
mod engine;
mod frames;
mod grids;

use build_log_model::{
    CacheStats, ChannelKey, ColumnData, ColumnRange, ColumnRequest, Error, ErrorKind, GridData,
    GridRequest, LaserKey, LayerId, Precision, ReadContext,
};
use hdf5::dataset::Dataset;

pub use cache::Cache;
pub use engine::can_read;
pub use frames::read_frame;
pub use grids::{describe_grid, read_grid};

/// What a locator found for a request: the dataset that holds the column.
// not exhaustive: an adapter will say more about what it found (a recorded column that holds no dataset, what the metadata expects) when a phase needs it
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Located {
    pub dataset: Dataset,
}

impl Located {
    pub fn new(dataset: Dataset) -> Self {
        Located { dataset }
    }
}

/// Where a column is, as the cache names it: the layer, the laser if it has one, and the channel. A locator builds it from a [`ColumnRequest`]; a reader of columns that no
/// request names (the events of a series) builds it for the column it reads, and **must make it different from that of every other column of the same file**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub layer: LayerId,
    pub laser: Option<LaserKey>,
    pub channel: ChannelKey,
}

/// What an adapter provides to the engine: where each column of its version is stored.
///
/// A locator is `Send + Sync` and is asked through a shared reference, so many threads can read at once. It does **not** read values and does not keep a dataset: the
/// handle is resolved for each read (decision D06-10).
pub trait Locator: Send + Sync {
    /// The key of the version whose files this locator reads, as the registry spells it (`17`): the errors that say a feature is not supported in a version name it.
    fn version(&self) -> &str;

    /// The dataset that holds the column a request names. A column that the file does not have is `NotPresent`; one that the file has but that cannot be opened is the
    /// error of the file. The error need not carry the location of the request: the engine adds it.
    fn locate(&self, request: &ColumnRequest) -> Result<Located, Error>;

    /// The dataset that holds the correction grid a request names (IP-24 phase 24B): a laser's grid, by the name the file spells. A grid that the file does not have is `NotPresent`.
    ///
    /// Defaulted: a version that holds no grid answers `UnsupportedInVersion` and names itself. The engine reads the grid (`read_grid`); the locator only says where it is.
    fn locate_grid(&self, request: &GridRequest) -> Result<Located, Error> {
        let _ = request;
        Err(Error::new(ErrorKind::UnsupportedInVersion {
            version: self.version().to_string(),
            feature: "reading correction grids".to_string(),
        }))
    }
}

/// Reads the column a request names, through `locator`, as described in the module documentation, **with no cache**: every block is decoded for this read alone.
pub fn read_column(
    request: &ColumnRequest,
    locator: &dyn Locator,
    context: &ReadContext,
) -> Result<ColumnData, Error> {
    engine::read(request, locator, None, context).map_err(|error| error.at(request.location()))
}

/// What an adapter's source holds to answer requests: its locator, and the cache of the decoded blocks of the columns it has served (decision D06-5, one cache for each open file).
pub struct ColumnReader<L: Locator> {
    locator: L,
    cache: Cache,
}

impl<L: Locator> ColumnReader<L> {
    pub fn new(locator: L) -> Self {
        ColumnReader {
            locator,
            cache: Cache::new(),
        }
    }

    pub fn locator(&self) -> &L {
        &self.locator
    }

    /// Reads the column a request names, through the locator and the cache. How much the cache may keep is the context's budget.
    pub fn read(
        &self,
        request: &ColumnRequest,
        context: &ReadContext,
    ) -> Result<ColumnData, Error> {
        engine::read(request, &self.locator, Some(&self.cache), context)
            .map_err(|error| error.at(request.location()))
    }

    /// Reads the whole correction grid a request names, as stored, through the locator (decision D24-11). A grid is not cached: it is read once by whoever needs it.
    pub fn read_grid(
        &self,
        request: &GridRequest,
        context: &ReadContext,
    ) -> Result<GridData, Error> {
        let located = self
            .locator
            .locate_grid(request)
            .map_err(|error| error.at(request.location()))?;
        grids::read_grid(&located.dataset, self.locator.version(), context)
            .map_err(|error| error.at(request.location()))
    }

    /// Reads the numbers of a column whose dataset the caller has found itself (a column that is not a laser's channel, such as the times of an event series), through the same blocks,
    /// cancel and progress rules and cache as `read`: `identity` names the column for the cache, and the numbers come as stored. The caller adds the location to an error.
    pub fn read_dataset(
        &self,
        dataset: &hdf5::dataset::Dataset,
        identity: Identity,
        range: &ColumnRange,
        context: &ReadContext,
    ) -> Result<ColumnData, Error> {
        context.check_cancelled()?;
        engine::read_dataset(
            dataset,
            self.locator.version(),
            identity,
            range,
            Precision::Native,
            Some(&self.cache),
            context,
        )
    }

    /// What the cache has done, and what it holds.
    pub fn stats(&self) -> CacheStats {
        self.cache.stats()
    }
}
