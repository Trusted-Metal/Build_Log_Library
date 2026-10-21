//! Time: a time exactly as a file gave it, and the time axis with its accuracy and the meaning of "start" (decisions D02-6 and D02-19).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** streams.
//! * **Constructed by:** producers (`TimeValue`, `TimeWindow`, `Instant`, `Segment`, `TimeAnchor`); library (`TimeAxis`, `TimeAxisSummary`);
//!   the enums `Accuracy` and `StartMeaning`.
//! * **Requirement:** BL-CD-10.
//!
//! # What this module claims, and what it does not
//!
//! * **A time is kept as the file gave it** (decision D02-19). A layer time, a laser time, a segment bound is a [`TimeValue`]: the integer as written, a
//!   unit only if the file records one, and where the unit came from. Writing it back writes the same integer. **No file records the unit of a layer
//!   time or a laser time, so in every file we have that unit is unknown**, and nothing here guesses it.
//! * **Nothing is derived.** There is no conversion from a [`TimeValue`] to an [`Instant`], no spacing between samples and no verdict on how much of a
//!   window holds stored samples: the window and the sample period are both there, as written, for anyone who reads the axis, and the comparison waits for
//!   the clock helper that D02-19 leaves for when a consumer needs it. [`Instant`] is the type of that helper's results, such as a parsed build date (the CDM keeps the date as the string the file gave).
//! * **Segments are runs of consecutive stored samples.** The segment table of a gated capture can hold 10^5 rows, so it is a column and not part of the
//!   metadata: the axis keeps a [`TimeAxisSummary`] and the key of the column. [`check_segments`] and [`index_range`] work on the rows once they are read.

use std::ops::Range;

use crate::error::{Error, ErrorKind, Violation};
use crate::identity::ChannelKey;
use crate::units::{Quantity, Unit, UnitSource};

/// A time exactly as a file gave it: the integer, the unit if the file records one, and where that unit came from.
///
/// The unit is [`UnitSource::Unknown`] unless the file says it. A value is never rescaled, and a value above `2^53` (a nanosecond since 1970 is about
/// `1.8 × 10^18`) is kept exactly, which is why this is an integer and not a [`Quantity`].
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TimeValue {
    pub value: i64,
    pub unit: Unit,
    pub unit_source: UnitSource,
}

impl TimeValue {
    /// A time as a file gave it: `unit` is the text of the unit attribute, or `None` if the file has none. A unit in the vocabulary is
    /// [`UnitSource::FromFile`]; anything else (none, empty, or a spelling nobody has listed) is [`UnitSource::Unknown`], with the spelling kept.
    /// **The value is never changed.**
    pub fn from_file(value: i64, unit: Option<&str>) -> Self {
        let Quantity {
            unit, unit_source, ..
        } = Quantity::from_file(0.0, unit);
        TimeValue {
            value,
            unit,
            unit_source,
        }
    }

    /// The same for a file that stores the time as an unsigned 64-bit integer. A value above `i64::MAX` does not fit and is an `OutOfWidth` error, never
    /// a wrapped or clamped number.
    pub fn from_file_unsigned(value: u64, unit: Option<&str>) -> Result<Self, Error> {
        match i64::try_from(value) {
            Ok(value) => Ok(TimeValue::from_file(value, unit)),
            Err(_) => Err(Error::new(ErrorKind::OutOfWidth {
                value: value.to_string(),
                stored_as: "a signed 64-bit integer".to_string(),
            })),
        }
    }

    /// Can the two be compared as numbers? True for the same known unit, and for two unknown units, which are taken to be the one clock of one file.
    /// False when one unit is known and the other is not, or when both are known and differ: a comparison would mean something that nobody checked.
    pub fn is_comparable_to(&self, other: &TimeValue) -> bool {
        self.unit.is_same_unit(&other.unit) || (!self.unit.is_known() && !other.unit.is_known())
    }
}

/// The span of time a stream was being recorded, as written. It is longer than the time the stored samples cover when the capture was gated.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TimeWindow {
    pub start: TimeValue,
    pub end: TimeValue,
}

/// A moment on the clock of the capture, in nanoseconds since the Unix epoch (decision G12): the type of the clock helper's results, such as a parsed build date,
/// that comes later. Nothing converts a layer or laser time to it (decision D02-19).
// constructed-by: producer
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(pub i64);

/// A run of consecutive stored samples: the indices `first_sample` to `first_sample + samples - 1`, captured without a gap, and the times of its first and
/// last sample (inclusive), as written.
///
/// Inside a segment the samples are taken to be evenly spaced between `start` and `end`; that is what a segment is. Between segments there is a gap.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub first_sample: u64,
    pub samples: u64,
    pub start: TimeValue,
    pub end: TimeValue,
}

/// A stored sample tied to a time, written once per batch by a capture that has such a clock. Optional: no file we have carries one.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeAnchor {
    pub sample: u64,
    pub at: TimeValue,
}

/// How well the times of a stream are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Accuracy {
    /// The file records the times of the segments, or of each sample, from the capture clock.
    Measured,
    /// The file declares a sample period, and the times follow from it: nothing was measured. (A spacing worked out from a window and a count is not this:
    /// that is an inference, and waits for the clock helper.)
    Nominal,
    /// The file records neither: only a window and a count, or nothing. The axis claims no spacing.
    #[default]
    Unknown,
}

/// What the start of a stream's window means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StartMeaning {
    /// The capture was recording from this time; samples may begin later (a gated capture stores nothing until its trigger).
    RecordingBegan,
    /// This is the time of the first stored sample.
    FirstStoredSample,
    /// The file does not say.
    #[default]
    Unknown,
}

/// A small description of the segment table: enough to know its extent without reading it.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct TimeAxisSummary {
    /// How many segments there are. Zero when the file has no segment table.
    pub segment_count: u64,
    /// The start of the first segment, if there is one.
    pub first_start: Option<TimeValue>,
    /// The end of the last segment, if there is one.
    pub last_end: Option<TimeValue>,
    /// How many samples the segments cover in all.
    pub covered_samples: u64,
}

impl TimeAxisSummary {
    /// The summary of a segment table that has been read. The segments are taken in the order given, which is the order of their sample indices.
    pub fn of(segments: &[Segment]) -> Self {
        TimeAxisSummary {
            segment_count: segments.len() as u64,
            first_start: segments.first().map(|s| s.start.clone()),
            last_end: segments.last().map(|s| s.end.clone()),
            covered_samples: segments.iter().map(|s| s.samples).sum(),
        }
    }
}

/// The times of one stream (a laser in a layer), as the file gives them: what is known, in the file's own numbers, and nothing derived from them.
///
/// Every part is optional or has a "not known" value, so an axis can say exactly as much as the file does: V18 has a window and a count and no period
/// (accuracy `Unknown`); V16 has a period with a unit (`Nominal`); a gated V17 capture has a window, a count and a period whose unit is not written.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TimeAxis {
    pub accuracy: Accuracy,
    pub start_meaning: StartMeaning,
    /// The span of time the stream was recorded, as written.
    pub window: Option<TimeWindow>,
    /// How many samples the stream stores.
    pub stored_samples: Option<u64>,
    /// The sample period where the file gives one, as written (`'10us'` has its unit; `10.0` does not).
    pub period: Option<Quantity>,
    pub summary: TimeAxisSummary,
    /// The key of the column that holds the segment table, if the file has one. The table itself is bulk data, read through a column accessor.
    pub segment_table: Option<ChannelKey>,
}

impl TimeAxis {
    pub fn new() -> Self {
        TimeAxis::default()
    }

    pub fn with_accuracy(mut self, accuracy: Accuracy) -> Self {
        self.accuracy = accuracy;
        self
    }

    pub fn with_start_meaning(mut self, start_meaning: StartMeaning) -> Self {
        self.start_meaning = start_meaning;
        self
    }

    pub fn with_window(mut self, window: TimeWindow) -> Self {
        self.window = Some(window);
        self
    }

    pub fn with_stored_samples(mut self, stored_samples: u64) -> Self {
        self.stored_samples = Some(stored_samples);
        self
    }

    pub fn with_period(mut self, period: Quantity) -> Self {
        self.period = Some(period);
        self
    }

    pub fn with_summary(mut self, summary: TimeAxisSummary) -> Self {
        self.summary = summary;
        self
    }

    pub fn with_segment_table(mut self, key: ChannelKey) -> Self {
        self.segment_table = Some(key);
        self
    }
}

/// Checks a segment table that has been read, and reports **every** violation: a segment without samples, one that ends before it starts or mixes units,
/// two that overlap in sample indices or in time, and two whose times cannot be compared.
pub fn check_segments(segments: &[Segment]) -> Result<(), Error> {
    let mut violations = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        if segment.samples == 0 {
            violations.push(Violation::new(format!("segment {i} holds no samples")));
        }
        if segment.first_sample.checked_add(segment.samples).is_none() {
            violations.push(Violation::new(format!(
                "segment {i} runs past the largest sample index"
            )));
        }
        if !segment.start.is_comparable_to(&segment.end) {
            violations.push(Violation::new(format!(
                "segment {i} starts and ends in different units"
            )));
        } else if segment.end.value < segment.start.value {
            violations.push(Violation::new(format!("segment {i} ends before it starts")));
        }
    }
    for (i, pair) in segments.windows(2).enumerate() {
        let (a, b) = (&pair[0], &pair[1]);
        if b.first_sample < a.first_sample.saturating_add(a.samples) {
            violations.push(Violation::new(format!(
                "segments {i} and {} overlap in sample indices (or are out of order)",
                i + 1
            )));
        }
        if !a.end.is_comparable_to(&b.start) {
            violations.push(Violation::new(format!(
                "segments {i} and {} are in different units",
                i + 1
            )));
        } else if b.start.value < a.end.value {
            violations.push(Violation::new(format!(
                "segments {i} and {} overlap in time",
                i + 1
            )));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::ValidationError { violations }))
    }
}

/// The sample indices that lie inside the time range `from` to `to` (both inclusive, in the clock of the segments), given a segment table that has been
/// read and passes [`check_segments`]: from the first sample at or after `from` to the last sample at or before `to`. A time that falls in a gap
/// between segments has no sample, so a range that lies wholly in a gap, before the first segment or after the last gives `None`.
///
/// Inside a segment the samples are evenly spaced between its bounds (see [`Segment`]). The range is in the clock of the file: if the request is in a unit
/// that cannot be compared with the segments' (see [`TimeValue::is_comparable_to`]), or ends before it starts, that is an `InvalidIndex` error.
pub fn index_range(
    segments: &[Segment],
    from: &TimeValue,
    to: &TimeValue,
) -> Result<Option<Range<u64>>, Error> {
    let invalid = |what: &str| {
        Error::new(ErrorKind::InvalidIndex {
            what: what.to_string(),
        })
    };
    if !from.is_comparable_to(to) || to.value < from.value {
        return Err(invalid(
            "a time range that ends before it starts, or mixes units",
        ));
    }
    let overflow = || {
        Error::new(ErrorKind::OutOfWidth {
            value: "a position inside a segment".to_string(),
            stored_as: "a 128-bit integer".to_string(),
        })
    };
    let (mut first, mut last) = (None, None);
    for segment in segments {
        if !from.is_comparable_to(&segment.start) {
            return Err(invalid(
                "a time range in a unit that the segments are not in",
            ));
        }
        let (s, e) = (
            i128::from(segment.start.value),
            i128::from(segment.end.value),
        );
        let (lo, hi) = (i128::from(from.value).max(s), i128::from(to.value).min(e));
        if lo > hi || segment.samples == 0 {
            continue;
        }
        let steps = i128::from(segment.samples) - 1;
        let span = e - s;
        let (a, b) = if span == 0 || steps == 0 {
            (0, steps)
        } else {
            let up = (lo - s).checked_mul(steps).ok_or_else(overflow)?;
            let down = (hi - s).checked_mul(steps).ok_or_else(overflow)?;
            ((up + span - 1) / span, down / span)
        };
        if a > b {
            continue;
        }
        let at = |step: i128| segment.first_sample.saturating_add(step as u64);
        first.get_or_insert(at(a));
        last = Some(at(b));
    }
    Ok(first.zip(last).map(|(first, last)| first..last + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorClass;
    use crate::identity::Presence;
    use crate::units::{parse_embedded, Dimension};

    fn at(value: i64) -> TimeValue {
        TimeValue::from_file(value, None)
    }

    fn window(start: i64, end: i64) -> TimeWindow {
        TimeWindow {
            start: at(start),
            end: at(end),
        }
    }

    fn segment(first_sample: u64, samples: u64, start: i64, end: i64) -> Segment {
        Segment {
            first_sample,
            samples,
            start: at(start),
            end: at(end),
        }
    }

    fn violations(error: &Error) -> &[Violation] {
        match error.kind() {
            ErrorKind::ValidationError { violations } => violations,
            other => panic!("expected a validation error, found {other:?}"),
        }
    }

    #[test]
    fn time_values_are_kept_exactly_as_written() {
        // above 2^53, where an f64 would round: a nanosecond since 1970
        let big = 1_792_165_086_845_123_457_i64;
        assert!(big > (1_i64 << 53));
        assert_eq!(TimeValue::from_file(big, None).value, big);
        // no unit in the file: unknown, and not guessed
        let none = TimeValue::from_file(1_792_165_086_845, None);
        assert_eq!(none.unit_source, UnitSource::Unknown);
        assert!(!none.unit.is_known() && none.unit.spelling.is_none());
        // a unit the file writes is taken as written
        let us = TimeValue::from_file(10, Some("us"));
        assert_eq!(us.unit_source, UnitSource::FromFile);
        assert_eq!(us.unit.dimension(), Some(Dimension::Time));
        // a spelling nobody has listed stays unknown, and is kept
        let odd = TimeValue::from_file(10, Some("ticks"));
        assert_eq!(odd.unit_source, UnitSource::Unknown);
        assert_eq!(odd.unit.spelling.as_deref(), Some("ticks"));
        // an unsigned value that does not fit is an error, never a wrapped number
        assert_eq!(
            TimeValue::from_file_unsigned(i64::MAX as u64, None)
                .unwrap()
                .value,
            i64::MAX
        );
        let error = TimeValue::from_file_unsigned(u64::MAX, None).unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::OutOfWidth { .. }));
        assert_eq!(error.class(), ErrorClass::Validation);
        // two unknown units are one clock, an unknown and a known unit are not
        assert!(none.is_comparable_to(&at(5)));
        assert!(!none.is_comparable_to(&us));
        assert!(us.is_comparable_to(&TimeValue::from_file(7, Some("usec"))));
        assert!(!us.is_comparable_to(&TimeValue::from_file(7, Some("ms"))));
    }

    #[test]
    fn an_instant_holds_a_nanosecond_epoch_time_without_loss() {
        let a = Instant(1_792_165_086_845_000_001);
        assert!(a > Instant(1_792_165_086_845_000_000));
        assert_eq!(a.0, 1_792_165_086_845_000_001);
    }

    #[test]
    fn v18_axis_holds_start_end_and_count_as_written_and_claims_no_spacing() {
        // 727,922 samples in a window of 32.96 s: a gated capture, so the window is not the time the samples cover
        let axis = TimeAxis::new()
            .with_window(window(1_787_071_705_000_000, 1_787_071_737_960_000))
            .with_stored_samples(727_922);
        assert_eq!(axis.accuracy, Accuracy::Unknown);
        assert_eq!(axis.start_meaning, StartMeaning::Unknown);
        assert_eq!(axis.stored_samples, Some(727_922));
        let w = axis.window.as_ref().unwrap();
        assert_eq!(
            (w.start.value, w.end.value),
            (1_787_071_705_000_000, 1_787_071_737_960_000)
        );
        // no period, no segment table, nothing derived from the window and the count
        assert!(axis.period.is_none() && axis.segment_table.is_none());
        assert_eq!(axis.summary, TimeAxisSummary::default());
    }

    #[test]
    fn v16_axis_is_nominal_from_its_sample_period() {
        // V16 writes the period as a text with its unit: '10us'
        let period = parse_embedded("10us").unwrap();
        assert_eq!(period.unit_source, UnitSource::FromFile);
        let axis = TimeAxis::new()
            .with_accuracy(Accuracy::Nominal)
            .with_window(window(1_000_000, 25_381_000))
            .with_stored_samples(2_048_219)
            .with_period(period.clone());
        assert_eq!(axis.accuracy, Accuracy::Nominal);
        assert_eq!(axis.period, Some(period));
        assert_eq!(axis.period.as_ref().unwrap().value, 10.0);
    }

    #[test]
    fn v17_mock_axis_keeps_window_count_and_period_as_written() {
        // the mock capture: 31,000 samples in a window of 901,600 (microseconds, though the file does not say so), a period of 10.0 with no unit
        let period = Quantity::from_file(10.0, None);
        let axis = TimeAxis::new()
            .with_accuracy(Accuracy::Nominal)
            .with_start_meaning(StartMeaning::RecordingBegan)
            .with_window(window(1_792_165_086_845_000, 1_792_165_087_746_600))
            .with_stored_samples(31_000)
            .with_period(period);
        let w = axis.window.as_ref().unwrap();
        assert_eq!(w.end.value - w.start.value, 901_600);
        assert_eq!(axis.stored_samples, Some(31_000));
        // both numbers are there, neither has a unit, and the axis offers no comparison of them
        assert_eq!(axis.period.as_ref().unwrap().value, 10.0);
        assert!(!axis.period.as_ref().unwrap().unit.is_known());
        assert!(!w.start.unit.is_known());
        assert_eq!(axis.start_meaning, StartMeaning::RecordingBegan);
    }

    #[test]
    fn the_2026_10_15_axis_keeps_window_and_count_as_written() {
        // one laser with 591,704 samples in a window of 41,837,000; the other laser is in the file and holds nothing
        let first = TimeAxis::new()
            .with_window(window(1_792_000_000_000_000, 1_792_000_041_837_000))
            .with_stored_samples(591_704);
        let w = first.window.as_ref().unwrap();
        assert_eq!(w.end.value - w.start.value, 41_837_000);
        assert_eq!(
            Presence::from_count(first.stored_samples.unwrap()).samples(),
            Some(591_704)
        );
        let second = TimeAxis::new().with_stored_samples(0);
        assert!(second.window.is_none());
        assert_eq!(
            Presence::from_count(second.stored_samples.unwrap()),
            Presence::RecordedEmpty
        );
    }

    #[test]
    fn overlapping_segments_are_rejected() {
        // a good table
        let good = [segment(0, 101, 1000, 2000), segment(101, 51, 5000, 5500)];
        assert!(check_segments(&good).is_ok());
        assert!(check_segments(&[]).is_ok());
        // overlapping sample indices
        let by_index = [segment(0, 101, 1000, 2000), segment(100, 51, 5000, 5500)];
        let error = check_segments(&by_index).unwrap_err();
        assert!(violations(&error)[0]
            .message
            .contains("overlap in sample indices"));
        // overlapping times
        let by_time = [segment(0, 101, 1000, 2000), segment(101, 51, 1500, 5500)];
        let error = check_segments(&by_time).unwrap_err();
        assert!(violations(&error)[0].message.contains("overlap in time"));
        // every violation is listed, not the first: no samples, ends before it starts, out of order, mixed units
        let bad = [
            segment(0, 0, 10, 20),
            Segment {
                first_sample: 0,
                samples: 5,
                start: at(300),
                end: at(200),
            },
            Segment {
                first_sample: 5,
                samples: 5,
                start: TimeValue::from_file(400, Some("us")),
                end: TimeValue::from_file(500, Some("us")),
            },
        ];
        let error = check_segments(&bad).unwrap_err();
        let text: Vec<&str> = violations(&error)
            .iter()
            .map(|v| v.message.as_str())
            .collect();
        assert!(
            text.iter()
                .any(|m| m.contains("segment 0 holds no samples")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|m| m.contains("segment 1 ends before it starts")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|m| m.contains("segments 1 and 2 are in different units")),
            "{text:?}"
        );
        assert!(text.len() >= 3);
    }

    #[test]
    fn a_gated_capture_with_many_segments_has_a_summary_and_a_column_reference() {
        // 10,000 segments of 100 samples with a gap of 50 time units between them
        let segments: Vec<Segment> = (0..10_000_i64)
            .map(|i| segment(i as u64 * 100, 100, i * 1_000, i * 1_000 + 990))
            .collect();
        assert!(check_segments(&segments).is_ok());
        let summary = TimeAxisSummary::of(&segments);
        assert_eq!(summary.segment_count, 10_000);
        assert_eq!(summary.covered_samples, 1_000_000);
        assert_eq!(summary.first_start, Some(at(0)));
        assert_eq!(summary.last_end, Some(at(9_999_990)));
        let axis = TimeAxis::new()
            .with_summary(summary.clone())
            .with_segment_table(ChannelKey::new(["segments"]));
        assert_eq!(axis.summary, summary);
        assert_eq!(axis.segment_table, Some(ChannelKey::new(["segments"])));
        assert_eq!(TimeAxisSummary::of(&[]), TimeAxisSummary::default());
    }

    #[test]
    fn index_range_for_a_time_range_skips_gaps() {
        // samples 0..=100 every 10 from 1000 to 2000, then a gap, then samples 101..=151 every 10 from 5000 to 5500
        let table = [segment(0, 101, 1000, 2000), segment(101, 51, 5000, 5500)];
        let range = |from: i64, to: i64| index_range(&table, &at(from), &at(to)).unwrap();
        // across the gap: from sample 50 (time 1500) to sample 121 (time 5200)
        assert_eq!(range(1500, 5200), Some(50..122));
        // the last sample of the first segment and the first of the second, the gap between them skipped
        assert_eq!(range(2000, 5000), Some(100..102));
        // wholly inside the gap, before the first segment, after the last: no samples
        assert_eq!(range(2500, 4500), None);
        assert_eq!(range(0, 999), None);
        assert_eq!(range(6000, 7000), None);
        // between two samples: no sample lies inside
        assert_eq!(range(1005, 1009), None);
        // exactly one sample, and the whole table
        assert_eq!(range(1010, 1010), Some(1..2));
        assert_eq!(range(i64::MIN, i64::MAX), Some(0..152));
        // a request that ends before it starts, or is in another unit, is an error and not an empty answer
        let backwards = index_range(&table, &at(5), &at(1)).unwrap_err();
        assert!(matches!(backwards.kind(), ErrorKind::InvalidIndex { .. }));
        let other_unit = index_range(
            &table,
            &TimeValue::from_file(1000, Some("us")),
            &TimeValue::from_file(2000, Some("us")),
        )
        .unwrap_err();
        assert!(matches!(other_unit.kind(), ErrorKind::InvalidIndex { .. }));
        // a one-sample segment, and a segment of one instant, are whole
        let single = [segment(7, 1, 100, 100)];
        assert_eq!(
            index_range(&single, &at(100), &at(100)).unwrap(),
            Some(7..8)
        );
        assert_eq!(index_range(&single, &at(101), &at(200)).unwrap(), None);
    }
}
