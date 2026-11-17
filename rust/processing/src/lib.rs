//! The **Processing layer**: computation derived from Model data (decimation, corrections, conversions, level of detail).
//!
//! * **Layer:** above the Model, beside the IO layer. It works on Model values only, so it can run without HDF5.
//! * **May depend on:** `build-log-model`. In tests only: `build-log-testkit` and `build-log-conformance`.
//! * **Must not depend on:** `build-log-io` (directly or through another crate) or the `build-log` facade.
//!
//! The architecture guards in `rust/build-log/tests` enforce the rules above.
//!
//! # What is here
//!
//! * [`calibration`]: **the Z calibration of a laser as the file and the user give it** (IP-27 phase 27D): the user's override over the file's value, each with its origin, and **an absent value
//!   reported as absent**. There are no defaults here.
//! * [`z_conversion`]: **the linear conversion of Z from bits to millimetres**, in Vision's order of operations, built from explicit numbers, with the reason when a series cannot be converted.
//!   Only Z is built.
//!
//! **The library reports and the caller decides what a report means** (decision D27-18): an absent value is reported as absent, and Vision supplies its own defaults, applies its own rules and
//! decides what a refused series means for a laser. The rest of the stages (the time shift, spot size, decimation and corrections) arrive with their own phases.

#![forbid(unsafe_code)]

pub mod calibration;
pub mod z_conversion;

pub use calibration::{resolve_z_calibration, Resolved, StoredWidths, ZCalibration};
pub use z_conversion::{
    bits_to_mm, ZConverter, ZRefusal, ZSeries, MAX_BIT_RESOLUTION, MIN_BIT_RESOLUTION,
};
