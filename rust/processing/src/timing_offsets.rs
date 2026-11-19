//! The timing offsets of a laser's scanner card, as the file and the user give them, in seconds and in the shifter's convention (IP-27 phase 27F).
//!
//! * **Layer:** Processing. It works on Model values only (the parameter snapshot, the user's overrides), so it runs without HDF5.
//! * **Requirement:** BL-PR-03 (every value says where it came from), BL-PR-04 (the sign convention is fixed once, documented and tested).
//!
//! # What the format means: the library's (decision D27-21)
//!
//! * **A bare number is a number of native sample-period steps** (`50` with a native period of 10 microseconds is 500 microseconds). The Model types the offsets as whole numbers
//!   ([`build_log_model::ScannerCardParameters`]), and every V17 file records them that way.
//! * **Text** (a form an older or foreign writer used, and the form of Vision's own loader) is read by [`parse_offset_text`]: one pair of quotes is tolerated, a bare number is steps, and a
//!   number with a unit is a time (`s`, `sec`, `ms`, `us`, `µs` with the micro sign U+00B5, `ns`, in any case; the sample period is then ignored). The Greek letter mu (U+03BC) is **not**
//!   a micro sign, as in Vision: such a unit is an unknown unit.
//! * **The sign depends on the galvo protocol** ([`card_convention`]): an SL2 card stores the offset **opposite** to the shifter's convention (`t' = t + dt`, a positive offset shifts a
//!   series forward), so it is negated; an XY2 card stores it in the shifter's own convention. The protocols are matched on their text, in any case, anywhere in it, and an XY2 in either
//!   galvo decides.
//! * **The user's override is in native steps, replaces the file's value, and is not signed** (the user types in the shifter's convention).
//!
//! # What the file does not say: the caller's (decisions D27-18 and D27-21)
//!
//! The library applies **no default**. It **reports** each case where the file is silent and takes the caller's answer from [`TimingFallbacks`], an explicit, immutable and hashable object
//! that has a value for every case, so that nothing is decided out of sight. A value that comes from the fallbacks has the origin [`Origin::CallerSupplied`]:
//!
//! | The file | Reported as | Taken from |
//! |---|---|---|
//! | no offset | [`OffsetReading::Absent`] (with the reader's reason, when it recorded one) | `absent_offset_s` |
//! | text that is empty, is not a number, or has a unit and no number | [`OffsetReading::Unparsed`] | `unparsed_offset_s` |
//! | a number with a unit that is not known | [`OffsetReading::UnknownUnit`] | `unknown_unit` (read as steps or as seconds) |
//! | a protocol that is neither XY2 nor SL2, or none | `None` from [`card_convention`] | `unrecognised_protocol` |
//! | no sample period, or one that is not above 0 | the period's origin is `CallerSupplied` | `sample_period_s` |
//!
//! **The unit of the sample period is declared by the caller** (settled at phase 27F): [`RecordedTiming::with_native_period_s`] takes the **native** sample period **in seconds**. A V17
//! file states no unit for it (`10.0`, which the Model keeps as a number with an unknown unit), and which unit a unit-less number is in is a reading of the version that Vision makes today
//! (a value above 0.001 is microseconds). The library does not guess it. A version's adapter that declared the unit (`UnitSource::DeclaredByAdapter`) would make this input unnecessary: that
//! is a change to the read half, and not taken here.
//!
//! **What is not here: the mirroring of a commanded override onto the actual offset.** Vision applies it (a commanded override alone also becomes the actual offset); it is Vision's policy
//! and its adapter makes it before it calls [`resolve_timing`], by giving both overrides. Here each offset takes only its own override.

use build_log_model::{GalvoProtocol, LaserParameters, Origin, TimingOverride};

use crate::calibration::Resolved;

// ---------------------------------------------------------------- the format: text

/// What a text offset says, before anything is decided about it.
// open-domain: the forms a writer can use for an offset; a consumer matches on the ones it knows
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ParsedOffset {
    /// A bare number: a number of native sample-period steps (not yet multiplied by the period).
    Steps(f64),
    /// A number with a time unit the library knows, as seconds.
    Time { seconds: f64 },
    /// A number with a unit that is not known (the unit as written, trimmed). What it means is the caller's: see [`UnknownUnitAs`].
    UnknownUnit { value: f64, unit: String },
    /// Nothing that is a number: empty or blank text, text that does not start with a number, or a unit with no number.
    Unparsed,
}

/// What a text offset says. **This is Vision's reading of the text, as it is, in the same operations** (the vectors pin it): the text is trimmed, one pair of double or single quotes is
/// removed and what is inside is trimmed, the number ends at the first character that is not a digit, `.`, `-`, `+`, `e` or `E`, and the rest, trimmed and lowercased, is the unit.
pub fn parse_offset_text(text: &str) -> ParsedOffset {
    let mut s = text.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s = s[1..s.len() - 1].trim();
    }
    if s.is_empty() {
        return ParsedOffset::Unparsed;
    }
    let number_end = s
        .find(|c: char| {
            !c.is_ascii_digit() && c != '.' && c != '-' && c != '+' && c != 'e' && c != 'E'
        })
        .unwrap_or(s.len());
    let number = s[..number_end].trim();
    let unit = s[number_end..].trim();
    let Ok(value) = number.parse::<f64>() else {
        return ParsedOffset::Unparsed;
    };
    match unit.to_lowercase().as_str() {
        "" => ParsedOffset::Steps(value),
        "s" | "sec" => ParsedOffset::Time { seconds: value },
        "ms" => ParsedOffset::Time {
            seconds: value * 1e-3,
        },
        "us" | "\u{b5}s" => ParsedOffset::Time {
            seconds: value * 1e-6,
        },
        "ns" => ParsedOffset::Time {
            seconds: value * 1e-9,
        },
        _ => ParsedOffset::UnknownUnit {
            value,
            unit: unit.to_string(),
        },
    }
}

// ---------------------------------------------------------------- the format: the sign

/// How the card stores its offsets, relative to the shifter's convention (`t' = t + dt`: a positive offset shifts a series forward).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CardConvention {
    /// In the shifter's own convention (an XY2 card, such as an intelliSCAN): the offset is used as it is.
    AsShifter,
    /// Opposite to it (an SL2 card, such as Aconity's SL2-100): the offset is negated.
    OppositeToShifter,
}

impl CardConvention {
    /// The factor that takes the card's offset to the shifter's convention: `1.0` or `-1.0`.
    pub fn sign(self) -> f64 {
        match self {
            CardConvention::AsShifter => 1.0,
            CardConvention::OppositeToShifter => -1.0,
        }
    }
}

/// A protocol as the text it is matched on: the library's names are the file's spelling, and a protocol the library has no name for is the text the file holds.
fn protocol_text(protocol: &GalvoProtocol) -> Option<&str> {
    match protocol {
        GalvoProtocol::Xy2100 => Some("XY2-100"),
        GalvoProtocol::Sl2100 => Some("SL2-100"),
        GalvoProtocol::Other(text) => Some(text),
        _ => None,
    }
}

/// How the card stores its offsets, from the protocols of the X and the Y galvo: **an `XY2` in either one** (in any case, anywhere in the text) **is the shifter's own convention**; otherwise
/// **an `SL2` in either one is the opposite**; otherwise `None`: the protocol is absent or not one the library knows, and the caller decides ([`TimingFallbacks::unrecognised_protocol`]).
pub fn card_convention(
    galvo_x: Option<&GalvoProtocol>,
    galvo_y: Option<&GalvoProtocol>,
) -> Option<CardConvention> {
    let names: Vec<String> = [galvo_x, galvo_y]
        .into_iter()
        .flatten()
        .filter_map(protocol_text)
        .map(str::to_uppercase)
        .collect();
    if names.iter().any(|name| name.contains("XY2")) {
        Some(CardConvention::AsShifter)
    } else if names.iter().any(|name| name.contains("SL2")) {
        Some(CardConvention::OppositeToShifter)
    } else {
        None
    }
}

// ---------------------------------------------------------------- the caller's answers

/// What a number with a unit that is not known means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnknownUnitAs {
    /// A number of native sample-period steps (Vision's choice: reading it as seconds would be out by about 100,000).
    Steps,
    /// A number of seconds.
    Seconds,
}

/// **What the caller decides where the file is silent.** Every field is required, so that every decision is visible at the call site; the library has no default for any of them. A value
/// taken from here has the origin [`Origin::CallerSupplied`]. The object is immutable, compares and hashes by the bits of its numbers, so it can be part of a cache key.
// constructed-by: caller-options
#[derive(Debug, Clone, Copy)]
pub struct TimingFallbacks {
    /// The native sample period, in seconds, when the caller gives none or one that is not above 0.
    pub sample_period_s: f64,
    /// The offset, in seconds and in the shifter's convention (not signed), when the file has none.
    pub absent_offset_s: f64,
    /// The offset, in seconds and in the shifter's convention (not signed), when the file's text cannot be read as a number.
    pub unparsed_offset_s: f64,
    /// What a number with an unknown unit means.
    pub unknown_unit: UnknownUnitAs,
    /// How the card stores its offsets when the protocol is absent or not one the library knows.
    pub unrecognised_protocol: CardConvention,
}

impl TimingFallbacks {
    fn key(&self) -> (u64, u64, u64, UnknownUnitAs, CardConvention) {
        (
            self.sample_period_s.to_bits(),
            self.absent_offset_s.to_bits(),
            self.unparsed_offset_s.to_bits(),
            self.unknown_unit,
            self.unrecognised_protocol,
        )
    }
}

impl PartialEq for TimingFallbacks {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for TimingFallbacks {}

impl std::hash::Hash for TimingFallbacks {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

// ---------------------------------------------------------------- what the file recorded

/// One offset as a file recorded it.
#[derive(Debug, Clone, PartialEq)]
enum OffsetValue {
    Steps(i64),
    Text(String),
}

/// A timing offset as the file recorded it: a whole number of native steps (what the Model types), or text, or nothing.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedOffset {
    value: Option<OffsetValue>,
    origin: Option<Origin>,
}

impl RecordedOffset {
    /// A whole number of native sample-period steps.
    pub fn steps(steps: i64) -> Self {
        RecordedOffset {
            value: Some(OffsetValue::Steps(steps)),
            origin: None,
        }
    }

    /// Text, in any of the forms [`parse_offset_text`] reads.
    pub fn text(text: impl Into<String>) -> Self {
        RecordedOffset {
            value: Some(OffsetValue::Text(text.into())),
            origin: None,
        }
    }

    /// The file has no offset.
    pub fn absent() -> Self {
        RecordedOffset {
            value: None,
            origin: None,
        }
    }

    /// The origin that the reader recorded for the value, or for its absence (a value that was not read says why).
    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.origin = Some(origin);
        self
    }
}

/// What the file records for the time shift of one laser, as the library reads it: the two offsets, the protocols of the two galvos, and the native sample period **in seconds**, which the
/// caller declares (see the module documentation).
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedTiming {
    commanded: RecordedOffset,
    actual: RecordedOffset,
    protocol_x: Option<GalvoProtocol>,
    protocol_y: Option<GalvoProtocol>,
    native_period_s: Option<f64>,
    native_period_origin: Option<Origin>,
}

impl Default for RecordedTiming {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordedTiming {
    /// A laser the file says nothing about: no offsets, no protocols, no sample period.
    pub fn new() -> Self {
        RecordedTiming {
            commanded: RecordedOffset::absent(),
            actual: RecordedOffset::absent(),
            protocol_x: None,
            protocol_y: None,
            native_period_s: None,
            native_period_origin: None,
        }
    }

    /// What the parameter snapshot of a laser holds (`None`: a laser with no snapshot): the offsets as whole numbers of native steps, with the origins that the reader recorded for them,
    /// and the protocols of the two galvos. **The native period is not taken from the snapshot:** its unit is the caller's (see the module documentation).
    pub fn from_parameters(parameters: Option<&LaserParameters>) -> Self {
        let Some(p) = parameters else {
            return Self::new();
        };
        let offset = |value: Option<i64>, name: &str| {
            let recorded = value.map_or_else(RecordedOffset::absent, RecordedOffset::steps);
            match p.origins.get(name) {
                Some(origin) => recorded.with_origin(origin.clone()),
                None => recorded,
            }
        };
        RecordedTiming {
            commanded: offset(
                p.scanner_card.commanded_position_timing_offset,
                "scanner_card.commanded_position_timing_offset",
            ),
            actual: offset(
                p.scanner_card.actual_position_timing_offset,
                "scanner_card.actual_position_timing_offset",
            ),
            protocol_x: p.galvo_x.protocol.clone(),
            protocol_y: p.galvo_y.protocol.clone(),
            native_period_s: None,
            native_period_origin: None,
        }
    }

    pub fn with_commanded(mut self, offset: RecordedOffset) -> Self {
        self.commanded = offset;
        self
    }

    pub fn with_actual(mut self, offset: RecordedOffset) -> Self {
        self.actual = offset;
        self
    }

    pub fn with_protocols(
        mut self,
        galvo_x: Option<GalvoProtocol>,
        galvo_y: Option<GalvoProtocol>,
    ) -> Self {
        self.protocol_x = galvo_x;
        self.protocol_y = galvo_y;
        self
    }

    /// The **native** sample period, in seconds: the period of the stored samples, whatever stride the layer was read with. Whatever is not above 0 is reported as unusable.
    pub fn with_native_period_s(mut self, seconds: f64) -> Self {
        self.native_period_s = Some(seconds);
        self
    }

    /// Where the native period came from, when it is not simply the file's: the caller's reading of a period whose unit the file does not state ([`Origin::CallerSupplied`]).
    pub fn with_native_period_origin(mut self, origin: Origin) -> Self {
        self.native_period_origin = Some(origin);
        self
    }
}

// ---------------------------------------------------------------- what is resolved

/// How the file's own offset was read.
// open-domain: the ways an offset can be present or missing; a consumer matches on the ones it knows
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum OffsetReading {
    /// A number of native steps (a whole number, or a bare number in text).
    Steps,
    /// A time with a unit that the library knows.
    Time,
    /// A number with a unit that is not known (the unit as written): read as the caller's [`TimingFallbacks::unknown_unit`] says.
    UnknownUnit { unit: String },
    /// Text that is not a number: the caller's [`TimingFallbacks::unparsed_offset_s`] is used.
    Unparsed { text: String },
    /// The file has no offset: the caller's [`TimingFallbacks::absent_offset_s`] is used. `why` is what the reader recorded for it, if anything (such as that it was not read).
    Absent { why: Option<Origin> },
}

/// One offset, in seconds and in the shifter's convention, with where it came from.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedOffset {
    seconds: Resolved<f64>,
    file: OffsetReading,
}

impl ResolvedOffset {
    /// The offset in seconds: positive shifts a series forward (`t' = t + offset`). Its origin says whether it is the file's (`None`), the user's ([`Origin::Overridden`]) or the caller's
    /// ([`Origin::CallerSupplied`]).
    pub fn seconds(&self) -> &Resolved<f64> {
        &self.seconds
    }

    /// How the file's own offset was read, **whether or not the user overrode it**.
    pub fn file(&self) -> &OffsetReading {
        &self.file
    }
}

/// The timing of one laser, resolved: the native period, the sign of the card, and the two offsets.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct TimingResolution {
    native_period: Resolved<f64>,
    convention: Option<CardConvention>,
    sign: Resolved<f64>,
    commanded: ResolvedOffset,
    actual: ResolvedOffset,
}

impl TimingResolution {
    /// The native sample period, in seconds (`CallerSupplied` when the caller gave none that is above 0).
    pub fn native_period(&self) -> &Resolved<f64> {
        &self.native_period
    }

    /// How the card stores its offsets, as the protocols say; `None` when they say nothing the library knows (the sign is then the caller's).
    pub fn convention(&self) -> Option<CardConvention> {
        self.convention
    }

    /// The sign applied to the file's offsets: `1.0` or `-1.0`. Its origin is `None` when the protocols decided it and `CallerSupplied` when the caller did.
    pub fn sign(&self) -> &Resolved<f64> {
        &self.sign
    }

    /// The offset of the commanded positions.
    pub fn commanded(&self) -> &ResolvedOffset {
        &self.commanded
    }

    /// The offset of the actual positions.
    pub fn actual(&self) -> &ResolvedOffset {
        &self.actual
    }
}

fn override_over(
    file: Resolved<f64>,
    override_steps: Option<f64>,
    native_period_s: f64,
) -> Resolved<f64> {
    match override_steps {
        Some(steps) => Resolved::new(steps * native_period_s, Some(Origin::Overridden)),
        None => file,
    }
}

/// The user's override over an offset that is already in seconds and in the shifter's convention (`file_seconds`, taken as the file's own value): **`steps x the native period`, replacing
/// the file's value, with no sign**. A value that is not overridden stays as it is. An override of `0` is an override, and a `NaN` is used as a `NaN` (the same operations as Vision's).
/// [`resolve_timing`] makes this step for each offset; it is here for a caller that has the offset in seconds already.
pub fn apply_override(
    file_seconds: f64,
    override_steps: Option<f64>,
    native_period_s: f64,
) -> Resolved<f64> {
    override_over(
        Resolved::new(file_seconds, None),
        override_steps,
        native_period_s,
    )
}

fn caller_supplied(seconds: f64) -> Resolved<f64> {
    Resolved::new(seconds, Some(Origin::CallerSupplied))
}

/// The file's own offset in seconds and in the shifter's convention, and how it was read.
fn read_offset(
    recorded: &RecordedOffset,
    period_s: f64,
    sign: f64,
    fallbacks: &TimingFallbacks,
) -> (Resolved<f64>, OffsetReading) {
    let from_file = |seconds: f64| Resolved::new(seconds, recorded.origin.clone());
    match &recorded.value {
        None => (
            caller_supplied(fallbacks.absent_offset_s),
            OffsetReading::Absent {
                why: recorded.origin.clone(),
            },
        ),
        Some(OffsetValue::Steps(steps)) => (
            from_file(sign * (*steps as f64 * period_s)),
            OffsetReading::Steps,
        ),
        Some(OffsetValue::Text(text)) => match parse_offset_text(text) {
            ParsedOffset::Steps(value) => {
                (from_file(sign * (value * period_s)), OffsetReading::Steps)
            }
            ParsedOffset::Time { seconds } => (from_file(sign * seconds), OffsetReading::Time),
            ParsedOffset::UnknownUnit { value, unit } => {
                let seconds = match fallbacks.unknown_unit {
                    UnknownUnitAs::Steps => value * period_s,
                    UnknownUnitAs::Seconds => value,
                };
                (
                    caller_supplied(sign * seconds),
                    OffsetReading::UnknownUnit { unit },
                )
            }
            ParsedOffset::Unparsed => (
                caller_supplied(fallbacks.unparsed_offset_s),
                OffsetReading::Unparsed { text: text.clone() },
            ),
        },
    }
}

/// The timing of a laser: the native period, the sign of the card, and the two offsets in seconds, with the user's overrides over the file's values. **Each case where the file is silent is
/// reported in the result and answered from `fallbacks`; nothing is defaulted here** (see the module documentation).
pub fn resolve_timing(
    recorded: &RecordedTiming,
    overrides: &TimingOverride,
    fallbacks: &TimingFallbacks,
) -> TimingResolution {
    let native_period = match recorded.native_period_s {
        Some(period) if period > 0.0 => {
            Resolved::new(period, recorded.native_period_origin.clone())
        }
        _ => caller_supplied(fallbacks.sample_period_s),
    };
    let convention = card_convention(recorded.protocol_x.as_ref(), recorded.protocol_y.as_ref());
    let sign = match convention {
        Some(known) => Resolved::new(known.sign(), None),
        None => caller_supplied(fallbacks.unrecognised_protocol.sign()),
    };
    let period = native_period.value();
    let one = |recorded: &RecordedOffset, override_steps: Option<f64>| {
        let (file_seconds, file) = read_offset(recorded, period, sign.value(), fallbacks);
        ResolvedOffset {
            seconds: override_over(file_seconds, override_steps, period),
            file,
        }
    };
    TimingResolution {
        commanded: one(&recorded.commanded, overrides.commanded_offset_steps),
        actual: one(&recorded.actual, overrides.actual_offset_steps),
        native_period,
        convention,
        sign,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallbacks() -> TimingFallbacks {
        TimingFallbacks {
            sample_period_s: 1e-5,
            absent_offset_s: 0.0,
            unparsed_offset_s: 0.0,
            unknown_unit: UnknownUnitAs::Steps,
            unrecognised_protocol: CardConvention::OppositeToShifter,
        }
    }

    fn sl2() -> Option<GalvoProtocol> {
        Some(GalvoProtocol::Sl2100)
    }

    fn xy2() -> Option<GalvoProtocol> {
        Some(GalvoProtocol::Xy2100)
    }

    #[test]
    fn text_is_read_as_vision_reads_it() {
        assert_eq!(parse_offset_text("50"), ParsedOffset::Steps(50.0));
        assert_eq!(parse_offset_text(" -2.5 "), ParsedOffset::Steps(-2.5));
        assert_eq!(parse_offset_text("+1e2"), ParsedOffset::Steps(100.0));
        assert_eq!(
            parse_offset_text("\"10 us\""),
            ParsedOffset::Time {
                seconds: 10.0 * 1e-6
            }
        );
        assert_eq!(
            parse_offset_text("'3ms'"),
            ParsedOffset::Time {
                seconds: 3.0 * 1e-3
            }
        );
        assert_eq!(
            parse_offset_text("7 NS"),
            ParsedOffset::Time {
                seconds: 7.0 * 1e-9
            }
        );
        assert_eq!(
            parse_offset_text("2 sec"),
            ParsedOffset::Time { seconds: 2.0 }
        );
        // the micro sign is a unit; the Greek letter mu is not
        assert!(matches!(
            parse_offset_text("10 \u{b5}s"),
            ParsedOffset::Time { .. }
        ));
        assert_eq!(
            parse_offset_text("10 \u{3bc}s"),
            ParsedOffset::UnknownUnit {
                value: 10.0,
                unit: "\u{3bc}s".to_string()
            }
        );
    }

    #[test]
    fn text_that_is_not_a_number_is_reported_as_unparsed() {
        for text in ["", "   ", "\"\"", "abc", "us", "- us", "''"] {
            assert_eq!(parse_offset_text(text), ParsedOffset::Unparsed, "{text:?}");
        }
        // only one pair of quotes is removed
        assert_eq!(parse_offset_text("\"\"5\"\""), ParsedOffset::Unparsed);
    }

    #[test]
    fn an_xy2_in_either_galvo_decides_and_it_is_matched_in_any_case_anywhere() {
        assert_eq!(
            card_convention(xy2().as_ref(), sl2().as_ref()),
            Some(CardConvention::AsShifter)
        );
        assert_eq!(
            card_convention(sl2().as_ref(), xy2().as_ref()),
            Some(CardConvention::AsShifter)
        );
        let odd = Some(GalvoProtocol::Other("my-xY2 card".to_string()));
        assert_eq!(
            card_convention(odd.as_ref(), None),
            Some(CardConvention::AsShifter)
        );
        assert_eq!(
            card_convention(None, odd.as_ref()),
            Some(CardConvention::AsShifter)
        );
    }

    #[test]
    fn an_sl2_card_is_opposite_and_anything_else_is_not_recognised() {
        assert_eq!(
            card_convention(sl2().as_ref(), sl2().as_ref()),
            Some(CardConvention::OppositeToShifter)
        );
        let lower = Some(GalvoProtocol::Other("sl2-100".to_string()));
        assert_eq!(
            card_convention(lower.as_ref(), None),
            Some(CardConvention::OppositeToShifter)
        );
        // none, or a name that is neither: reported, never defaulted
        assert_eq!(card_convention(None, None), None);
        let other = Some(GalvoProtocol::Other("ILDA".to_string()));
        assert_eq!(card_convention(other.as_ref(), other.as_ref()), None);
        let empty = Some(GalvoProtocol::Other(String::new()));
        assert_eq!(card_convention(empty.as_ref(), None), None);
    }

    #[test]
    fn the_sign_negates_an_sl2_offset_and_leaves_an_xy2_one() {
        let recorded = |galvo: Option<GalvoProtocol>| {
            RecordedTiming::new()
                .with_commanded(RecordedOffset::steps(50))
                .with_actual(RecordedOffset::steps(-8))
                .with_protocols(galvo.clone(), galvo)
                .with_native_period_s(1e-5)
        };
        let none = TimingOverride::new();
        let sl = resolve_timing(&recorded(sl2()), &none, &fallbacks());
        assert_eq!(sl.commanded().seconds().value(), -(50.0 * 1e-5));
        assert_eq!(sl.actual().seconds().value(), -(-8.0 * 1e-5));
        assert_eq!(sl.sign().value(), -1.0);
        assert_eq!(sl.sign().origin(), None);
        let xy = resolve_timing(&recorded(xy2()), &none, &fallbacks());
        assert_eq!(xy.commanded().seconds().value(), 50.0 * 1e-5);
        assert_eq!(xy.actual().seconds().value(), -8.0 * 1e-5);
        assert_eq!(xy.sign().value(), 1.0);
        // the file's own values have no origin: they are as recorded
        assert_eq!(xy.commanded().seconds().origin(), None);
        assert_eq!(xy.commanded().file(), &OffsetReading::Steps);
    }

    #[test]
    fn a_protocol_that_is_not_known_takes_the_callers_sign_and_says_so() {
        let recorded = RecordedTiming::new()
            .with_commanded(RecordedOffset::steps(10))
            .with_native_period_s(1e-5);
        for (answer, expected) in [
            (CardConvention::OppositeToShifter, -(10.0 * 1e-5)),
            (CardConvention::AsShifter, 10.0 * 1e-5),
        ] {
            let mut callers = fallbacks();
            callers.unrecognised_protocol = answer;
            let r = resolve_timing(&recorded, &TimingOverride::new(), &callers);
            assert_eq!(r.convention(), None);
            assert_eq!(r.sign().origin(), Some(&Origin::CallerSupplied));
            assert_eq!(r.commanded().seconds().value(), expected);
        }
    }

    #[test]
    fn what_the_file_does_not_say_is_reported_and_answered_by_the_caller() {
        let callers = TimingFallbacks {
            sample_period_s: 2e-5,
            absent_offset_s: 3e-5,
            unparsed_offset_s: 5e-5,
            unknown_unit: UnknownUnitAs::Seconds,
            unrecognised_protocol: CardConvention::AsShifter,
        };
        let recorded = RecordedTiming::new()
            .with_commanded(RecordedOffset::text("what"))
            .with_actual(RecordedOffset::text("4 furlongs"))
            .with_protocols(sl2(), sl2());
        let r = resolve_timing(&recorded, &TimingOverride::new(), &callers);
        // no sample period: the caller's, and it says so
        assert_eq!(r.native_period().value(), 2e-5);
        assert_eq!(r.native_period().origin(), Some(&Origin::CallerSupplied));
        // text that is not a number: reported, the caller's value (not signed)
        assert_eq!(r.commanded().seconds().value(), 5e-5);
        assert_eq!(
            r.commanded().seconds().origin(),
            Some(&Origin::CallerSupplied)
        );
        assert_eq!(
            r.commanded().file(),
            &OffsetReading::Unparsed {
                text: "what".to_string()
            }
        );
        // an unknown unit: the number is the file's, its meaning is the caller's (here seconds), and the sign applies
        assert_eq!(r.actual().seconds().value(), -4.0);
        assert_eq!(r.actual().seconds().origin(), Some(&Origin::CallerSupplied));
        assert_eq!(
            r.actual().file(),
            &OffsetReading::UnknownUnit {
                unit: "furlongs".to_string()
            }
        );
        // no offset at all: the caller's, with the reader's reason when it recorded one
        let reason = Origin::NullDataspace;
        let absent =
            RecordedTiming::new().with_actual(RecordedOffset::absent().with_origin(reason.clone()));
        let r = resolve_timing(&absent, &TimingOverride::new(), &callers);
        assert_eq!(r.commanded().seconds().value(), 3e-5);
        assert_eq!(r.commanded().file(), &OffsetReading::Absent { why: None });
        assert_eq!(
            r.actual().file(),
            &OffsetReading::Absent { why: Some(reason) }
        );
    }

    #[test]
    fn a_period_that_is_not_above_zero_is_unusable_and_the_callers_is_taken() {
        for bad in [0.0, -1e-5, f64::NAN] {
            let r = resolve_timing(
                &RecordedTiming::new().with_native_period_s(bad),
                &TimingOverride::new(),
                &fallbacks(),
            );
            assert_eq!(r.native_period().value(), 1e-5, "{bad}");
            assert_eq!(r.native_period().origin(), Some(&Origin::CallerSupplied));
        }
        let r = resolve_timing(
            &RecordedTiming::new().with_native_period_s(2.5e-5),
            &TimingOverride::new(),
            &fallbacks(),
        );
        assert_eq!(r.native_period().value(), 2.5e-5);
        assert_eq!(r.native_period().origin(), None);
    }

    #[test]
    fn an_override_is_steps_times_the_period_replaces_the_file_and_is_not_signed() {
        let recorded = RecordedTiming::new()
            .with_commanded(RecordedOffset::steps(50))
            .with_actual(RecordedOffset::steps(-8))
            .with_protocols(sl2(), sl2())
            .with_native_period_s(1e-5);
        let user = TimingOverride::new().with_commanded_offset_steps(3.0);
        let r = resolve_timing(&recorded, &user, &fallbacks());
        assert_eq!(r.commanded().seconds().value(), 3.0 * 1e-5);
        assert_eq!(r.commanded().seconds().origin(), Some(&Origin::Overridden));
        // the file's own offset is still reported
        assert_eq!(r.commanded().file(), &OffsetReading::Steps);
        // each offset takes only its own override: the library does not mirror one onto the other
        assert_eq!(r.actual().seconds().value(), -(-8.0 * 1e-5));
        assert_eq!(r.actual().seconds().origin(), None);
        // an override of zero is an override
        let zero = TimingOverride::new().with_actual_offset_steps(0.0);
        let r = resolve_timing(&recorded, &zero, &fallbacks());
        assert_eq!(r.actual().seconds().value(), 0.0);
        assert_eq!(r.actual().seconds().origin(), Some(&Origin::Overridden));
    }

    #[test]
    fn the_snapshot_gives_its_offsets_protocols_and_origins_and_not_the_period() {
        let mut p = LaserParameters::new();
        p.scanner_card.commanded_position_timing_offset = Some(50);
        p.galvo_x.protocol = sl2();
        p.galvo_y.protocol = sl2();
        let recorded = RecordedTiming::from_parameters(Some(&p));
        let r = resolve_timing(&recorded, &TimingOverride::new(), &fallbacks());
        assert_eq!(r.commanded().seconds().value(), -(50.0 * 1e-5));
        assert_eq!(r.commanded().file(), &OffsetReading::Steps);
        // the actual offset is not in the snapshot: absent, and the sample period is the caller's
        assert_eq!(r.actual().file(), &OffsetReading::Absent { why: None });
        assert_eq!(r.native_period().origin(), Some(&Origin::CallerSupplied));
        // a laser with no snapshot has nothing
        let none = resolve_timing(
            &RecordedTiming::from_parameters(None),
            &TimingOverride::new(),
            &fallbacks(),
        );
        assert_eq!(none.convention(), None);
        assert_eq!(
            none.commanded().file(),
            &OffsetReading::Absent { why: None }
        );
    }

    #[test]
    fn the_fallbacks_compare_and_hash_by_their_bits() {
        use std::collections::HashSet;
        let a = fallbacks();
        let mut b = fallbacks();
        assert_eq!(a, b);
        b.absent_offset_s = -0.0;
        assert_ne!(a, b, "0.0 and -0.0 are different answers");
        let mut nan = fallbacks();
        nan.sample_period_s = f64::NAN;
        assert_eq!(nan, nan, "a NaN equals itself by its bits");
        let set: HashSet<TimingFallbacks> = [a, b, nan, a].into_iter().collect();
        assert_eq!(set.len(), 3);
    }
}
