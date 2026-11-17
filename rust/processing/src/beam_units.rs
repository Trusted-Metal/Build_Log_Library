//! The units of the beam's stored values, and how a stored number is brought to the unit the resolver works in (IP-27 phase 27H, decision D27-23).
//!
//! * **Layer:** Processing. It works on Model values only.
//! * **Requirement:** BL-PR-03 (a value says where it came from), BL-CD-06 (a unit is reported when the file states one, and nothing is guessed).
//!
//! # What the format means: the library's, and Vision's table
//!
//! A length or an angle is stored with a unit companion in the current files and without one in the older ones. **The table of the words is Vision's, as it is** (its reader's
//! `length_unit_to_mm` and `angle_unit_to_degrees`): the unit text is trimmed and lowercased, and the micro sign (U+00B5) is read as the Greek mu (U+03BC).
//!
//! | Kind | Words | Multiplier |
//! |---|---|---|
//! | length, to millimetres | `nm` | 1e-6 |
//! | | `μm`, `µm`, `um`, `micron`, `microns`, `micrometer`, `micrometers` | 1e-3 |
//! | | `mm`, `millimeter`, `millimeters` | 1 |
//! | | `cm` | 10 |
//! | | `m` (so `M` too) | 1000 |
//! | angle, to degrees | `deg`, `degree`, `degrees`, `°` | 1 |
//! | | `rad`, `radian`, `radians` | 180/π |
//!
//! **The Model's own vocabulary has fewer words** (`um`, `mm`, `nm`, `deg`, with the micro prefix normalised): it has no `cm`, no `m`, no `micron(s)` and no `micrometer(s)`. This table is
//! used on the spelling the file wrote, so a unit that Vision converts is converted here too, and the two agree to the last bit.
//!
//! **The arithmetic is Vision's:** a length is `value x to_mm / target`, where `to_mm` is the multiplier of the file's unit (or of the assumed one) and `target` is the multiplier of the unit the
//! resolver works in. **That is not an identity even from micrometres to micrometres:** `7.857142857142857 x 1e-3 / 1e-3` is not 7.857142857142857 (the values of the files we hold happen to survive
//! it, and a unit test pins one that does not); a conversion between two different units is pinned by the vectors.
//!
//! # What the file does not say: the caller's
//!
//! A value with **no unit, an empty one or one the table does not know** is taken in the unit the caller assumes ([`crate::beam::BeamAssumptions`]): Vision assumes micrometres for a waist,
//! millimetres for the other lengths and degrees for an angle.

use build_log_model::Quantity;

/// A unit of length that the resolver can be told to assume, with its multiplier to millimetres (Vision's table).
// closed: these are the units of Vision's table, each with its multiplier; a unit that is not in the table cannot be assumed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LengthUnit {
    Nanometre,
    Micrometre,
    Millimetre,
    Centimetre,
    Metre,
}

impl LengthUnit {
    /// How many millimetres one of this unit is: the multiplier of Vision's table.
    pub fn to_millimetres(self) -> f64 {
        match self {
            LengthUnit::Nanometre => 1e-6,
            LengthUnit::Micrometre => 1e-3,
            LengthUnit::Millimetre => 1.0,
            LengthUnit::Centimetre => 10.0,
            LengthUnit::Metre => 1000.0,
        }
    }
}

/// A unit of angle that the resolver can be told to assume, with its multiplier to degrees (Vision's table).
// closed: these are the units of Vision's table, each with its multiplier; a unit that is not in the table cannot be assumed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AngleUnit {
    Degree,
    Radian,
}

impl AngleUnit {
    /// How many degrees one of this unit is: the multiplier of Vision's table.
    pub fn to_degrees(self) -> f64 {
        match self {
            AngleUnit::Degree => 1.0,
            AngleUnit::Radian => 180.0 / std::f64::consts::PI,
        }
    }
}

/// The multiplier to millimetres of a length unit as a file spells it, or `None` for a word that is not in the table (an empty unit included).
pub fn length_multiplier(unit: &str) -> Option<f64> {
    match unit
        .trim()
        .to_lowercase()
        .replace('\u{00b5}', "\u{03bc}")
        .as_str()
    {
        "nm" => Some(1e-6),
        "\u{03bc}m" | "um" | "micron" | "microns" | "micrometer" | "micrometers" => Some(1e-3),
        "mm" | "millimeter" | "millimeters" => Some(1.0),
        "cm" => Some(10.0),
        "m" => Some(1000.0),
        _ => None,
    }
}

/// The multiplier to degrees of an angle unit as a file spells it, or `None` for a word that is not in the table (an empty unit included).
pub fn angle_multiplier(unit: &str) -> Option<f64> {
    match unit.trim().to_lowercase().as_str() {
        "deg" | "degree" | "degrees" | "\u{00b0}" => Some(1.0),
        "rad" | "radian" | "radians" => Some(180.0 / std::f64::consts::PI),
        _ => None,
    }
}

/// Whether the file states a unit that the length table knows.
pub fn length_unit_stated(quantity: &Quantity) -> bool {
    quantity
        .unit
        .spelling
        .as_deref()
        .and_then(length_multiplier)
        .is_some()
}

/// Whether the file states a unit that the angle table knows.
pub fn angle_unit_stated(quantity: &Quantity) -> bool {
    quantity
        .unit
        .spelling
        .as_deref()
        .and_then(angle_multiplier)
        .is_some()
}

/// A stored length in the unit `target`: `value x to_mm / target`, where `to_mm` is the file's unit if the table knows it and `assumed` otherwise. **Vision's arithmetic, in its order.**
pub fn convert_length(quantity: &Quantity, assumed: LengthUnit, target: LengthUnit) -> f64 {
    let to_mm = quantity
        .unit
        .spelling
        .as_deref()
        .and_then(length_multiplier)
        .unwrap_or_else(|| assumed.to_millimetres());
    quantity.value * to_mm / target.to_millimetres()
}

/// A stored angle in degrees: `value x to_degrees`, where `to_degrees` is the file's unit if the table knows it and `assumed` otherwise. **Vision's arithmetic.**
pub fn convert_angle(quantity: &Quantity, assumed: AngleUnit) -> f64 {
    let to_degrees = quantity
        .unit
        .spelling
        .as_deref()
        .and_then(angle_multiplier)
        .unwrap_or_else(|| assumed.to_degrees());
    quantity.value * to_degrees
}

/// A wavelength in nanometres: **the number as it is recorded, taken in the unit the caller assumes** (decision D27-18, Vision's quirk, kept in version 1): the unit that the file states is not
/// consulted (it stays in the quantity, as data, for any consumer that wants a guard). A wavelength taken in nanometres is returned **as it is**.
pub fn wavelength_in_nanometres(value: f64, assumed: LengthUnit) -> f64 {
    if assumed == LengthUnit::Nanometre {
        value
    } else {
        value * assumed.to_millimetres() / LengthUnit::Nanometre.to_millimetres()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(value: f64, unit: Option<&str>) -> Quantity {
        Quantity::from_file(value, unit)
    }

    #[test]
    fn the_length_table_is_visions_with_the_micro_sign_read_as_the_greek_mu() {
        for word in [
            "\u{03bc}m",
            "\u{b5}m",
            "um",
            "UM",
            "  um  ",
            "micron",
            "Microns",
            "micrometer",
            "micrometers",
        ] {
            assert_eq!(length_multiplier(word), Some(1e-3), "{word:?}");
        }
        assert_eq!(length_multiplier("nm"), Some(1e-6));
        for word in ["mm", "Millimeter", "millimeters"] {
            assert_eq!(length_multiplier(word), Some(1.0), "{word:?}");
        }
        assert_eq!(length_multiplier("cm"), Some(10.0));
        // a capital M is a metre, as in Vision
        assert_eq!(length_multiplier("m"), Some(1000.0));
        assert_eq!(length_multiplier("M"), Some(1000.0));
        for word in ["", "   ", "furlongs", "mu m", "km"] {
            assert_eq!(length_multiplier(word), None, "{word:?}");
        }
    }

    #[test]
    fn the_angle_table_is_visions() {
        for word in ["deg", "degree", "Degrees", "\u{b0}", "  DEG "] {
            assert_eq!(angle_multiplier(word), Some(1.0), "{word:?}");
        }
        for word in ["rad", "radian", "Radians", " RAD "] {
            assert_eq!(
                angle_multiplier(word),
                Some(180.0 / std::f64::consts::PI),
                "{word:?}"
            );
        }
        for word in ["", "turns", "gon"] {
            assert_eq!(angle_multiplier(word), None, "{word:?}");
        }
    }

    #[test]
    fn a_length_is_value_times_to_mm_over_target_and_that_is_not_an_identity() {
        // micrometres to micrometres is `value x 1e-3 / 1e-3`, which is not the value for every value
        let awkward = 7.857_142_857_142_857;
        assert_ne!(
            awkward * 1e-3 / 1e-3,
            awkward,
            "the counter-example is still one"
        );
        let converted = convert_length(
            &q(awkward, Some("\u{3bc}m")),
            LengthUnit::Micrometre,
            LengthUnit::Micrometre,
        );
        assert_eq!(converted.to_bits(), (awkward * 1e-3 / 1e-3).to_bits());
        assert_ne!(converted, awkward);
        // and the order matters: the product first, then the quotient
        assert_ne!(converted, awkward * (1e-3 / 1e-3));
        // the values of the files we hold survive it
        for v in [66.73, 66.7297, 68.1252, 66.0, 68.0] {
            let same = convert_length(
                &q(v, Some("um")),
                LengthUnit::Micrometre,
                LengthUnit::Micrometre,
            );
            assert_eq!(same, v);
        }
        // between different units
        assert_eq!(
            convert_length(
                &q(66.73, Some("um")),
                LengthUnit::Millimetre,
                LengthUnit::Millimetre
            ),
            66.73 * 1e-3 / 1.0
        );
        assert_eq!(
            convert_length(
                &q(0.457, Some("cm")),
                LengthUnit::Millimetre,
                LengthUnit::Millimetre
            ),
            0.457 * 10.0 / 1.0
        );
        assert_eq!(
            convert_length(
                &q(1.178, Some("m")),
                LengthUnit::Millimetre,
                LengthUnit::Millimetre
            ),
            1.178 * 1000.0 / 1.0
        );
        assert_eq!(
            convert_length(
                &q(66.73, Some("nm")),
                LengthUnit::Micrometre,
                LengthUnit::Micrometre
            ),
            66.73 * 1e-6 / 1e-3
        );
    }

    #[test]
    fn a_length_with_no_unit_an_empty_one_or_an_unknown_one_takes_the_assumed_unit() {
        for unit in [None, Some(""), Some("furlongs")] {
            let quantity = q(2.5, unit);
            assert!(!length_unit_stated(&quantity), "{unit:?}");
            assert_eq!(
                convert_length(&quantity, LengthUnit::Nanometre, LengthUnit::Millimetre),
                2.5 * 1e-6 / 1.0,
                "{unit:?}"
            );
            assert_eq!(
                convert_length(&quantity, LengthUnit::Millimetre, LengthUnit::Millimetre),
                2.5 * 1.0 / 1.0
            );
        }
        // a stated unit beats the assumed one
        let stated = q(2.5, Some("mm"));
        assert!(length_unit_stated(&stated));
        assert_eq!(
            convert_length(&stated, LengthUnit::Nanometre, LengthUnit::Millimetre),
            2.5
        );
    }

    #[test]
    fn an_angle_is_value_times_to_degrees_and_no_known_unit_takes_the_assumed_one() {
        assert_eq!(
            convert_angle(&q(12.5, Some("degrees")), AngleUnit::Radian),
            12.5
        );
        assert_eq!(
            convert_angle(&q(std::f64::consts::PI, Some("rad")), AngleUnit::Degree),
            std::f64::consts::PI * (180.0 / std::f64::consts::PI)
        );
        // no unit: the assumed one (degrees for Vision; radians if the caller says so)
        assert_eq!(convert_angle(&q(12.5, None), AngleUnit::Degree), 12.5);
        assert_eq!(
            convert_angle(&q(1.0, Some("turns")), AngleUnit::Radian),
            180.0 / std::f64::consts::PI
        );
        assert!(!angle_unit_stated(&q(1.0, Some("turns"))));
        assert!(angle_unit_stated(&q(1.0, Some("\u{b0}"))));
    }

    #[test]
    fn a_wavelength_taken_in_nanometres_is_the_number_as_recorded() {
        for v in [1070.0, 1064.0, 0.0, f64::NAN, -0.0] {
            assert_eq!(
                wavelength_in_nanometres(v, LengthUnit::Nanometre).to_bits(),
                v.to_bits()
            );
        }
        // a caller that says micrometres gets nanometres
        assert_eq!(
            wavelength_in_nanometres(1.07, LengthUnit::Micrometre),
            1.07 * 1e-3 / 1e-6
        );
    }
}
