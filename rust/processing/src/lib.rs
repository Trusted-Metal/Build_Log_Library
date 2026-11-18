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
//! * [`stage`]: **the names of the stages** that a laser's data goes through (raw, physical, shifted, corrected), so that a result can say which it is at (decision D27-21).
//! * [`timing_offsets`]: **the scanner card's timing offsets of a laser** in seconds and in the shifter's convention (IP-27 phase 27F): what the format means (a bare number is native steps,
//!   a text may carry a unit, the sign depends on the galvo protocol) is applied here, and **what the file does not say is reported and answered by the caller's explicit
//!   [`TimingFallbacks`]**.
//! * [`beam_units`]: **the units of the beam's stored values** and how a stored number is brought to the unit the resolver works in: Vision's table and arithmetic, with the caller's unit where the file
//!   states none (IP-27 phase 27H).
//! * [`beam`]: **the beam model of a laser** resolved from its stored values and the user's overrides, with the origin of each value and structured findings; what the file does not say is answered by the
//!   caller's explicit [`BeamAssumptions`] (IP-27 phase 27H, decision D27-23).
//! * [`spot_size`](mod@spot_size): **the spot size** of a laser at every sample: the defocus and the Gaussian-beam kernel, Vision's CPU path to the last bit, with the version of the algorithm (IP-27 phase 27I).
//! * [`spot_size_access`]: **the spot size of a laser from a file**, through any `BuildSource`: the request, the caller's explicit bundle of answers, and the two accessors (IP-27 phase 27J,
//!   decision D27-24).
//! * [`time_shift`]: **the time shift** of a position series by its offset, with the timestamps it needs: Vision's algorithm, to the last bit, behind a stage that shifts only positions.
//!
//! **The library reports and the caller decides what a report means** (decision D27-18): an absent value is reported as absent, and Vision supplies its own defaults, applies its own rules and
//! decides what a refused series means for a laser. The rest of the stages (spot size, decimation and corrections) arrive with their own phases.

#![forbid(unsafe_code)]

pub mod beam;
pub mod beam_units;
pub mod calibration;
pub mod spot_size;
pub mod spot_size_access;
pub mod stage;
pub mod time_shift;
pub mod timing_offsets;
pub mod z_conversion;

pub use beam::{
    resolve_beam, resolve_beam_of, BeamAssumptions, BeamAxis, BeamFinding, BeamInput,
    BeamUnavailable, OneWaistMissing, RecordedBeam, ResolvedBeam, ThermalTest,
};
pub use beam_units::{
    angle_multiplier, convert_angle, convert_length, length_multiplier, wavelength_in_nanometres,
    AngleUnit, LengthUnit,
};
pub use calibration::{resolve_z_calibration, Resolved, StoredWidths, ZCalibration};
pub use spot_size::{
    actual_defocus, commanded_defocus, compute_spot_size, propagate, spot_size_from_z, BeamShape,
    SpotKernelInput, SpotSeries, SpotSize, SpotSizeResult, SPOT_SIZE_ALGORITHM_VERSION,
};
pub use spot_size_access::{
    spot_size, spot_size_beam, LaserSpotSize, SpotSizeAssumptions, SpotSizeFinding,
    SpotSizeRequest, SpotSizeUnavailable, UnitlessPeriod, ZAssumptions, ZUsed,
};
pub use stage::Stage;
pub use time_shift::{
    shift_channel, shift_series, timestamps, ShiftRefusal, ShiftedSeries, SHIFT_FIT_N,
};
pub use timing_offsets::{
    apply_override, card_convention, parse_offset_text, resolve_timing, CardConvention,
    OffsetReading, ParsedOffset, RecordedOffset, RecordedTiming, ResolvedOffset, TimingFallbacks,
    TimingResolution, UnknownUnitAs,
};
pub use z_conversion::{
    bits_to_mm, ZConverter, ZRefusal, ZSeries, MAX_BIT_RESOLUTION, MIN_BIT_RESOLUTION,
};
