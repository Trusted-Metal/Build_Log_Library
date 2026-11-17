//! What Vision does with the library's reports, written once for the tests that run Vision's vectors on the miniatures (IP-27 phases 27D and 27F; decision D27-18).
//!
//! **Everything in this module is Vision's and not the library's**, and it is made here as the adapter of phase 27K will make it: Vision's defaults for a Z calibration, its
//! narrower-dataset rule, which lasers it keeps, how it reads a sample period whose unit the file does not state, and what it does with a laser whose Z cannot be converted. The library
//! reports each fact (an absent value, a stored width, a refusal) and decides none of them.
//!
//! A test names a miniature, a layer by its position, a selection and a Z override; [`kept_layer`] opens the file, chooses the rows of each laser once (27C), resolves the calibration (27D),
//! converts the Z codes to millimetres, and gives each laser Vision keeps with what a later stage needs.

#![allow(dead_code)] // each test that uses this module uses a part of it

use build_log::testing::miniature_path;
use build_log::{
    open, Axis, BuildSource, ChannelKind, ColumnRequest, ErrorKind, LaserKey, LaserParameters,
    LayerId, NativeIndex, Precision, Quantity, ReadContext, ReadSelection, StateRows,
    ZCalibrationOverride,
};
use build_log_processing::{resolve_z_calibration, Resolved, StoredWidths, ZConverter, ZSeries};

use build_log::{Shared, TimingOverride};
use build_log_processing::{
    resolve_timing, shift_series, timestamps, CardConvention, RecordedTiming, ShiftRefusal,
    TimingFallbacks, TimingResolution, UnknownUnitAs,
};

/// Vision's defaults for a Z calibration that the file and the user do not give (Vision's `CalibrationParams::default()`).
pub const VISION_FIELD_SIZE_Z_MM: f64 = 400.0;
pub const VISION_OFFSET_Z_MM: f64 = 0.0;
pub const VISION_BIT_RESOLUTION: i64 = 20;

/// Vision's narrower-dataset rule: a series is converted at the width of its stored type when that is smaller than the calibrated resolution (itself Vision's 20 when none is given).
pub fn vision_resolution(calibrated: Option<i64>, stored_width: Option<u32>) -> i64 {
    let calibrated = calibrated.unwrap_or(VISION_BIT_RESOLUTION);
    match stored_width {
        Some(width) if i64::from(width) < calibrated => i64::from(width),
        _ => calibrated,
    }
}

/// Vision's reading of a sample period, in seconds. A period whose unit the file states is converted by it (V16 writes `10us`); **a number with no unit is read as Vision reads it: a value above
/// 0.001 is microseconds, a smaller one is already seconds** (V17 writes `10.0`).
pub fn vision_period_seconds(period: &Quantity) -> f64 {
    match period.unit.id.as_deref() {
        Some("s") => period.value,
        Some("ms") => period.value * 1e-3,
        Some("us") => period.value * 1e-6,
        Some("ns") => period.value * 1e-9,
        _ if period.value > 0.001 => period.value * 1e-6,
        _ => period.value,
    }
}

pub fn rows_of(text: &str) -> StateRows {
    match text {
        "all" => StateRows::All,
        "on" => StateRows::OnOnly,
        "off" => StateRows::OffOnly,
        other => panic!("rows is all, on or off, not {other:?}"),
    }
}

/// What the test reads of one laser: the channels of the laser in the layer, and the rows chosen for them.
struct Laser<'a> {
    source: &'a dyn BuildSource,
    layer: LayerId,
    key: LaserKey,
    channels: Vec<build_log::ChannelDescriptor>,
    rows: build_log::LaserRows,
}

impl Laser<'_> {
    /// The values of the channel of this kind at `f64` for the rows chosen, and the native index of each; `None` if the laser has no such channel.
    fn read(&self, kind: ChannelKind) -> Option<(Vec<f64>, NativeIndex)> {
        let channel = self.channels.iter().find(|c| c.kind == kind)?;
        let request = ColumnRequest::new(self.layer, Some(self.key.clone()), channel.key.clone())
            .with_precision(Precision::F64);
        let cut = self
            .source
            .read_rows(&request, &self.rows, &ReadContext::new())
            .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        Some((
            cut.values.as_f64().expect("a read at f64").to_vec(),
            cut.native_index,
        ))
    }

    /// The length of a stream of positions: the shorter of its X and Y, when both have rows after the selection, and the native index of its rows.
    fn stream(&self, kind: fn(Axis) -> ChannelKind) -> Option<(usize, NativeIndex)> {
        let (x, index) = self.read(kind(Axis::X))?;
        let (y, _) = self.read(kind(Axis::Y))?;
        (!x.is_empty() && !y.is_empty()).then(|| (x.len().min(y.len()), index))
    }
}

/// A laser that Vision keeps, with its Z in millimetres.
pub struct KeptLaser {
    /// The position of the laser in the layer's list.
    pub index: usize,
    /// The file's own name of the laser (`Laser_1`), which Vision's messages use.
    pub name: String,
    /// How many rows Vision keeps: the shortest of the streams that were asked for.
    pub n: usize,
    /// The commanded Z in millimetres, `n` values (zeros where the laser has fewer codes).
    pub commanded_mm: Vec<f64>,
    /// The actual Z in millimetres, `n` values.
    pub actual_mm: Vec<f64>,
    /// The native index of each of the `n` kept rows.
    pub native_index: NativeIndex,
    /// What the file records for the laser's calibration, if it has a snapshot.
    pub parameters: Option<LaserParameters>,
    /// The sample period as the laser records it (the snapshot's, else its time axis's), whatever its unit.
    pub recorded_period: Option<Quantity>,
}

pub struct KeptLayer {
    pub layer_id: String,
    pub lasers: Vec<KeptLaser>,
}

/// A miniature's layer read with a selection, as Vision keeps it: **a laser is kept when a stream that was asked for has its X and Y rows**; a filter on a laser that has no state drops
/// that laser; **a laser whose Z cannot be converted is dropped whole, X and Y included, the commanded series being checked first**; and a Z series with fewer than `n` codes is zeros.
pub fn kept_layer(
    miniature: &str,
    position: usize,
    selection: &ReadSelection,
    z_override: &ZCalibrationOverride,
) -> KeptLayer {
    let opened = open(miniature_path(miniature)).expect("the miniature opens");
    let source = opened.source();
    let layer = source.layers().expect("the layers")[position];
    let detail = source.layer_meta(layer).expect("the layer");
    let build = source.build().expect("the build");

    let mut lasers = Vec::new();
    for (index, found) in detail.lasers.iter().enumerate() {
        // the rows of the laser; a filter on a laser that has no state is refused, and Vision drops that laser
        let rows = match source.laser_rows(layer, &found.key, selection, &ReadContext::new()) {
            Ok(rows) => rows,
            Err(e) if matches!(e.kind(), ErrorKind::NotPresent { .. }) => continue,
            Err(e) => panic!("laser {}: {e}", found.key.number),
        };
        let laser = Laser {
            source,
            layer,
            key: found.key.clone(),
            channels: found.channels.clone(),
            rows,
        };
        // a laser is in the answer when a stream that was asked for has its X and Y rows, and n is the shortest of them
        let streams: Vec<(usize, NativeIndex)> = [
            laser.stream(|axis| ChannelKind::CommandedPosition { axis }),
            laser.stream(|axis| ChannelKind::ActualPosition { axis }),
        ]
        .into_iter()
        .flatten()
        .collect();
        let Some(n) = streams.iter().map(|(len, _)| *len).min() else {
            continue;
        };
        let native_index = streams[0].1.clone();

        // what the library reports: the user's override over the file's value, or nothing; and the stored width of each Z dataset
        let parameters = build.laser(&found.key).and_then(|l| l.parameters.as_ref());
        let calibration = resolve_z_calibration(parameters, z_override);
        let widths = StoredWidths::of_channels(&found.channels);
        // what Vision does with it: its defaults, its narrower-dataset rule, and the converter of each series
        let field_size = calibration
            .field_size_mm()
            .map_or(VISION_FIELD_SIZE_Z_MM, Resolved::value);
        let offset = calibration
            .offset_mm()
            .map_or(VISION_OFFSET_Z_MM, Resolved::value);
        let commanded = ZConverter::new(
            ZSeries::Commanded,
            field_size,
            offset,
            vision_resolution(
                calibration.commanded_resolution().map(Resolved::value),
                widths.commanded,
            ),
        );
        let actual = ZConverter::new(
            ZSeries::Actual,
            field_size,
            offset,
            vision_resolution(
                calibration.actual_resolution().map(Resolved::value),
                widths.actual,
            ),
        );
        // Vision's decision: a series that cannot be converted drops the whole laser, the commanded one being checked first
        let (Ok(commanded), Ok(actual)) = (commanded, actual) else {
            continue;
        };
        let convert = |kind: ChannelKind, converter: ZConverter| match laser.read(kind) {
            // a series with at least n codes is converted, cut to n; a shorter or missing one is zeros
            Some((codes, _)) if codes.len() >= n => {
                codes[..n].iter().map(|c| converter.to_mm(*c)).collect()
            }
            _ => vec![0.0; n],
        };
        lasers.push(KeptLaser {
            index,
            name: found.key.key.clone(),
            n,
            commanded_mm: convert(ChannelKind::CommandedPosition { axis: Axis::Z }, commanded),
            actual_mm: convert(ChannelKind::ActualPosition { axis: Axis::Z }, actual),
            native_index,
            parameters: parameters.cloned(),
            recorded_period: parameters
                .and_then(|p| p.scanner_card.sample_period.clone())
                .or_else(|| found.entry.time_axis.period.clone()),
        });
    }
    KeptLayer {
        layer_id: format!("Layer_{layer}"),
        lasers,
    }
}

// ---------------------------------------------------------------- the time shift, as Vision applies it (IP-27 phase 27F)

/// Vision's answers where the file is silent about the timing (the same decisions as `rust/processing/tests/time_shift_vectors.rs` carries for the pure cases).
pub fn vision_fallbacks() -> TimingFallbacks {
    TimingFallbacks {
        sample_period_s: 1e-5,
        absent_offset_s: 0.0,
        unparsed_offset_s: 0.0,
        unknown_unit: UnknownUnitAs::Steps,
        unrecognised_protocol: CardConvention::OppositeToShifter,
    }
}

/// Vision's mirroring: a commanded override alone is also the actual offset.
pub fn vision_mirrored(user: &TimingOverride) -> TimingOverride {
    let mut mirrored = user.clone();
    if user.commanded_offset_steps.is_some() && user.actual_offset_steps.is_none() {
        mirrored.actual_offset_steps = user.commanded_offset_steps;
    }
    mirrored
}

/// The native sample period under a stride as Vision takes it: `(native x stride) / stride`, which is one unit in the last place off the plain period for some strides.
pub fn vision_native_period(native_s: f64, stride: u64) -> f64 {
    let stride = stride.max(1) as f64;
    (native_s * stride) / stride
}

/// What the library resolves for one laser's timing: what the file records, the native period as Vision reads it, and the user's overrides as Vision mirrors them.
pub fn timing_of(
    laser: &KeptLaser,
    period_s: Option<f64>,
    stride: u64,
    user: &TimingOverride,
) -> TimingResolution {
    let mut recorded = RecordedTiming::from_parameters(laser.parameters.as_ref());
    if let Some(period) = period_s {
        recorded = recorded.with_native_period_s(vision_native_period(period, stride));
    }
    resolve_timing(&recorded, &vision_mirrored(user), &vision_fallbacks())
}

/// The shifted Z of a laser in millimetres. A laser of fewer than two samples is not shifted (Vision's use case skips it).
fn shifted(
    timing: &TimingResolution,
    kind: ChannelKind,
    times: &[f64],
    values: &[f64],
) -> Vec<f64> {
    match shift_series(timing, &kind, times, values) {
        Ok(series) => series.into_values(),
        Err(ShiftRefusal::TooFewSamples { .. }) => values.to_vec(),
        Err(other) => panic!("{kind:?}: {other}"),
    }
}

/// A laser's timestamps and its Z after the time shift.
pub struct ShiftedLaser {
    /// The time of each kept row: origin 0 (Vision reads `Start_Time`, which no miniature has), the native index and the period of the file.
    pub times: Vec<f64>,
    /// The commanded Z in millimetres, shifted by the commanded offset.
    pub commanded: Vec<f64>,
    /// The actual Z in millimetres, shifted by the actual offset.
    pub actual: Vec<f64>,
}

/// A kept laser's Z, shifted as Vision's Tier 1 shifts it: its timestamps, its timing resolved from the file and the user's overrides, and each Z series by its own offset.
pub fn shift_laser(laser: &KeptLaser, stride: u64, user: &TimingOverride) -> ShiftedLaser {
    let period = laser.recorded_period.as_ref().map(vision_period_seconds);
    let first_n: Vec<u64> = laser.native_index.iter().take(laser.n).collect();
    let times = timestamps(
        &NativeIndex::Listed(Shared::new(first_n)),
        0.0,
        period.unwrap_or(1e-5),
    );
    let timing = timing_of(laser, period, stride, user);
    let commanded = shifted(
        &timing,
        ChannelKind::CommandedPosition { axis: Axis::Z },
        &times,
        &laser.commanded_mm,
    );
    let actual = shifted(
        &timing,
        ChannelKind::ActualPosition { axis: Axis::Z },
        &times,
        &laser.actual_mm,
    );
    ShiftedLaser {
        times,
        commanded,
        actual,
    }
}
