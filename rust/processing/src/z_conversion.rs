//! The linear conversion of a laser's Z from bits to millimetres, and what refuses it (IP-27 phase 27D).
//!
//! * **Layer:** Processing. It works on Model values only.
//! * **Requirement:** BL-PR-03 (the stages that produced a value are named, and a value that cannot be produced says why).
//!
//! # The formula
//!
//! `mm = code x field_size / (2^resolution - 1) - offset`, with the largest code of a resolution `2^resolution - 1` (not `2^(resolution-1)`), the offset subtracted after the scaling, a negative
//! code converted as it is (no clamp), and a signed zero kept. **The operations are in this order and are Vision's**, so that the result is the same bits ([`bits_to_mm`]).
//!
//! **This is a copy, for now.** The corrector crate (`tmscanfieldcorrector`) has the same functions (`max_code`, `validate_bit_resolution`, `bits_to_mm`), and the corrections will be ported
//! into this library with the corrector as a dependency; **when they are, this copy goes and the conversion calls the corrector's** (the plan's Appendix D lists it, with what has to be
//! settled first). The vectors pin the result, including the order of the operations, so the copy cannot drift unseen.
//!
//! # What refuses a conversion, and who decides what that means
//!
//! A conversion is made by a [`ZConverter`], **built from explicit numbers** that the caller gives (the library applies no default: see [`crate::calibration`]). [`ZConverter::new`] refuses, with a
//! [`ZRefusal`] that says why:
//!
//! * a field size or an offset that is not a finite number;
//! * a bit resolution outside `1..=32`.
//!
//! **A Z field size of zero or less is not refused**: zero gives zero millimetres for every code, and a negative size mirrors the millimetres (a behaviour of Vision's that is reproduced and
//! marked as a quirk in the spec, decision D27-6).
//!
//! **The library reports each series on its own, and Vision decides what that means** (decision D27-18). Vision today drops the **whole laser**, X and Y included, when either series is
//! refused, and checks the commanded series first; a caller that wants that builds the commanded converter, then the actual one, and takes the first refusal. A caller that wants to keep a
//! laser's X and Y when its Z cannot be converted has the information to do so.
//!
//! **Only Z is built.** [`bits_to_mm`] takes the axis so that X and Y arrive with the physical stage without changing its shape (decision D27-3); for them it says it is not built.

use std::fmt;

use build_log_model::Axis;

/// The smallest bit resolution that converts.
pub const MIN_BIT_RESOLUTION: i64 = 1;
/// The largest bit resolution that converts.
pub const MAX_BIT_RESOLUTION: i64 = 32;

/// Which of a laser's two Z series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ZSeries {
    /// Where the scanner was told to be.
    Commanded,
    /// Where the scanner was, as measured.
    Actual,
}

impl ZSeries {
    fn name(self) -> &'static str {
        match self {
            ZSeries::Commanded => "commanded Z",
            ZSeries::Actual => "actual Z",
        }
    }
}

/// Why a conversion is refused.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ZRefusal {
    /// The field size or the offset is not a finite number. `name` says which.
    NotFinite { name: &'static str, value: f64 },
    /// The bit resolution of a series is outside `1..=32`. `name` says which series.
    ResolutionOutOfRange { name: &'static str, resolution: i64 },
    /// The axis is not built yet: only Z is (decision D27-3).
    AxisNotBuilt { axis: Axis },
}

impl fmt::Display for ZRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZRefusal::NotFinite { name, value } => {
                write!(f, "{name} must be finite, got {value}")
            }
            ZRefusal::ResolutionOutOfRange { name, resolution } => write!(
                f,
                "{name} bit resolution {resolution} is outside {MIN_BIT_RESOLUTION}..={MAX_BIT_RESOLUTION}"
            ),
            ZRefusal::AxisNotBuilt { axis } => write!(
                f,
                "the conversion of {axis:?} is not built yet: only Z is (decision D27-3)"
            ),
        }
    }
}

impl std::error::Error for ZRefusal {}

/// The largest code of a resolution, `2^resolution - 1`, as a number. The resolution is `1..=32`.
fn max_code(resolution: u32) -> f64 {
    ((1_u64 << resolution) - 1) as f64
}

/// The linear conversion, in Vision's order of operations.
fn convert(code: f64, field_size_mm: f64, resolution: u32, offset_mm: f64) -> f64 {
    (code * field_size_mm / max_code(resolution)) - offset_mm
}

fn check_resolution(name: &'static str, resolution: i64) -> Result<u32, ZRefusal> {
    if (MIN_BIT_RESOLUTION..=MAX_BIT_RESOLUTION).contains(&resolution) {
        Ok(resolution as u32)
    } else {
        Err(ZRefusal::ResolutionOutOfRange { name, resolution })
    }
}

/// The millimetres of one code of `axis`: `code x field_size / (2^resolution - 1) - offset`. **Only Z is built;** X and Y are refused with [`ZRefusal::AxisNotBuilt`]. A resolution outside
/// `1..=32` is refused, and nothing else is: a field size of zero or less, or one that is not finite, converts as the formula says (use a [`ZConverter`] to have them refused).
pub fn bits_to_mm(
    axis: Axis,
    code: f64,
    field_size_mm: f64,
    resolution: u32,
    offset_mm: f64,
) -> Result<f64, ZRefusal> {
    match axis {
        Axis::Z => {
            check_resolution("Z", i64::from(resolution))?;
            Ok(convert(code, field_size_mm, resolution, offset_mm))
        }
        other => Err(ZRefusal::AxisNotBuilt { axis: other }),
    }
}

/// A checked conversion of one Z series: its field size, offset and resolution are usable, so converting a code cannot fail.
// constructed-by: library
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZConverter {
    field_size_mm: f64,
    offset_mm: f64,
    resolution: u32,
}

impl ZConverter {
    /// The converter of one series, from the numbers the caller gives, or the reason it cannot convert. **The order of the checks is Vision's:** the field size, then the offset (each must be a
    /// finite number), then the resolution of the series (`1..=32`). A field size of zero or less is not refused. `series` names the series in the refusal.
    pub fn new(
        series: ZSeries,
        field_size_mm: f64,
        offset_mm: f64,
        resolution: i64,
    ) -> Result<ZConverter, ZRefusal> {
        if !field_size_mm.is_finite() {
            return Err(ZRefusal::NotFinite {
                name: "field_size_z",
                value: field_size_mm,
            });
        }
        if !offset_mm.is_finite() {
            return Err(ZRefusal::NotFinite {
                name: "offset_z",
                value: offset_mm,
            });
        }
        Ok(ZConverter {
            field_size_mm,
            offset_mm,
            resolution: check_resolution(series.name(), resolution)?,
        })
    }

    /// The millimetres of one code.
    pub fn to_mm(&self, code: f64) -> f64 {
        convert(code, self.field_size_mm, self.resolution, self.offset_mm)
    }

    /// The bit resolution that this series converts at, `1..=32`.
    pub fn resolution(&self) -> u32 {
        self.resolution
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn converter(series: ZSeries, field: f64, offset: f64, resolution: i64) -> ZConverter {
        ZConverter::new(series, field, offset, resolution).expect("a usable calibration")
    }

    #[test]
    fn the_formula_is_the_defining_one_in_the_order_vision_has_it() {
        // the largest code of a resolution is 2^res - 1, and the offset comes off after the scaling
        assert_eq!(max_code(1), 1.0);
        assert_eq!(max_code(16), 65_535.0);
        assert_eq!(max_code(20), 1_048_575.0);
        assert_eq!(max_code(32), 4_294_967_295.0);
        let z = converter(ZSeries::Commanded, 76.5, -1.0, 20);
        // a zero code is the negative of the offset
        assert_eq!(z.to_mm(0.0), 1.0);
        // the largest code is the field size less the offset
        assert_eq!(z.to_mm(1_048_575.0), 76.5 - -1.0);
        // a negative code converts as it is (no clamp)
        assert_eq!(z.to_mm(-1_048_575.0), -76.5 - -1.0);
        // the same arithmetic, written out, for a code in the middle
        assert_eq!(z.to_mm(262_143.0), (262_143.0 * 76.5 / 1_048_575.0) - -1.0);
    }

    #[test]
    fn a_signed_zero_is_kept() {
        let z = converter(ZSeries::Actual, 400.0, 0.0, 20);
        assert_eq!(z.to_mm(0.0).to_bits(), 0.0_f64.to_bits());
        let zero_field = converter(ZSeries::Actual, 0.0, 0.0, 20);
        // a zero field size is not refused, every code gives zero, and a negative code keeps the sign of its zero
        assert_eq!(zero_field.to_mm(5.0), 0.0);
        assert_eq!(zero_field.to_mm(-5.0).to_bits(), (-0.0_f64).to_bits());
    }

    #[test]
    fn a_zero_or_negative_field_size_is_accepted_and_a_negative_one_mirrors() {
        let negative = converter(ZSeries::Commanded, -400.0, 0.0, 20);
        let positive = converter(ZSeries::Commanded, 400.0, 0.0, 20);
        for code in [1.0, 262_143.0, -1.0] {
            assert_eq!(negative.to_mm(code), -positive.to_mm(code));
        }
    }

    #[test]
    fn a_field_size_or_an_offset_that_is_not_finite_refuses_both_series_with_its_name() {
        for (field, offset, name) in [
            (f64::NAN, 0.0, "field_size_z"),
            (f64::INFINITY, 0.0, "field_size_z"),
            (400.0, f64::NAN, "offset_z"),
            (400.0, f64::NEG_INFINITY, "offset_z"),
        ] {
            for series in [ZSeries::Commanded, ZSeries::Actual] {
                match ZConverter::new(series, field, offset, 20) {
                    Err(ZRefusal::NotFinite { name: found, .. }) => assert_eq!(found, name),
                    other => panic!("{other:?}"),
                }
            }
        }
        // the field size is checked before the offset, as Vision's pipeline does
        match ZConverter::new(ZSeries::Commanded, f64::NAN, f64::NAN, 20) {
            Err(ZRefusal::NotFinite { name, .. }) => assert_eq!(name, "field_size_z"),
            other => panic!("{other:?}"),
        }
        let text = ZConverter::new(ZSeries::Commanded, f64::NAN, 0.0, 20)
            .unwrap_err()
            .to_string();
        assert_eq!(text, "field_size_z must be finite, got NaN");
    }

    #[test]
    fn a_resolution_outside_one_to_thirty_two_refuses_its_own_series_only() {
        for (bits, refused) in [(0, true), (1, false), (32, false), (33, true), (-4, true)] {
            assert_eq!(
                ZConverter::new(ZSeries::Commanded, 400.0, 0.0, bits).is_err(),
                refused,
                "{bits}"
            );
            // the other series, with a good resolution, is not affected by this one
            assert!(ZConverter::new(ZSeries::Actual, 400.0, 0.0, 20).is_ok());
        }
        let text = ZConverter::new(ZSeries::Actual, 400.0, 0.0, -4)
            .unwrap_err()
            .to_string();
        assert_eq!(text, "actual Z bit resolution -4 is outside 1..=32");
        let text = ZConverter::new(ZSeries::Commanded, 400.0, 0.0, 33)
            .unwrap_err()
            .to_string();
        assert_eq!(text, "commanded Z bit resolution 33 is outside 1..=32");
    }

    #[test]
    fn the_largest_resolution_converts_without_overflow() {
        let z = converter(ZSeries::Commanded, 400.0, 0.0, 32);
        assert_eq!(z.resolution(), 32);
        assert_eq!(z.to_mm(4_294_967_295.0), 400.0);
        assert_eq!(
            z.to_mm(f64::from(i32::MIN)),
            f64::from(i32::MIN) * 400.0 / 4_294_967_295.0
        );
    }

    #[test]
    fn only_z_is_built_and_x_and_y_say_so() {
        assert_eq!(bits_to_mm(Axis::Z, 1_048_575.0, 400.0, 20, 0.0), Ok(400.0));
        for axis in [Axis::X, Axis::Y] {
            assert_eq!(
                bits_to_mm(axis, 1.0, 400.0, 20, 0.0),
                Err(ZRefusal::AxisNotBuilt { axis })
            );
        }
        assert!(bits_to_mm(Axis::X, 1.0, 400.0, 20, 0.0)
            .unwrap_err()
            .to_string()
            .contains("not built yet"));
        // a resolution that cannot be used is refused for Z too, and does not overflow a shift
        for bad in [0_u32, 33, 64, u32::MAX] {
            assert!(bits_to_mm(Axis::Z, 1.0, 400.0, bad, 0.0).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_refusal_is_a_value_that_can_be_sent_and_is_an_error() {
        fn assert_traits<T: std::error::Error + Send + Sync + Clone + 'static>() {}
        assert_traits::<ZRefusal>();
    }
}
