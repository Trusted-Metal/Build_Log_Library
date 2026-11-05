//! The V16 adapter (IP-12): the version that the earlier writer produced, key `"16"`, **read-only** (decision D12-2).
//!
//! * **One folder, one reader, one shape.** V16 has one shape (the miniature, the trimmed file and the original agree), so there is no detector (decision D12-8): the registry selects the
//!   adapter by the marker, `File_Version` = 16, and a V16 file with no marker is for the probe of IP-14.
//! * **[`layout`] holds every name** of the version, and no other file of this folder spells one (BL-ST-13); every path is relative to the mount.
//! * **The toolkit does what versions share** ([`crate::toolkit`], IP-12 phase 12A): it reads a value in whichever form V16 stored it, the build's context from a table, each laser's channels from
//!   a layout value, and the source that the registry hands to a consumer. **This folder says what V16 holds:** **[`read`]** builds the Model's types, with **[`layers`]** for the layers,
//!   **[`lasers`]** for V16's lasers and **[`parameters`]** for their calibration.
//! * **It reads what each phase of IP-12 has reached:** the build level, the context and the layers (12C), the lasers with their channels and the build's sample period (12D), the parameters of the
//!   lasers (12E), and **the samples of the lasers (12F, through [`columns`] and the column engine)**. What a file holds and the adapter does not read is kept in `extra` and never dropped.
//! * **It shares code with no other version.** Everything V16 and V17 have in common is in [`crate::toolkit`], which names no version; this folder names only itself, the toolkit, the shared primitives and
//!   the Model, and two guards keep it so (`tests/adapter_isolation.rs`).
//! * **It declares `Samples` and `CorrectionGrids` and nothing else** (decisions D12-9, D13-7 and D24-11: nothing is declared that the reader does not return). V16 has no outcomes, no events, no images,
//!   no event columns, no image regions, and no write capability.
//! * **It never names another version's module, and never borrows its files** (BL-VA-05; `tests/adapter_isolation.rs`).

pub mod columns;
mod grids;
pub mod lasers;
pub mod layers;
pub mod layout;
pub mod parameters;
pub mod read;

use std::sync::Arc;

use build_log_model::{BuildSource, Capabilities, Direction, Error, Feature};
use hdf5::Group;

use super::{AdapterSet, MarkerPlace};
use crate::toolkit::source::SampleSource;
use columns::V16Locator;

/// The version this adapter reads.
pub const KEY: &str = "16";
/// The adapter's name in a version report.
pub const NAME: &str = "v16";

/// The registration unit of V16: the place of its marker, what it can do (the metadata, and the samples of the lasers), and its reader. **The writer and the patcher are declared unsupported for good**: the library
/// reads V16 and never writes it (the owner's statement of 2026-10-16, G2).
pub fn adapter_set() -> AdapterSet {
    AdapterSet::builder(KEY, NAME, MarkerPlace::new(layout::MOUNT, layout::MARKER))
        .capabilities(
            Capabilities::new()
                .with(Direction::Read, Feature::Samples)
                .with(Direction::Read, Feature::CorrectionGrids),
        )
        .reader(reader)
        .without_writer("V16 is read-only: the library never writes it")
        .without_patcher("V16 is read-only: the library never patches it")
        .build()
        .expect("the V16 adapter set is whole")
}

/// The reader: the build of the mount, as a source of metadata and of samples. The source holds the mount, so the file stays open for as long as the source does.
fn reader(mount: &Group) -> Result<Box<dyn BuildSource>, Error> {
    let build = Arc::new(read::read_build(mount)?);
    let locator = V16Locator::new(Arc::clone(&build), mount.clone());
    Ok(Box::new(SampleSource::new(build, locator)))
}
