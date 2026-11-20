//! The spot size of a laser from a file: the accessors (IP-27 phase 27J, decisions D27-8 and D27-24).
//!
//! * **Layer:** Processing. It reads through the Model's `BuildSource`, so it runs on any source (a file, an archive, the mock) and needs no HDF5.
//! * **Requirement:** BL-PR-03 (the result says its stages, its algorithm version and where each parameter came from), BL-PR-04, V:QE-04 (a laser that cannot be calculated says why, and the others
//!   still compute).
//!
//! # The two accessors
//!
//! * [`spot_size_beam`]: **metadata only.** The resolved beam of a laser, or why it has none. No sample is read. For the warnings when a file opens and for a settings panel.
//! * [`spot_size`](fn@spot_size): **the spot size of one laser in one layer**, for a [`SpotSizeRequest`]. It chooses the laser's rows once with the request's `ReadSelection` (27C), reads the commanded and the actual
//!   Z through them, converts them to millimetres with the calibration the file and the user give (27D), shifts them in time by the scanner card's offsets (27F), resolves the beam (27H) and
//!   calculates the spot (27I). The result is a [`LaserSpotSize`]: the series with **the native index of each row**, the beam with the origin of each value, the timing and the Z calibration as
//!   they were resolved, the algorithm version, the stage the Z reached and the findings. **Or the laser is unavailable, with the reason** ([`SpotSizeUnavailable`]).
//!
//! Both return `Result<Result<_, Unavailable>, Error>`: the outer `Error` is a real failure (a corrupt file, a failed read, a laser the build does not hold); the inner one is an expected
//! absence, which a caller shows and goes on from.
//!
//! # `spot_size` does not walk the layers for a laser that the layer records (findings of two reviews, 2026-11-19 and 2026-11-20)
//!
//! A lazy source reads a layer when it is asked for, but [`BuildSource::build`] **reads every layer the first time**. [`spot_size`](fn@spot_size) takes the laser's channels, and whether the laser is
//! recorded, from the **one layer** that is asked about, and a recorded laser's parameters from the build's **overview** (the lasers that the calibration names, which costs no layer).
//! **A name in the overview is not proof that the build holds the laser:** the overview lists what the calibration names and the build holds what a layer records, so a laser that only the
//! calibration names is a laser that the build does not hold (the error is part of the contract). So `build()` is called for **a laser that the layer does not record**, to tell one that the file
//! records elsewhere from one that it does not hold at all, and for **a recorded laser that the overview names under another name of the same number** (a group `Laser_01` for a calibration of
//! `Laser_1`: the readers attach a calibration by number), which only the build resolves. Both are rare, and the walk is paid once. [`spot_size_beam`] has **no layer** to prove that the laser exists, so
//! it always asks the build; a caller that lists the calibration's lasers (a settings panel) resolves their beams from the overview's parameters with [`resolve_beam_of`].
//!
//! # No defaults here (decisions D27-18 and D27-24)
//!
//! Everything the file does not say is **the caller's**, in one explicit, immutable and hashable bundle, [`SpotSizeAssumptions`], which is part of the request's hash: the beam's seven answers
//! ([`BeamAssumptions`]), the timing's ([`TimingFallbacks`]), the Z's ([`ZAssumptions`]: the values for a calibration the file lacks, and the narrower-dataset rule), how a sample period with no
//! unit is read ([`UnitlessPeriod`]) and the time of the first sample. A value filled in from them has the origin [`Origin::CallerSupplied`]. **The mirroring of a commanded timing override onto
//! the actual offset is the caller's too:** the request carries the overrides as the user set them.
//!
//! # What Vision's behaviour becomes here, and where it differs (listed, for phase 27L)
//!
//! * **The rows are each Z series' own kept rows.** Vision keeps the shortest of the streams it reads (X, Y and the rest). The two agree wherever the channels of a laser are as long as each other,
//!   which is every miniature; where they are not, the commanded and the actual series have their own lengths, and [`SpotSizeFinding::ZLengthsDiffer`] says so.
//! * **A laser with no Z channel is unavailable.** Vision fills the missing series with zeros, and a spot from a Z of zero is a number with no meaning.
//! * **The native sample period under a stride is `(period x stride) / stride`**, as Vision takes it (it stores `period x stride` and divides again), which is one unit in the last place off the plain
//!   period for some strides. A quirk of version 1, kept so that the two paths agree to the last bit; the owner decides at 27L.
//! * **A series of fewer than two samples is not shifted** (Vision's use case skips such a laser); [`SpotSizeFinding::NotShifted`] says so and the stage stays physical.

use std::borrow::Cow;

use build_log_model::{
    Axis, BuildMeta, BuildOverview, BuildSource, ChannelDescriptor, ChannelKind, ColumnRequest,
    Error, ErrorKind, LaserKey, LaserParameters, LayerId, NativeIndex, Origin, Precision, Quantity,
    ReadContext, ReadSelection, SelectedColumn, SpotSizeOverride, TimingOverride,
    ZCalibrationOverride,
};

use crate::beam::{resolve_beam_of, BeamAssumptions, BeamUnavailable, ResolvedBeam};
use crate::calibration::{resolve_z_calibration, Resolved, StoredWidths};
use crate::spot_size::{spot_size_from_z, SpotSeries, SpotSize};
use crate::stage::Stage;
use crate::time_shift::{shift_series, timestamps, ShiftRefusal};
use crate::timing_offsets::{resolve_timing, RecordedTiming, TimingFallbacks, TimingResolution};
use crate::z_conversion::{ZConverter, ZRefusal, ZSeries};

// ---------------------------------------------------------------- what the caller decides

/// What the caller decides about the Z where the file is silent. Every field is required; the library has no default for any of them.
// constructed-by: caller-options
#[derive(Debug, Clone, Copy)]
pub struct ZAssumptions {
    /// The Z field size, in millimetres, when neither the user nor the file gives one. Vision: 400.
    pub field_size_mm: f64,
    /// The Z offset, in millimetres, when the file has none (there is no override of it). Vision: 0.
    pub offset_mm: f64,
    /// The bit resolution of a Z series when neither the user nor the file gives one. Vision: 20.
    pub bit_resolution: i64,
    /// Whether a series is converted at the width of its stored type when that is smaller than its calibrated resolution (a 16-bit dataset under a 20-bit calibration is converted at 16).
    /// Vision: yes.
    pub narrow_to_stored_width: bool,
}

impl ZAssumptions {
    fn key(&self) -> (u64, u64, i64, bool) {
        (
            self.field_size_mm.to_bits(),
            self.offset_mm.to_bits(),
            self.bit_resolution,
            self.narrow_to_stored_width,
        )
    }
}

impl PartialEq for ZAssumptions {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for ZAssumptions {}

impl std::hash::Hash for ZAssumptions {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// How a sample period whose unit the file does not state (a V17 file writes `10.0`) is read. A period whose unit the file states (`10us`) is converted by it.
// closed: a bare number is seconds, microseconds, or one or the other by its size (Vision's reading)
#[derive(Debug, Clone, Copy)]
pub enum UnitlessPeriod {
    /// A number of seconds.
    Seconds,
    /// A number of microseconds.
    Microseconds,
    /// A number of microseconds when it is **above** the threshold and of seconds when it is not. Vision: above 0.001.
    MicrosecondsAbove(f64),
}

impl UnitlessPeriod {
    fn key(&self) -> (u8, u64) {
        match self {
            UnitlessPeriod::Seconds => (0, 0),
            UnitlessPeriod::Microseconds => (1, 0),
            UnitlessPeriod::MicrosecondsAbove(threshold) => (2, threshold.to_bits()),
        }
    }
}

impl PartialEq for UnitlessPeriod {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for UnitlessPeriod {}

impl std::hash::Hash for UnitlessPeriod {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// **Everything the file does not say about a spot size**, as the caller answers it: explicit, immutable and hashable, and part of the request's hash. The library has no default for any part.
// constructed-by: caller-options
#[derive(Debug, Clone, Copy)]
pub struct SpotSizeAssumptions {
    /// The beam's seven answers.
    pub beam: BeamAssumptions,
    /// The timing's answers: the offsets and the sample period when the file has none, an offset that cannot be read, an unknown unit, a protocol the library has no name for.
    pub timing: TimingFallbacks,
    /// The Z's answers.
    pub z: ZAssumptions,
    /// How a sample period with no unit is read.
    pub sample_period: UnitlessPeriod,
    /// The time, in seconds, of the first sample of the laser's stream. Vision: 0 (it reads `Start_Time`, which no V17 file has).
    pub time_origin_s: f64,
}

// `f64` has no `Eq` and no `Hash`, so the bundle is compared by the bits of its time origin
impl SpotSizeAssumptions {
    #[allow(clippy::type_complexity)]
    fn key(
        &self,
    ) -> (
        BeamAssumptions,
        TimingFallbacks,
        ZAssumptions,
        UnitlessPeriod,
        u64,
    ) {
        (
            self.beam,
            self.timing,
            self.z,
            self.sample_period,
            self.time_origin_s.to_bits(),
        )
    }
}

impl PartialEq for SpotSizeAssumptions {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for SpotSizeAssumptions {}

impl std::hash::Hash for SpotSizeAssumptions {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

// ---------------------------------------------------------------- the request

/// The spot size of one laser in one layer, asked for: an immutable value that compares and hashes (so it can key a cache).
// constructed-by: caller-options
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpotSizeRequest {
    layer: LayerId,
    laser: LaserKey,
    selection: ReadSelection,
    timing: TimingOverride,
    z: ZCalibrationOverride,
    beam: SpotSizeOverride,
    assumptions: SpotSizeAssumptions,
    beam_from: Option<LaserKey>,
}

impl SpotSizeRequest {
    /// A request for every row of the laser, with no overrides. **The caller's answers are required:** there is no request without them.
    pub fn new(layer: LayerId, laser: LaserKey, assumptions: SpotSizeAssumptions) -> Self {
        SpotSizeRequest {
            layer,
            laser,
            selection: ReadSelection::new(),
            timing: TimingOverride::new(),
            z: ZCalibrationOverride::new(),
            beam: SpotSizeOverride::new(),
            assumptions,
            beam_from: None,
        }
    }

    /// The rows to calculate: a stride and which rows of the laser's state to keep.
    pub fn with_selection(mut self, selection: ReadSelection) -> Self {
        self.selection = selection;
        self
    }

    /// The user's timing offsets, **as the user set them** (the mirroring of a commanded override onto the actual offset is the caller's).
    pub fn with_timing_override(mut self, overrides: TimingOverride) -> Self {
        self.timing = overrides;
        self
    }

    /// The user's overrides of the Z calibration.
    pub fn with_z_override(mut self, overrides: ZCalibrationOverride) -> Self {
        self.z = overrides;
        self
    }

    /// The user's overrides of the beam.
    pub fn with_beam_override(mut self, overrides: SpotSizeOverride) -> Self {
        self.beam = overrides;
        self
    }

    /// The beam of **another laser**: the parameters of this laser are used for the beam instead of the requested laser's own. The library never picks one: this is how a caller whose laser has no
    /// train asks again (Vision uses the first train's beam).
    pub fn with_beam_from(mut self, laser: LaserKey) -> Self {
        self.beam_from = Some(laser);
        self
    }

    pub fn layer(&self) -> LayerId {
        self.layer
    }

    pub fn laser(&self) -> &LaserKey {
        &self.laser
    }

    pub fn selection(&self) -> &ReadSelection {
        &self.selection
    }

    pub fn timing_override(&self) -> &TimingOverride {
        &self.timing
    }

    pub fn z_override(&self) -> &ZCalibrationOverride {
        &self.z
    }

    pub fn beam_override(&self) -> &SpotSizeOverride {
        &self.beam
    }

    pub fn assumptions(&self) -> &SpotSizeAssumptions {
        &self.assumptions
    }

    pub fn beam_from(&self) -> Option<&LaserKey> {
        self.beam_from.as_ref()
    }
}

// ---------------------------------------------------------------- what comes out

/// Why a laser has no spot size. An expected absence, with its reason: a caller shows it and goes on with the other lasers.
// open-domain: the reasons grow with the stages; a consumer matches on the ones it knows
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SpotSizeUnavailable {
    /// The laser has no beam: no waist, no train, or a Rayleigh length that cannot be derived.
    Beam(BeamUnavailable),
    /// The laser is not recorded in the layer.
    LaserNotInLayer,
    /// The laser has no Z channel of this series in the layer. (Vision fills it with zeros; a spot from a Z of zero has no meaning.)
    NoZChannel { series: ZSeries },
    /// A Z series cannot be converted: a calibration that is not a number, or a bit resolution outside `1..=32`. The commanded series is checked first.
    ZRefused { series: ZSeries, refusal: ZRefusal },
    /// The selection keeps only laser-on or only laser-off rows, and the laser has no state channel in the layer to say which they are.
    FilterNeedsState,
    /// The selection keeps no row of the laser's Z (an empty laser, or a filter that keeps nothing).
    NoRows,
}

impl std::fmt::Display for SpotSizeUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpotSizeUnavailable::Beam(reason) => write!(f, "no beam: {reason}"),
            SpotSizeUnavailable::LaserNotInLayer => {
                f.write_str("the laser is not recorded in the layer")
            }
            SpotSizeUnavailable::NoZChannel { series } => {
                write!(f, "the laser has no {} Z channel in the layer", series_word(*series))
            }
            SpotSizeUnavailable::ZRefused { refusal, .. } => {
                write!(f, "the Z cannot be converted: {refusal}")
            }
            SpotSizeUnavailable::FilterNeedsState => f.write_str(
                "the selection keeps laser-on or laser-off rows and the laser has no state channel in the layer",
            ),
            SpotSizeUnavailable::NoRows => f.write_str("the selection keeps no row of the laser"),
        }
    }
}

impl std::error::Error for SpotSizeUnavailable {}

fn series_word(series: ZSeries) -> &'static str {
    match series {
        ZSeries::Commanded => "commanded",
        ZSeries::Actual => "actual",
    }
}

/// Something about the Z that a person looking at the spot size would want to know.
// open-domain: the observations grow with the stages; a consumer matches on the ones it knows
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SpotSizeFinding {
    /// The commanded and the actual Z have different numbers of kept rows, and each series has its own length.
    ZLengthsDiffer { commanded: usize, actual: usize },
    /// A series of fewer than two samples is not shifted in time (a line needs two), as Vision does not shift it.
    NotShifted { series: ZSeries, samples: usize },
}

/// The Z calibration that was used, with where each value came from: the user's override, the file's value, or the caller's answer for a value the file lacks (and the narrower-dataset rule).
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct ZUsed {
    field_size_mm: Resolved<f64>,
    offset_mm: Resolved<f64>,
    commanded_resolution: Resolved<i64>,
    actual_resolution: Resolved<i64>,
}

impl ZUsed {
    /// The Z field size, in millimetres.
    pub fn field_size_mm(&self) -> &Resolved<f64> {
        &self.field_size_mm
    }

    /// The Z offset, in millimetres (there is no override of it).
    pub fn offset_mm(&self) -> &Resolved<f64> {
        &self.offset_mm
    }

    /// The bit resolution that the commanded Z was converted at: the calibration's, **or the stored width when that is narrower and the caller's rule says so** (the origin is then the caller's).
    pub fn commanded_resolution(&self) -> &Resolved<i64> {
        &self.commanded_resolution
    }

    /// The bit resolution that the actual Z was converted at.
    pub fn actual_resolution(&self) -> &Resolved<i64> {
        &self.actual_resolution
    }
}

/// The spot size of one laser in one layer, for the rows that a selection kept.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct LaserSpotSize {
    spot: SpotSize,
    commanded_rows: NativeIndex,
    actual_rows: NativeIndex,
    timing: TimingResolution,
    z: ZUsed,
    period: Resolved<f64>,
    stage: Stage,
    findings: Vec<SpotSizeFinding>,
}

impl LaserSpotSize {
    /// The spot at each commanded sample.
    pub fn commanded(&self) -> &SpotSeries {
        self.spot.commanded()
    }

    /// The spot at each actual sample.
    pub fn actual(&self) -> &SpotSeries {
        self.spot.actual()
    }

    /// The native sample index in the file of each commanded sample.
    pub fn commanded_rows(&self) -> &NativeIndex {
        &self.commanded_rows
    }

    /// The native sample index in the file of each actual sample.
    pub fn actual_rows(&self) -> &NativeIndex {
        &self.actual_rows
    }

    /// The orientation of the major axis in the build frame, in `[0, 180)` degrees.
    pub fn axis_angle_deg(&self) -> f64 {
        self.spot.axis_angle_deg()
    }

    /// The beam the spot was calculated from, with the origin of each value and what was found when it was resolved.
    pub fn beam(&self) -> &ResolvedBeam {
        self.spot.beam()
    }

    /// The version of the algorithm: [`crate::SPOT_SIZE_ALGORITHM_VERSION`].
    pub fn algorithm_version(&self) -> u32 {
        self.spot.algorithm_version()
    }

    /// The last stage the Z reached: [`Stage::Shifted`], or [`Stage::Physical`] where it was not shifted ([`SpotSizeFinding::NotShifted`]).
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The timing as resolved: the native period, the sign of the card and the two offsets, each with its origin.
    pub fn timing(&self) -> &TimingResolution {
        &self.timing
    }

    /// The Z calibration as used, each value with its origin.
    pub fn z(&self) -> &ZUsed {
        &self.z
    }

    /// The sample period as recorded, in seconds (**not** under a stride): the file's own when it states a unit, the caller's reading when it does not, and the caller's answer when the file has none.
    pub fn period(&self) -> &Resolved<f64> {
        &self.period
    }

    /// What was found about the Z.
    pub fn findings(&self) -> &[SpotSizeFinding] {
        &self.findings
    }
}

// ---------------------------------------------------------------- the pieces

fn not_present(what: &str) -> Error {
    Error::new(ErrorKind::NotPresent {
        what: what.to_string(),
    })
}

/// The parameters of a laser that the build holds, or the error for one that it does not.
fn parameters_of<'a>(
    build: &'a BuildMeta,
    laser: &LaserKey,
) -> Result<Option<&'a LaserParameters>, Error> {
    let meta = build
        .laser(laser)
        .ok_or_else(|| not_present("the laser, which the build does not hold"))?;
    Ok(meta.parameters.as_ref())
}

/// The parameters of a laser: **from the overview when the layer at hand records the laser, from the build otherwise.**
///
/// The overview lists the lasers that the build's *calibration* names, which a lazy source gives without reading a layer. Those are not the build's lasers, which are the ones that a layer
/// records: **a name in the overview is not proof that the build holds the laser**, and the error for a laser it does not hold is part of the accessors' contract.
///
/// * `recorded` is true when the caller has seen the laser in a layer, so that it exists and the overview can say whether the calibration gives it parameters: the overview's laser of the same key
///   has them (or none), and a laser the overview does not name at all has none. **A laser that the overview names under another name of the same number** (the file's group is `Laser_01` and
///   the calibration says `Laser_1`; the readers attach a calibration by number) is left to the build, which knows.
/// * Otherwise nothing proves that the build holds the laser, so the build is asked (`build()`, which reads every layer the first time on a lazy source): it says whether the laser is recorded
///   anywhere, with the parameters it holds, or not at all, which is an error.
fn parameters_in<'a>(
    source: &dyn BuildSource,
    overview: &'a BuildOverview,
    laser: &LaserKey,
    recorded: bool,
) -> Result<Option<Cow<'a, LaserParameters>>, Error> {
    if recorded {
        if let Some(named) = overview
            .lasers
            .iter()
            .find(|candidate| &candidate.key == laser)
        {
            return Ok(named.parameters.as_ref().map(Cow::Borrowed));
        }
        if !overview
            .lasers
            .iter()
            .any(|candidate| candidate.key.number == laser.number)
        {
            return Ok(None);
        }
    }
    let build = source.build()?;
    Ok(parameters_of(&build, laser)?.cloned().map(Cow::Owned))
}

/// The channel of a laser that holds the Z of a series.
fn z_channel(channels: &[ChannelDescriptor], series: ZSeries) -> Option<&ChannelDescriptor> {
    let kind = match series {
        ZSeries::Commanded => ChannelKind::CommandedPosition { axis: Axis::Z },
        ZSeries::Actual => ChannelKind::ActualPosition { axis: Axis::Z },
    };
    channels.iter().find(|channel| channel.kind == kind)
}

/// A value from the file or the user, or the caller's answer for one the file lacks.
fn or_callers(found: Option<&Resolved<f64>>, callers: f64) -> Resolved<f64> {
    match found {
        Some(resolved) => resolved.clone(),
        None => Resolved::new(callers, Some(Origin::CallerSupplied)),
    }
}

/// The bit resolution a Z series is converted at: the calibration's (the user's, the file's, or the caller's answer), **narrowed to the stored width** when the caller's rule says so and the stored
/// type is narrower. Vision's rule, in Vision's order.
fn resolution_used(
    calibrated: Option<&Resolved<i64>>,
    stored_width: Option<u32>,
    z: &ZAssumptions,
) -> Resolved<i64> {
    let base = match calibrated {
        Some(resolved) => resolved.clone(),
        None => Resolved::new(z.bit_resolution, Some(Origin::CallerSupplied)),
    };
    match stored_width {
        Some(width) if z.narrow_to_stored_width && i64::from(width) < base.value() => {
            Resolved::new(i64::from(width), Some(Origin::CallerSupplied))
        }
        _ => base,
    }
}

/// A sample period in seconds, and whether the file stated its unit. A unit the file states is applied (`s`, `ms`, `us`, `ns`); a number with no unit is read by the caller's rule.
fn read_period(quantity: &Quantity, unitless: UnitlessPeriod) -> (f64, bool) {
    match quantity.unit.id.as_deref() {
        Some("s") => (quantity.value, true),
        Some("ms") => (quantity.value * 1e-3, true),
        Some("us") => (quantity.value * 1e-6, true),
        Some("ns") => (quantity.value * 1e-9, true),
        _ => (
            match unitless {
                UnitlessPeriod::Seconds => quantity.value,
                UnitlessPeriod::Microseconds => quantity.value * 1e-6,
                UnitlessPeriod::MicrosecondsAbove(threshold) => {
                    if quantity.value > threshold {
                        quantity.value * 1e-6
                    } else {
                        quantity.value
                    }
                }
            },
            false,
        ),
    }
}

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The kept values of a Z channel, read through the laser's rows, at `f64`.
fn read_z(
    source: &dyn BuildSource,
    layer: LayerId,
    laser: &LaserKey,
    channel: &ChannelDescriptor,
    rows: &build_log_model::LaserRows,
    context: &ReadContext,
) -> Result<(Vec<f64>, NativeIndex), Error> {
    let request = ColumnRequest::new(layer, Some(laser.clone()), channel.key.clone())
        .with_precision(Precision::F64);
    let SelectedColumn {
        values,
        native_index,
        ..
    } = source.read_rows(&request, rows, context)?;
    let codes = values
        .as_f64()
        .ok_or_else(|| corrupt("a Z read at f64 did not come back as f64".to_string()))?
        .to_vec();
    Ok((codes, native_index))
}

/// A Z series shifted in time by its own offset, or as it is when it has fewer than two samples (the finding says so).
fn shifted(
    timing: &TimingResolution,
    series: ZSeries,
    times: &[f64],
    z_mm: Vec<f64>,
    findings: &mut Vec<SpotSizeFinding>,
) -> Result<(Vec<f64>, bool), Error> {
    let kind = match series {
        ZSeries::Commanded => ChannelKind::CommandedPosition { axis: Axis::Z },
        ZSeries::Actual => ChannelKind::ActualPosition { axis: Axis::Z },
    };
    match shift_series(timing, &kind, times, &z_mm) {
        Ok(shifted) => Ok((shifted.into_values(), true)),
        Err(ShiftRefusal::TooFewSamples { samples }) => {
            findings.push(SpotSizeFinding::NotShifted { series, samples });
            Ok((z_mm, false))
        }
        Err(other) => Err(corrupt(format!(
            "the {} Z could not be shifted: {other}",
            series_word(series)
        ))),
    }
}

// ---------------------------------------------------------------- the accessors

/// The beam of a laser: **metadata only, no sample is read.** `beam_from` names another laser whose parameters are used instead of the laser's own (the library never picks a train for a laser that
/// has none). `Ok(Err(..))` is a laser with no beam, with the reason; `Err(..)` is a laser that the build does not hold (**a laser that only the calibration names is not held**).
///
/// With no layer to prove that the laser exists, **it asks the build** (`build()`, which reads every layer the first time on a lazy source). A caller that lists the calibration's lasers and has
/// their parameters in the overview resolves each beam with [`resolve_beam_of`], which reads nothing.
pub fn spot_size_beam(
    source: &dyn BuildSource,
    laser: &LaserKey,
    beam_from: Option<&LaserKey>,
    overrides: &SpotSizeOverride,
    assumptions: &BeamAssumptions,
) -> Result<Result<ResolvedBeam, BeamUnavailable>, Error> {
    let build = source.build()?;
    // the laser itself must be there, whichever laser the beam is taken from
    parameters_of(&build, laser)?;
    let parameters = parameters_of(&build, beam_from.unwrap_or(laser))?;
    Ok(resolve_beam_of(parameters, overrides, assumptions))
}

/// The spot size of one laser in one layer. See the module documentation for what it does, what it returns and where it differs from Vision's behaviour.
///
/// `Ok(Err(..))` is a laser with no spot size, with the reason (a caller shows it and goes on); `Err(..)` is a real failure: a layer or a laser the build does not hold, a corrupt file or a failed
/// read. The checks are in this order, and the first that fails is the reason: the laser is in the layer, it has a beam, it has both Z channels, the Z can be converted (the commanded series first),
/// the selection can be applied to it, and it keeps a row of each Z series.
pub fn spot_size(
    source: &dyn BuildSource,
    request: &SpotSizeRequest,
    context: &ReadContext,
) -> Result<Result<LaserSpotSize, SpotSizeUnavailable>, Error> {
    let assumptions = request.assumptions();
    let layer = request.layer();
    let laser = request.laser();
    // the layer, and whether the laser is recorded in it (the one layer is read, and no other)
    let detail = source.layer_meta(layer)?;
    let found = detail
        .lasers
        .iter()
        .find(|candidate| &candidate.key == laser);
    // the parameters of a laser that the layer records come from the overview: no layer is walked (see the module documentation)
    let overview = source.overview()?;
    let own_parameters = parameters_in(source, &overview, laser, found.is_some())?;
    let own = own_parameters.as_deref();
    let beam_laser = request.beam_from().unwrap_or(laser);
    let beam_recorded = detail
        .lasers
        .iter()
        .any(|candidate| &candidate.key == beam_laser);
    let beam_laser_parameters = parameters_in(source, &overview, beam_laser, beam_recorded)?;
    let beam_parameters = beam_laser_parameters.as_deref();

    // the laser is recorded in the layer
    let Some(found) = found else {
        return Ok(Err(SpotSizeUnavailable::LaserNotInLayer));
    };

    // the beam: metadata only, so it is settled before any sample is read
    let beam = match resolve_beam_of(beam_parameters, request.beam_override(), &assumptions.beam) {
        Ok(beam) => beam,
        Err(reason) => return Ok(Err(SpotSizeUnavailable::Beam(reason))),
    };

    // both Z channels, the commanded one first
    let Some(commanded_channel) = z_channel(&found.channels, ZSeries::Commanded) else {
        return Ok(Err(SpotSizeUnavailable::NoZChannel {
            series: ZSeries::Commanded,
        }));
    };
    let Some(actual_channel) = z_channel(&found.channels, ZSeries::Actual) else {
        return Ok(Err(SpotSizeUnavailable::NoZChannel {
            series: ZSeries::Actual,
        }));
    };

    // the Z calibration: the user's override over the file's value, the caller's answer for what neither gives, and a converter for each series
    let calibration = resolve_z_calibration(own, request.z_override());
    let widths = StoredWidths::of_channels(&found.channels);
    let field_size = or_callers(calibration.field_size_mm(), assumptions.z.field_size_mm);
    let offset = or_callers(calibration.offset_mm(), assumptions.z.offset_mm);
    let commanded_resolution = resolution_used(
        calibration.commanded_resolution(),
        widths.commanded,
        &assumptions.z,
    );
    let actual_resolution = resolution_used(
        calibration.actual_resolution(),
        widths.actual,
        &assumptions.z,
    );
    let converter = |series, resolution: &Resolved<i64>| {
        ZConverter::new(
            series,
            field_size.value(),
            offset.value(),
            resolution.value(),
        )
    };
    let commanded_converter = match converter(ZSeries::Commanded, &commanded_resolution) {
        Ok(converter) => converter,
        Err(refusal) => {
            return Ok(Err(SpotSizeUnavailable::ZRefused {
                series: ZSeries::Commanded,
                refusal,
            }))
        }
    };
    let actual_converter = match converter(ZSeries::Actual, &actual_resolution) {
        Ok(converter) => converter,
        Err(refusal) => {
            return Ok(Err(SpotSizeUnavailable::ZRefused {
                series: ZSeries::Actual,
                refusal,
            }))
        }
    };

    // the rows of the laser, chosen once, and the Z through them
    let rows = match source.laser_rows(layer, laser, request.selection(), context) {
        Ok(rows) => rows,
        Err(error) if matches!(error.kind(), ErrorKind::NotPresent { .. }) => {
            return Ok(Err(SpotSizeUnavailable::FilterNeedsState))
        }
        Err(error) => return Err(error),
    };
    let (commanded_codes, commanded_rows) =
        read_z(source, layer, laser, commanded_channel, &rows, context)?;
    let (actual_codes, actual_rows) = read_z(source, layer, laser, actual_channel, &rows, context)?;
    if commanded_codes.is_empty() || actual_codes.is_empty() {
        return Ok(Err(SpotSizeUnavailable::NoRows));
    }
    let commanded_mm: Vec<f64> = commanded_codes
        .iter()
        .map(|code| commanded_converter.to_mm(*code))
        .collect();
    let actual_mm: Vec<f64> = actual_codes
        .iter()
        .map(|code| actual_converter.to_mm(*code))
        .collect();

    // the timing: the offsets from the card and the user, in seconds and in the shifter's convention
    let recorded_period = own
        .and_then(|p| p.scanner_card.sample_period.as_ref())
        .or(found.entry.time_axis.period.as_ref());
    let read = recorded_period.map(|quantity| read_period(quantity, assumptions.sample_period));
    let stride = request.selection().stride().max(1) as f64;
    let mut recorded = RecordedTiming::from_parameters(own);
    if let Some((seconds, stated)) = read {
        // the native period under a stride is `(period x stride) / stride`, as Vision takes it (a quirk of version 1: see the module documentation)
        recorded = recorded.with_native_period_s((seconds * stride) / stride);
        if !stated {
            recorded = recorded.with_native_period_origin(Origin::CallerSupplied);
        }
    }
    let timing = resolve_timing(&recorded, request.timing_override(), &assumptions.timing);
    let (period_s, period_origin) = match read {
        Some((seconds, true)) => (seconds, None),
        Some((seconds, false)) => (seconds, Some(Origin::CallerSupplied)),
        None => (
            assumptions.timing.sample_period_s,
            Some(Origin::CallerSupplied),
        ),
    };

    // the time of each kept row is `origin + native index x period` (the period as recorded, not under a stride), and each series is shifted by its own offset
    let mut findings = Vec::new();
    let commanded_times = timestamps(&commanded_rows, assumptions.time_origin_s, period_s);
    let actual_times = timestamps(&actual_rows, assumptions.time_origin_s, period_s);
    let (commanded_z, commanded_shifted) = shifted(
        &timing,
        ZSeries::Commanded,
        &commanded_times,
        commanded_mm,
        &mut findings,
    )?;
    let (actual_z, actual_shifted) = shifted(
        &timing,
        ZSeries::Actual,
        &actual_times,
        actual_mm,
        &mut findings,
    )?;
    if commanded_z.len() != actual_z.len() {
        findings.push(SpotSizeFinding::ZLengthsDiffer {
            commanded: commanded_z.len(),
            actual: actual_z.len(),
        });
    }

    let spot = spot_size_from_z(&beam, &commanded_z, &actual_z);
    Ok(Ok(LaserSpotSize {
        spot,
        commanded_rows,
        actual_rows,
        timing,
        z: ZUsed {
            field_size_mm: field_size,
            offset_mm: offset,
            commanded_resolution,
            actual_resolution,
        },
        period: Resolved::new(period_s, period_origin),
        stage: if commanded_shifted && actual_shifted {
            Stage::Shifted
        } else {
            Stage::Physical
        },
        findings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beam::OneWaistMissing;
    use crate::beam_units::{AngleUnit, LengthUnit};
    use crate::timing_offsets::{CardConvention, UnknownUnitAs};
    use std::collections::HashSet;

    fn assumptions() -> SpotSizeAssumptions {
        SpotSizeAssumptions {
            beam: BeamAssumptions {
                waist_unit: LengthUnit::Micrometre,
                length_unit: LengthUnit::Millimetre,
                angle_unit: AngleUnit::Degree,
                wavelength_unit: LengthUnit::Nanometre,
                absent_value: 0.0,
                m2_when_none: 1.0,
                one_waist_missing: OneWaistMissing::AssumeCircular,
            },
            timing: TimingFallbacks {
                sample_period_s: 1e-5,
                absent_offset_s: 0.0,
                unparsed_offset_s: 0.0,
                unknown_unit: UnknownUnitAs::Steps,
                unrecognised_protocol: CardConvention::OppositeToShifter,
            },
            z: ZAssumptions {
                field_size_mm: 400.0,
                offset_mm: 0.0,
                bit_resolution: 20,
                narrow_to_stored_width: true,
            },
            sample_period: UnitlessPeriod::MicrosecondsAbove(0.001),
            time_origin_s: 0.0,
        }
    }

    fn q(value: f64, unit: Option<&str>) -> Quantity {
        Quantity::from_file(value, unit)
    }

    #[test]
    fn a_period_with_a_unit_is_converted_by_it_and_one_with_none_is_read_by_the_callers_rule() {
        let rule = UnitlessPeriod::MicrosecondsAbove(0.001);
        // V16 writes `10us`: the unit is the file's
        assert_eq!(read_period(&q(10.0, Some("us")), rule), (10.0 * 1e-6, true));
        assert_eq!(read_period(&q(2.0, Some("ms")), rule), (2.0 * 1e-3, true));
        assert_eq!(read_period(&q(1e-5, Some("s")), rule), (1e-5, true));
        assert_eq!(
            read_period(&q(100.0, Some("ns")), rule),
            (100.0 * 1e-9, true)
        );
        // V17 writes `10.0`: above the threshold it is microseconds, below it seconds
        assert_eq!(read_period(&q(10.0, None), rule), (10.0 * 1e-6, false));
        assert_eq!(read_period(&q(1e-5, None), rule), (1e-5, false));
        assert_eq!(
            read_period(&q(0.001, None), rule),
            (0.001, false),
            "the threshold itself is not above it"
        );
        // another caller reads the same numbers otherwise
        assert_eq!(
            read_period(&q(10.0, None), UnitlessPeriod::Seconds),
            (10.0, false)
        );
        assert_eq!(
            read_period(&q(10.0, None), UnitlessPeriod::Microseconds),
            (10.0 * 1e-6, false)
        );
        // a unit the table does not know is no unit
        assert_eq!(
            read_period(&q(10.0, Some("fortnights")), rule),
            (10.0 * 1e-6, false)
        );
    }

    #[test]
    fn a_series_is_converted_at_the_stored_width_only_when_that_is_narrower_and_the_caller_says_so()
    {
        let z = assumptions().z;
        let twenty = Resolved::new(20, None);
        // narrower: the width, and the origin is the caller's rule
        let narrowed = resolution_used(Some(&twenty), Some(16), &z);
        assert_eq!(narrowed.value(), 16);
        assert_eq!(narrowed.origin(), Some(&Origin::CallerSupplied));
        // as wide or wider: the calibration's, with its own origin
        for width in [20, 32] {
            let kept = resolution_used(Some(&twenty), Some(width), &z);
            assert_eq!((kept.value(), kept.origin()), (20, None));
        }
        // no calibration: the caller's answer, narrowed if the type is narrower than it
        let missing = resolution_used(None, Some(32), &z);
        assert_eq!(
            (missing.value(), missing.origin()),
            (20, Some(&Origin::CallerSupplied))
        );
        assert_eq!(resolution_used(None, Some(16), &z).value(), 16);
        // a caller that does not narrow gets the calibration whatever the type
        let mut keep = z;
        keep.narrow_to_stored_width = false;
        assert_eq!(resolution_used(Some(&twenty), Some(16), &keep).value(), 20);
        // no stored width known: the calibration
        assert_eq!(resolution_used(Some(&twenty), None, &z).value(), 20);
    }

    #[test]
    fn a_value_the_file_lacks_is_the_callers_answer_and_says_so() {
        let from_file = Resolved::new(76.5, None);
        assert_eq!(or_callers(Some(&from_file), 400.0), from_file);
        let callers = or_callers(None, 400.0);
        assert_eq!(
            (callers.value(), callers.origin()),
            (400.0, Some(&Origin::CallerSupplied))
        );
    }

    #[test]
    fn a_request_compares_and_hashes_and_cannot_be_made_without_the_callers_answers() {
        let laser = LaserKey::new("Laser_1", 1);
        let base = SpotSizeRequest::new(LayerId(1), laser.clone(), assumptions());
        assert_eq!(base, base.clone());
        // every part of the request is part of its identity
        let mut variants = vec![
            base.clone()
                .with_selection(ReadSelection::new().with_stride(3)),
            base.clone()
                .with_timing_override(TimingOverride::new().with_commanded_offset_steps(2.0)),
            base.clone()
                .with_z_override(ZCalibrationOverride::new().with_field_size_z_mm(100.0)),
            base.clone()
                .with_beam_override(SpotSizeOverride::new().with_waist_major_um(60.0)),
            base.clone().with_beam_from(LaserKey::new("Laser_2", 2)),
            SpotSizeRequest::new(LayerId(2), laser.clone(), assumptions()),
            SpotSizeRequest::new(LayerId(1), LaserKey::new("Laser_2", 2), assumptions()),
        ];
        let mut other = assumptions();
        other.time_origin_s = 1.5;
        variants.push(SpotSizeRequest::new(LayerId(1), laser.clone(), other));
        let mut other = assumptions();
        other.z.bit_resolution = 16;
        variants.push(SpotSizeRequest::new(LayerId(1), laser, other));
        let mut seen: HashSet<SpotSizeRequest> = HashSet::new();
        seen.insert(base.clone());
        for variant in variants {
            assert_ne!(variant, base);
            assert!(
                seen.insert(variant),
                "two requests that differ hashed alike"
            );
        }
        assert_eq!(seen.len(), 10);
        // the getters give back what was asked for
        assert_eq!(base.layer(), LayerId(1));
        assert_eq!(base.laser().number, 1);
        assert!(base.beam_from().is_none());
        assert_eq!(base.selection(), &ReadSelection::new());
    }

    #[test]
    fn the_bundle_of_answers_compares_and_hashes_by_the_bits() {
        let a = assumptions();
        let mut b = assumptions();
        b.time_origin_s = -0.0;
        assert_ne!(a, b, "0.0 and -0.0 are different answers");
        let mut nan = assumptions();
        nan.sample_period = UnitlessPeriod::MicrosecondsAbove(f64::NAN);
        assert_eq!(nan, nan);
        let set: HashSet<SpotSizeAssumptions> = [a, b, nan, a].into_iter().collect();
        assert_eq!(set.len(), 3);
        // the three ways to read a bare period are three answers
        let periods: HashSet<UnitlessPeriod> = [
            UnitlessPeriod::Seconds,
            UnitlessPeriod::Microseconds,
            UnitlessPeriod::MicrosecondsAbove(0.001),
            UnitlessPeriod::MicrosecondsAbove(0.002),
        ]
        .into_iter()
        .collect();
        assert_eq!(periods.len(), 4);
    }

    #[test]
    fn a_reason_says_what_is_missing_and_a_z_channel_is_found_by_its_kind() {
        let reasons = [
            SpotSizeUnavailable::Beam(BeamUnavailable::NoWaist),
            SpotSizeUnavailable::LaserNotInLayer,
            SpotSizeUnavailable::NoZChannel {
                series: ZSeries::Actual,
            },
            SpotSizeUnavailable::ZRefused {
                series: ZSeries::Commanded,
                refusal: ZRefusal::ResolutionOutOfRange {
                    name: "commanded Z",
                    resolution: 0,
                },
            },
            SpotSizeUnavailable::FilterNeedsState,
            SpotSizeUnavailable::NoRows,
        ];
        let words: Vec<String> = reasons.iter().map(ToString::to_string).collect();
        assert!(words[0].contains("no beam"));
        assert!(words[2].contains("actual Z channel"));
        assert!(words[3].contains("outside 1..=32"));
        assert!(words[4].contains("no state channel"));
        assert!(words[5].contains("no row"));
        // no channels, no Z
        assert!(z_channel(&[], ZSeries::Commanded).is_none());
        fn assert_traits<T: std::error::Error + Send + Sync + Clone + 'static>() {}
        assert_traits::<SpotSizeUnavailable>();
    }
}
