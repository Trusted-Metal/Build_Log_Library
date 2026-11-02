//! The V17 adapter (IP-13): **one adapter for every flavor of V17**, key `"17"`, the version that tauri writes.
//!
//! * **One folder, one reader.** The files of V17 come from different tauri releases (the current output of August to October 2026, and the March 2026
//!   output); they differ in how a value is stored and not in what it means, so one tolerant reader serves them all and no detector is needed (decision
//!   D13-2). The dispatcher sees one version.
//! * **[`layout`] holds every name** of the version, and no other file of this folder spells one (BL-ST-13); every path is relative to the mount.
//! * **[`values`]** reads a value in whichever form V17 stored it; **[`read`]** builds the Model's types from it, with **[`layers`]** for the layers; **[`source`]** is what the registry hands to
//!   a consumer.
//! * **It reads what each phase of IP-13 has reached:** the build level (13B), the layers (13C), the lasers with their channels (13D), their parameters, outcomes, images and events
//!   (13H, 13J, 13F, 13K) and **the samples of the lasers (13L, through [`columns`] and the column engine)**. The rest of what a file holds is kept in `extra` and never dropped. The writer and the patcher are declared unsupported until IP-08 and IP-11.
//! * **It never names another version's module** (BL-VA-05; `tests/adapter_isolation.rs`).

pub mod columns;
mod context;
mod event_columns;
pub mod events;
mod image_regions;
pub mod images;
pub mod lasers;
pub mod layers;
pub mod layout;
pub mod parameters;
pub mod read;
pub mod source;
pub mod values;

use build_log_model::{BuildSource, Capabilities, Direction, Error, Feature};
use hdf5::Group;

use super::{AdapterSet, MarkerPlace};
use source::V17Source;

/// The version this adapter reads.
pub const KEY: &str = "17";
/// The adapter's name in a version report.
pub const NAME: &str = "v17";

/// The registration unit of V17: the place of its marker, what it can do (the metadata, and the samples of the lasers), and its reader.
pub fn adapter_set() -> AdapterSet {
    AdapterSet::builder(KEY, NAME, MarkerPlace::new(layout::MOUNT, layout::MARKER))
        .capabilities(
            Capabilities::new()
                .with(Direction::Read, Feature::Outcomes)
                .with(Direction::Read, Feature::Images)
                .with(Direction::Read, Feature::Events)
                .with(Direction::Read, Feature::Samples)
                .with(Direction::Read, Feature::EventColumns)
                .with(Direction::Read, Feature::ImageRegions),
        )
        .reader(reader)
        .without_writer("the V17 writer arrives with IP-08")
        .without_patcher("the V17 patcher arrives with IP-11")
        .build()
        .expect("the V17 adapter set is whole")
}

/// The reader: the build of the mount, as a source of metadata.
fn reader(mount: &Group) -> Result<Box<dyn BuildSource>, Error> {
    let build = read::read_build(mount)?;
    Ok(Box::new(V17Source::new(build, mount.clone())))
}
