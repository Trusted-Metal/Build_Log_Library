//! The time shift of a laser's position series, and the timestamps it needs (IP-27 phase 27F).
//!
//! * **Layer:** Processing. It works on Model values and plain numbers only, so it runs without HDF5.
//! * **Requirement:** BL-PR-03 (a result says the stage that was applied and where each parameter came from), BL-PR-04 (the convention is fixed once, documented and tested).
//!
//! # What it does (algorithm version 1: Vision's, bit for bit)
//!
//! The scanner card records how late each position series is relative to the samples. A series is **resampled on a time axis moved by its offset**: the sample at `t` takes the value that
//! the signal has at `t + offset` ([`shift_channel`]). Where `t + offset` is inside the axis the value is **interpolated linearly** between the two samples that enclose it (or is the stored
//! sample, when it falls on one); where it is not, it is **extrapolated** by the least-squares line through the first (or last) [`SHIFT_FIT_N`] samples. The results are the same as
//! Vision's, to the last bit; the vectors in `testdata/vectors/time_shift.json` pin them.
//!
//! * **The shift is after the selection, on the kept rows, by their timestamps** ([`timestamps`]: `origin + native index x native period`). A state filter leaves gaps in the axis; a
//!   position whose shifted time falls in a gap is interpolated across it.
//! * **The commanded positions are shifted by the commanded offset and the actual positions by the actual one** ([`shift_series`]). **Nothing else is shifted:** power and state are the true
//!   laser-on signal, and a request to shift a series that is not a position is refused.
//! * **Which unit the offsets and the period are in is not decided here:** see [`crate::timing_offsets`] for what the format means and what the caller decides.
//!
//! # Known behaviours of version 1 that are kept (marked `quirk` in Vision's spec of the chunk; the owner decides at phase 27L whether version 2 changes them)
//!
//! * A sample whose shifted time is **exactly at the first or the last timestamp** is extrapolated, not read.
//! * The axis is taken as uniform when the last timestamp is within `dt x 1e-6` of `first + dt x (n - 1)`; a **large absolute time origin** (1.7e9 s) quantises the axis so that it is not,
//!   and the other path is taken, which fits the extrapolation in large coordinates.
//! * A **NaN** in a value spreads to the samples that interpolate from it, and a NaN in the window of a fit makes every extrapolated sample NaN.
//!
//! # What it refuses, and who decides what that means
//!
//! Where Vision's code would index past a slice (one sample moved backward) or does something that hides a mistake (series of different lengths are returned unchanged), the library
//! **refuses with a reason** ([`ShiftRefusal`]) and the caller decides what that means (decision D27-18). Vision's use case skips a laser of fewer than two samples, and its adapter will.

use std::fmt;

use build_log_model::{ChannelKind, NativeIndex};

use crate::stage::Stage;
use crate::timing_offsets::{ResolvedOffset, TimingResolution};

/// The number of boundary samples that the extrapolation at the ends is fitted to (algorithm version 1).
pub const SHIFT_FIT_N: usize = 50;

/// Why a series is not shifted.
// open-domain: the reasons grow with the stages; a consumer matches on the ones it knows
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ShiftRefusal {
    /// The series and its timestamps are not the same length. (Vision returns the series unchanged.)
    LengthMismatch { timestamps: usize, values: usize },
    /// There are fewer samples than the two that a line needs. (Vision's shifter would index past its slice for one sample moved backward, and its use case skips such a laser.)
    TooFewSamples { samples: usize },
    /// The series is not a position: only positions carry the card's timing offset, and power and state are never shifted.
    NotAPosition { kind: ChannelKind },
}

impl fmt::Display for ShiftRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShiftRefusal::LengthMismatch { timestamps, values } => write!(
                f,
                "the series has {values} values and {timestamps} timestamps: they must be the same length"
            ),
            ShiftRefusal::TooFewSamples { samples } => write!(
                f,
                "{samples} sample(s) cannot be shifted: a line needs two"
            ),
            ShiftRefusal::NotAPosition { kind } => write!(
                f,
                "{kind:?} is not a position: only positions are shifted, and power and state never are"
            ),
        }
    }
}

impl std::error::Error for ShiftRefusal {}

// ---------------------------------------------------------------- the timestamps

/// The time of each kept row, in seconds: `origin + native index x native period` (the same operations as Vision's). **The index is the native one**, so a strided or filtered row keeps
/// its true time. **The origin is the caller's:** Vision uses the `Start_Time` attribute of the laser's group when there is one and `0` otherwise, and the V17 files write
/// `Start_Laser_Time`, so every V17 laser starts at `0` in Vision.
pub fn timestamps(native_index: &NativeIndex, origin_s: f64, native_period_s: f64) -> Vec<f64> {
    native_index
        .iter()
        .map(|index| origin_s + index as f64 * native_period_s)
        .collect()
}

// ---------------------------------------------------------------- the shifter

/// A least-squares line `y = a + b x`: the intercept and the slope. One point, or a column of x that does not vary, gives a flat line at the mean.
fn ols_fit(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    if n < 2.0 {
        return (ys.first().copied().unwrap_or(0.0), 0.0);
    }
    let x_mean = xs.iter().sum::<f64>() / n;
    let y_mean = ys.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for (&x, &y) in xs.iter().zip(ys.iter()) {
        let dx = x - x_mean;
        num += dx * (y - y_mean);
        den += dx * dx;
    }
    if den.abs() < 1e-30 {
        return (y_mean, 0.0);
    }
    let slope = num / den;
    let intercept = y_mean - slope * x_mean;
    (intercept, slope)
}

/// The line through the first `fit_count` samples, or `None` when there are not that many.
fn left_fit(timestamps: &[f64], values: &[f64], fit_count: usize) -> Option<(f64, f64)> {
    (fit_count <= timestamps.len()).then(|| ols_fit(&timestamps[..fit_count], &values[..fit_count]))
}

/// The line through the last `fit_count` samples.
fn right_fit(timestamps: &[f64], values: &[f64], fit_count: usize) -> (f64, f64) {
    let start = timestamps.len().saturating_sub(fit_count);
    ols_fit(&timestamps[start..], &values[start..])
}

/// Linear interpolation between the samples that enclose `t`, or the stored sample when `t` is on one.
fn interpolate(timestamps: &[f64], values: &[f64], t: f64) -> f64 {
    let j = match timestamps.binary_search_by(|ts| ts.total_cmp(&t)) {
        Ok(index) => return values[index],
        Err(index) => index,
    };
    if j == 0 {
        return values[0];
    }
    if j >= timestamps.len() {
        return values[timestamps.len() - 1];
    }
    let t0 = timestamps[j - 1];
    let t1 = timestamps[j];
    let dt = t1 - t0;
    if dt.abs() < 1e-30 {
        return values[j - 1];
    }
    let fraction = (t - t0) / dt;
    values[j - 1] + (values[j] - values[j - 1]) * fraction
}

/// A series moved in time by `shift_seconds` (positive: forward, so that `t' = t + shift_seconds` takes the value the signal has there): interpolated inside the axis, extrapolated at its ends
/// by the least-squares line through the first or last `fit_n` samples (`fit_n` is taken as at least 2 and at most the number of samples). **The same results as Vision's shifter, to the last
/// bit.** A shift smaller than `1e-15` s returns the values as they are. Refuses what is said at [`ShiftRefusal`].
pub fn shift_channel(
    timestamps: &[f64],
    values: &[f64],
    shift_seconds: f64,
    fit_n: usize,
) -> Result<Vec<f64>, ShiftRefusal> {
    let n = timestamps.len();
    if n != values.len() {
        return Err(ShiftRefusal::LengthMismatch {
            timestamps: n,
            values: values.len(),
        });
    }
    if shift_seconds.abs() < 1e-15 {
        return Ok(values.to_vec());
    }
    let fit_count = fit_n.min(n).max(2);

    // an axis of constant step is shifted by index arithmetic, any other by a search for each sample
    if n >= 2 {
        let dt = timestamps[1] - timestamps[0];
        if dt > 0.0 {
            let expected_last = timestamps[0] + dt * (n - 1) as f64;
            if (timestamps[n - 1] - expected_last).abs() < dt * 1e-6 {
                return Ok(shift_uniform(
                    timestamps,
                    values,
                    shift_seconds,
                    fit_count,
                    dt,
                ));
            }
        }
    }

    let mut left: Option<(f64, f64)> = None;
    let mut right: Option<(f64, f64)> = None;
    let mut result = Vec::with_capacity(n);
    for &t in timestamps {
        let shifted = t + shift_seconds;
        if shifted <= timestamps[0] {
            let (a, b) = match left {
                Some(line) => line,
                None => {
                    let line = left_fit(timestamps, values, fit_count)
                        .ok_or(ShiftRefusal::TooFewSamples { samples: n })?;
                    left = Some(line);
                    line
                }
            };
            result.push(a + b * shifted);
        } else if shifted >= timestamps[n - 1] {
            let (a, b) = *right.get_or_insert_with(|| right_fit(timestamps, values, fit_count));
            result.push(a + b * shifted);
        } else {
            result.push(interpolate(timestamps, values, shifted));
        }
    }
    Ok(result)
}

/// The shift of an axis of constant step: the position of a shifted time on it is found by arithmetic, and the two lines of the ends are fitted once.
fn shift_uniform(
    timestamps: &[f64],
    values: &[f64],
    shift_seconds: f64,
    fit_count: usize,
    dt: f64,
) -> Vec<f64> {
    let n = timestamps.len();
    let first = timestamps[0];
    let last = timestamps[n - 1];
    let (left_a, left_b) = ols_fit(&timestamps[..fit_count], &values[..fit_count]);
    let (right_a, right_b) = right_fit(timestamps, values, fit_count);
    let mut result = Vec::with_capacity(n);
    for &t in timestamps {
        let shifted = t + shift_seconds;
        if shifted <= first {
            result.push(left_a + left_b * shifted);
        } else if shifted >= last {
            result.push(right_a + right_b * shifted);
        } else {
            let fractional_index = (shifted - first) / dt;
            let j = fractional_index as usize;
            let fraction = fractional_index - j as f64;
            if fraction.abs() < 1e-12 {
                result.push(values[j]);
            } else {
                let next = (j + 1).min(n - 1);
                result.push(values[j] + (values[next] - values[j]) * fraction);
            }
        }
    }
    result
}

// ---------------------------------------------------------------- the stage

/// A position series that the time shift was applied to: the values, the stage it is at, and the offset it was shifted by with where that came from.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct ShiftedSeries {
    kind: ChannelKind,
    values: Vec<f64>,
    offset: ResolvedOffset,
}

impl ShiftedSeries {
    /// The stage of the values: always [`Stage::Shifted`].
    pub fn stage(&self) -> Stage {
        Stage::Shifted
    }

    /// Which series this is.
    pub fn kind(&self) -> &ChannelKind {
        &self.kind
    }

    /// The shifted values, one for each timestamp.
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    pub fn into_values(self) -> Vec<f64> {
        self.values
    }

    /// The offset the series was shifted by, in seconds, with its origin (the file's, the user's or the caller's) and how the file's own offset was read.
    pub fn offset(&self) -> &ResolvedOffset {
        &self.offset
    }
}

/// One position series of a laser shifted by the offset that belongs to it: **a commanded position by the commanded offset, an actual position by the actual one.** `timestamps` are those of
/// the kept rows ([`timestamps`]) and `values` the series at those rows, in the unit it is already in (the shift does not change it).
///
/// Refuses a series that is not a position ([`ShiftRefusal::NotAPosition`]: power and state are never shifted), fewer than two samples ([`ShiftRefusal::TooFewSamples`]: Vision skips such a
/// laser) and a series of another length than the timestamps ([`ShiftRefusal::LengthMismatch`]).
pub fn shift_series(
    timing: &TimingResolution,
    kind: &ChannelKind,
    timestamps: &[f64],
    values: &[f64],
) -> Result<ShiftedSeries, ShiftRefusal> {
    let offset = match kind {
        ChannelKind::CommandedPosition { .. } => timing.commanded(),
        ChannelKind::ActualPosition { .. } => timing.actual(),
        other => {
            return Err(ShiftRefusal::NotAPosition {
                kind: other.clone(),
            })
        }
    };
    if timestamps.len() < 2 {
        return Err(ShiftRefusal::TooFewSamples {
            samples: timestamps.len(),
        });
    }
    let shifted = shift_channel(timestamps, values, offset.seconds().value(), SHIFT_FIT_N)?;
    Ok(ShiftedSeries {
        kind: kind.clone(),
        values: shifted,
        offset: offset.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing_offsets::{
        resolve_timing, CardConvention, RecordedOffset, RecordedTiming, TimingFallbacks,
        UnknownUnitAs,
    };
    use build_log_model::{Axis, Origin, Shared, TimingOverride};

    fn axis(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 1e-5).collect()
    }

    fn ramp(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64).collect()
    }

    #[test]
    fn a_zero_shift_and_one_below_the_floor_return_the_values_as_they_are() {
        let (t, v) = (axis(10), ramp(10));
        assert_eq!(shift_channel(&t, &v, 0.0, 5).unwrap(), v);
        assert_eq!(shift_channel(&t, &v, 9e-16, 5).unwrap(), v);
        assert_ne!(shift_channel(&t, &v, 2e-15, 5).unwrap(), v);
    }

    #[test]
    fn a_shift_of_one_sample_forward_takes_the_next_stored_value_and_the_end_is_extrapolated() {
        let (t, v) = (axis(20), ramp(20));
        let shifted = shift_channel(&t, &v, 1e-5, 5).unwrap();
        assert_eq!(&shifted[..18], &v[1..19]);
        // the last sample lands exactly on the last timestamp and is extrapolated, not read (the test is `>=`): the line is exact here, so it comes out as the next integer, within rounding
        assert!((shifted[19] - 20.0).abs() < 1e-9, "{}", shifted[19]);
    }

    #[test]
    fn a_fraction_of_a_sample_interpolates_linearly() {
        let (t, v) = (axis(10), ramp(10));
        let shifted = shift_channel(&t, &v, 0.5e-5, 3).unwrap();
        for (i, got) in shifted.iter().take(9).enumerate() {
            assert!((got - (i as f64 + 0.5)).abs() < 1e-9, "{i}: {got}");
        }
    }

    #[test]
    fn a_backward_shift_extrapolates_the_start_from_the_first_samples() {
        let (t, v) = (axis(10), ramp(10));
        let shifted = shift_channel(&t, &v, -2e-5, 4).unwrap();
        // a ramp is its own line: extrapolation continues it
        for (i, got) in shifted.iter().enumerate() {
            assert!((got - (i as f64 - 2.0)).abs() < 1e-9, "{i}: {got}");
        }
    }

    #[test]
    fn the_general_path_and_the_uniform_path_give_the_same_function() {
        // the same values on an axis with a step in it are not uniform; every sample off the step must agree with the uniform answer where the two axes coincide
        let (t, v) = (axis(30), ramp(30));
        let uniform = shift_channel(&t, &v, 3.5e-5, 10).unwrap();
        let mut bent = t.clone();
        bent[29] += 1e-6;
        let general = shift_channel(&bent, &v, 3.5e-5, 10).unwrap();
        for i in 0..20 {
            assert!((uniform[i] - general[i]).abs() < 1e-9, "{i}");
        }
    }

    #[test]
    fn a_gap_in_the_axis_is_interpolated_across() {
        // rows 3 and 4 were filtered out, so the axis jumps from 2e-5 to 5e-5. The sample at 1e-5 moved forward by 2e-5 lands at 3e-5, inside the gap, and is interpolated between its
        // neighbours across it: 20 + (50 - 20) x (1e-5 / 3e-5) = 30
        let t = [0.0, 1e-5, 2e-5, 5e-5, 6e-5, 7e-5];
        let v = [0.0, 10.0, 20.0, 50.0, 60.0, 70.0];
        let shifted = shift_channel(&t, &v, 2e-5, 3).unwrap();
        assert!((shifted[1] - 30.0).abs() < 1e-9, "{}", shifted[1]);
    }

    #[test]
    fn a_nan_value_spreads_to_the_two_samples_that_interpolate_from_it() {
        let (t, mut v) = (axis(20), ramp(20));
        v[7] = f64::NAN;
        let shifted = shift_channel(&t, &v, 1.5e-5, 5).unwrap();
        let nans: Vec<usize> = (0..20).filter(|i| shifted[*i].is_nan()).collect();
        assert_eq!(nans, vec![5, 6]);
    }

    #[test]
    fn series_of_different_lengths_are_refused_and_not_returned_unchanged() {
        let err = shift_channel(&axis(5), &ramp(4), 1e-5, 5).unwrap_err();
        assert_eq!(
            err,
            ShiftRefusal::LengthMismatch {
                timestamps: 5,
                values: 4
            }
        );
        assert!(err.to_string().contains("same length"));
        // nothing at all is nothing
        assert_eq!(shift_channel(&[], &[], 1e-5, 5).unwrap(), Vec::<f64>::new());
    }

    #[test]
    fn one_sample_moved_forward_keeps_its_value_and_moved_backward_is_refused_not_a_panic() {
        assert_eq!(shift_channel(&[0.0], &[3.5], 1e-5, 5).unwrap(), vec![3.5]);
        assert_eq!(
            shift_channel(&[0.0], &[3.5], -1e-5, 5),
            Err(ShiftRefusal::TooFewSamples { samples: 1 })
        );
        // a shift under the floor is no shift, and never reaches the fit
        assert_eq!(shift_channel(&[0.0], &[3.5], -1e-16, 5).unwrap(), vec![3.5]);
        // two samples can be shifted either way
        assert!(shift_channel(&[0.0, 1e-5], &[1.0, 2.0], -1e-5, 5).is_ok());
    }

    #[test]
    fn the_timestamps_are_origin_plus_native_index_times_period() {
        let strided = NativeIndex::Strided { step: 3, len: 4 };
        let t = timestamps(&strided, 0.0, 1e-5);
        assert_eq!(t, vec![0.0, 3.0 * 1e-5, 6.0 * 1e-5, 9.0 * 1e-5]);
        // the origin is added to each, as it is: `origin + (index x period)`
        let listed = NativeIndex::Listed(Shared::new(vec![2, 5, 6]));
        let t = timestamps(&listed, 1.5, 1e-5);
        assert_eq!(
            t,
            vec![1.5 + 2.0 * 1e-5, 1.5 + 5.0 * 1e-5, 1.5 + 6.0 * 1e-5]
        );
    }

    fn callers() -> TimingFallbacks {
        TimingFallbacks {
            sample_period_s: 1e-5,
            absent_offset_s: 0.0,
            unparsed_offset_s: 0.0,
            unknown_unit: UnknownUnitAs::Steps,
            unrecognised_protocol: CardConvention::OppositeToShifter,
        }
    }

    fn timing(commanded: i64, actual: i64) -> TimingResolution {
        let recorded = RecordedTiming::new()
            .with_commanded(RecordedOffset::steps(commanded))
            .with_actual(RecordedOffset::steps(actual))
            .with_protocols(
                Some(build_log_model::GalvoProtocol::Xy2100),
                Some(build_log_model::GalvoProtocol::Xy2100),
            )
            .with_native_period_s(1e-5);
        resolve_timing(&recorded, &TimingOverride::new(), &callers())
    }

    #[test]
    fn a_commanded_position_takes_the_commanded_offset_and_an_actual_one_the_actual_offset() {
        let (t, v) = (axis(40), ramp(40));
        let timing = timing(2, 5);
        let cz = ChannelKind::CommandedPosition { axis: Axis::Z };
        let az = ChannelKind::ActualPosition { axis: Axis::Z };
        let commanded = shift_series(&timing, &cz, &t, &v).unwrap();
        let actual = shift_series(&timing, &az, &t, &v).unwrap();
        assert_eq!(commanded.stage(), Stage::Shifted);
        assert_eq!(commanded.kind(), &cz);
        assert_eq!(
            commanded.values(),
            shift_channel(&t, &v, 2.0 * 1e-5, SHIFT_FIT_N).unwrap()
        );
        assert_eq!(
            actual.values(),
            shift_channel(&t, &v, 5.0 * 1e-5, SHIFT_FIT_N).unwrap()
        );
        assert_ne!(commanded.values(), actual.values());
        // the result says where its offset came from
        assert_eq!(commanded.offset().seconds().origin(), None);
        assert_eq!(commanded.offset().seconds().value(), 2.0 * 1e-5);
        let overridden = resolve_timing(
            &RecordedTiming::new().with_native_period_s(1e-5),
            &TimingOverride::new().with_commanded_offset_steps(4.0),
            &callers(),
        );
        let moved = shift_series(&overridden, &cz, &t, &v).unwrap();
        assert_eq!(moved.offset().seconds().origin(), Some(&Origin::Overridden));
    }

    #[test]
    fn power_state_and_everything_that_is_not_a_position_are_refused() {
        let (t, v) = (axis(10), ramp(10));
        let timing = timing(2, 5);
        for kind in [
            ChannelKind::LaserPower,
            ChannelKind::LaserState,
            ChannelKind::Sensor,
            ChannelKind::Image,
            ChannelKind::Other("x".to_string()),
        ] {
            let err = shift_series(&timing, &kind, &t, &v).unwrap_err();
            assert_eq!(err, ShiftRefusal::NotAPosition { kind: kind.clone() });
            assert!(err.to_string().contains("never"));
        }
    }

    #[test]
    fn fewer_than_two_samples_and_a_length_mismatch_are_refused_by_the_stage() {
        let timing = timing(2, 5);
        let kind = ChannelKind::CommandedPosition { axis: Axis::Z };
        for n in [0, 1] {
            assert_eq!(
                shift_series(&timing, &kind, &axis(n), &ramp(n)).unwrap_err(),
                ShiftRefusal::TooFewSamples { samples: n }
            );
        }
        assert!(matches!(
            shift_series(&timing, &kind, &axis(5), &ramp(4)).unwrap_err(),
            ShiftRefusal::LengthMismatch { .. }
        ));
    }

    #[test]
    fn the_refusals_are_values_that_can_be_sent_and_are_errors() {
        fn assert_traits<T: std::error::Error + Send + Sync + Clone + 'static>() {}
        assert_traits::<ShiftRefusal>();
    }
}
