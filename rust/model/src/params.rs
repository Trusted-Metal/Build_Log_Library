//! The parameter snapshot: per-laser parameters, equations as a type plus named constants, and (later) the descriptors of correction grids and files.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** streams.
//! * **Constructed by:** library (`LaserParameters` and the parts it holds, `Equation`, `NamedConstant`); the enums `GalvoProtocol`, `EquationKind` and `ParameterValue`.
//!   (The plan listed the producers. They are library-built here, with `Default`, because a snapshot has some seventy optional values and a producer sets the ones it
//!   knows: a value added later must not break a producer's build. Decision D13-25.)
//! * **Requirement:** BL-CD-12.
//!
//! # What is here so far
//!
//! The part of IP-02 phase 2N that IP-13 phase 13H needs: what a V17 file holds for a laser, which is the calibration of its optical train. **Parameters are held per
//! laser** (decision D13-18), with the number of the optical train kept as a label. The snapshot has the scanner, the three galvos, the scanner card, the light source
//! (with the volts-to-watts equation as named constants) and the optics of the train. **The correction grids and the correction file are not here:** what a grid is
//! indexed by and how it is laid out is the open question Q2-5 (c), so they stay kept, untyped, in `extra`. ADC scale and the decode conventions come with the versions
//! that record them.
//!
//! # What this module claims
//!
//! * **Every value is as the file gave it.** A number keeps its value (a number stored as an integer is the same number), and a quantity says its unit only if the file
//!   does: the unit of a value with no unit attribute is unknown, and nothing is guessed.
//! * **An absent value is absent.** Every value is an `Option`; a laser with no calibration has no snapshot at all.
//! * **Equations are a type and named constants**, never a positional string: the names are V18's (`b` and `a` for a line, `c0`, `c1` and so on for a polynomial), the
//!   intercept first.
//!
//! The policy guards in `tests/policy_guards.rs` scan this file from now on: every enum is `#[non_exhaustive]` or says `// closed: <reason>`, every public struct says
//! `// constructed-by: producer|library|caller-options` and follows that rule, and none holds bulk data.

use crate::error::{Error, ErrorKind, Violation};
use crate::extra::Extra;
use crate::grids::CorrectionGrid;
use crate::provenance::Origins;
use crate::units::Quantity;

/// What the origin of a scan-head offset says when its sign is opposite to the sign the same offset has in other versions' files. The relation is **inferred** from the magnitudes of one machine's two
/// lasers (V:CP-10), so the sentence says so, and **the value is kept as written and never flipped** (IP-12 decision D12-6).
pub const SCAN_HEAD_OFFSET_SIGN_OPPOSITE: &str =
    "scan-head offset: its sign is opposite to other versions' (inferred), kept as written";
/// What the origin of a galvo's bit resolution says when the file gave one value for every laser and every axis, and the adapter gave it to this laser's galvo (IP-12 phase 12B, IP-24 finding F2).
pub const BIT_RESOLUTION_GLOBAL: &str =
    "bit resolution: one value in the file for every laser and axis";

/// The protocol a galvo is driven with. Only the words that are known are named, and they are matched exactly as written.
// open-domain: a machine may use a protocol that nobody has listed, and it is kept as the file spells it
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GalvoProtocol {
    /// `XY2-100`: the 16-bit protocol.
    Xy2100,
    /// `SL2-100`: the 20-bit protocol that the V17 files hold.
    Sl2100,
    /// A protocol that this library has no name for, exactly as the file spells it.
    Other(String),
}

/// The algorithm of an equation. Only the words that are known are named, and they are matched exactly as written.
// open-domain: a writer may use an algorithm that nobody has listed, and it is kept as the file spells it
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EquationKind {
    /// `W = a*V + b`: a line, with the constants `b` (the intercept) and `a` (the slope), in that order.
    Linear,
    /// `W = c0 + c1*V + c2*V^2 + ...`: a polynomial, with the constants `c0`, `c1`, and so on, in that order.
    Polynomial,
    /// An algorithm that this library has no name for, exactly as the file spells it. Its constants are not named, so there are none.
    Other(String),
}

/// A constant of an equation, by name. **Match on the name, never on the position** (the order of a list means nothing).
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct NamedConstant {
    pub name: String,
    pub value: f64,
}

impl NamedConstant {
    pub fn new(name: impl Into<String>, value: f64) -> Self {
        NamedConstant {
            name: name.into(),
            value,
        }
    }
}

/// An equation: its algorithm and its constants, by name.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Equation {
    pub kind: EquationKind,
    pub constants: Vec<NamedConstant>,
}

impl Equation {
    /// An equation of this kind with no constants yet.
    pub fn new(kind: EquationKind) -> Self {
        Equation {
            kind,
            constants: Vec::new(),
        }
    }

    /// The same, with one more constant.
    pub fn with_constant(mut self, name: impl Into<String>, value: f64) -> Self {
        self.constants.push(NamedConstant::new(name, value));
        self
    }

    /// The value of the constant of this name, if the equation has one.
    pub fn constant(&self, name: &str) -> Option<f64> {
        self.constants
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.value)
    }
}

/// The optics of an optical train: what identifies it, its beam and its thermal-lensing test. Several values are absent from the March files, which have no units.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct OpticalTrainParameters {
    pub id: Option<String>,
    pub scanner_number: Option<String>,
    pub metadata: Option<String>,
    pub profile_type: Option<String>,
    pub waist_definition: Option<String>,
    pub waist_major: Option<Quantity>,
    pub waist_minor: Option<Quantity>,
    pub waist_offset_z: Option<Quantity>,
    pub build_plane_offset_major: Option<Quantity>,
    pub build_plane_offset_minor: Option<Quantity>,
    pub collimator_focal_length: Option<Quantity>,
    pub m2_major: Option<f64>,
    pub m2_minor: Option<f64>,
    pub major_axis_angle: Option<Quantity>,
    pub major_waist_d4sigma: Option<Quantity>,
    pub minor_waist_d4sigma: Option<Quantity>,
    pub nominal_working_distance: Option<Quantity>,
    pub default_position: Option<Quantity>,
    pub moveable: Option<bool>,
    pub position: Option<Quantity>,
    pub rayleigh_length_major: Option<Quantity>,
    pub rayleigh_length_minor: Option<Quantity>,
    pub thermal_lensing_focal_plane_shift: Option<Quantity>,
    pub thermal_lensing_test_passed: Option<bool>,
    pub thermal_lensing_threshold: Option<Quantity>,
}

/// The scan head: its model and serial number, the field it covers, where it sits and how it is turned and scaled.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ScannerParameters {
    pub model: Option<String>,
    pub serial: Option<String>,
    pub field_size_x: Option<Quantity>,
    pub field_size_y: Option<Quantity>,
    pub field_size_z: Option<Quantity>,
    pub offset_x: Option<Quantity>,
    pub offset_y: Option<Quantity>,
    pub offset_z: Option<Quantity>,
    pub rotation: Option<Quantity>,
    pub scaling_x: Option<f64>,
    pub scaling_y: Option<f64>,
    pub x_galvo_to_exit_aperture: Option<Quantity>,
    pub y_galvo_to_exit_aperture: Option<Quantity>,
}

/// One galvo (an axis of the scan head): its bit widths, protocol, range of motion, smoothing and tuning.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct GalvoParameters {
    pub actual_bit_resolution: Option<i64>,
    pub commanded_bit_resolution: Option<i64>,
    pub protocol: Option<GalvoProtocol>,
    pub range_of_motion: Option<Quantity>,
    pub smoothing_kernel: Option<String>,
    pub smoothing_parameters: Option<f64>,
    pub tuning_parameters: Option<String>,
    pub tuning_type: Option<String>,
}

/// The scanner card: when each position is sampled, and how often. The offsets are whole numbers whose unit the file does not state.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ScannerCardParameters {
    pub actual_position_timing_offset: Option<i64>,
    pub commanded_position_timing_offset: Option<i64>,
    pub sample_period: Option<Quantity>,
}

/// The light source: the laser's model and serial number, its wavelength and power limits, and the equation from the volts of its power channel to watts.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct LightSourceParameters {
    pub model: Option<String>,
    pub serial: Option<String>,
    pub wavelength: Option<Quantity>,
    pub power_bit_resolution: Option<i64>,
    pub power_max_actual: Option<Quantity>,
    pub power_max_nominal: Option<Quantity>,
    pub power_min_actual: Option<Quantity>,
    pub power_min_nominal: Option<Quantity>,
    pub volts_to_watts: Option<Equation>,
}

/// A value of the snapshot, by kind, as [`LaserParameters::values`] lists it.
// closed: a value is a quantity, a plain number, a whole number, a flag, text or a protocol, and a consumer matches on which
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterValue {
    /// A number with a unit that the file states, or that is unknown.
    Quantity(Quantity),
    /// A number that has no unit (a scaling, a smoothing parameter).
    Plain(f64),
    /// A whole number (a bit width, a timing offset).
    Whole(i64),
    Flag(bool),
    Text(String),
    Protocol(GalvoProtocol),
}

/// The parameters of one laser, as its file holds them: the calibration of its optical train. Laser N uses optical train N in V17 (decision D13-18).
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct LaserParameters {
    /// The number of the optical train the values come from, as a label (decision D02-8). V17 does not say which train a laser uses, so the adapter pairs them by
    /// number and its origin says so.
    pub optical_train_number: Option<u32>,
    pub optics: OpticalTrainParameters,
    pub scanner: ScannerParameters,
    pub galvo_x: GalvoParameters,
    pub galvo_y: GalvoParameters,
    pub galvo_z: GalvoParameters,
    pub scanner_card: ScannerCardParameters,
    pub light_source: LightSourceParameters,
    /// How a value came to be what it is, when that is not simply "the file said so", by the name of the value (`light_source.volts_to_watts`).
    pub origins: Origins,
    /// The correction grids of the laser, as recorded (IP-24 phase 24B): the descriptor of each, in the order the file names them. **The values are read through `ColumnSource::read_grid`**, as stored;
    /// the library does not interpret a grid.
    pub grids: Vec<CorrectionGrid>,
    /// What the adapter did not type of the calibration: attributes and nodes below the optical train, by their path (the grids are typed and are not here).
    pub extra: Extra,
}

impl LaserParameters {
    pub fn new() -> Self {
        LaserParameters::default()
    }

    /// Every value that is set, by name, in name order. A value that is absent is not listed, and the equation is not here (see
    /// [`LightSourceParameters::volts_to_watts`]).
    pub fn values(&self) -> Vec<(String, ParameterValue)> {
        let mut found = Vec::new();
        let mut add = |name: &str, value: Option<ParameterValue>| {
            if let Some(value) = value {
                found.push((name.to_string(), value));
            }
        };
        let q = |v: &Option<Quantity>| v.clone().map(ParameterValue::Quantity);
        let p = |v: &Option<f64>| v.map(ParameterValue::Plain);
        let w = |v: &Option<i64>| v.map(ParameterValue::Whole);
        let f = |v: &Option<bool>| v.map(ParameterValue::Flag);
        let t = |v: &Option<String>| v.clone().map(ParameterValue::Text);

        let o = &self.optics;
        add("optics.id", t(&o.id));
        add("optics.scanner_number", t(&o.scanner_number));
        add("optics.metadata", t(&o.metadata));
        add("optics.profile_type", t(&o.profile_type));
        add("optics.waist_definition", t(&o.waist_definition));
        add("optics.waist_major", q(&o.waist_major));
        add("optics.waist_minor", q(&o.waist_minor));
        add("optics.waist_offset_z", q(&o.waist_offset_z));
        add(
            "optics.build_plane_offset_major",
            q(&o.build_plane_offset_major),
        );
        add(
            "optics.build_plane_offset_minor",
            q(&o.build_plane_offset_minor),
        );
        add(
            "optics.collimator_focal_length",
            q(&o.collimator_focal_length),
        );
        add("optics.m2_major", p(&o.m2_major));
        add("optics.m2_minor", p(&o.m2_minor));
        add("optics.major_axis_angle", q(&o.major_axis_angle));
        add("optics.major_waist_d4sigma", q(&o.major_waist_d4sigma));
        add("optics.minor_waist_d4sigma", q(&o.minor_waist_d4sigma));
        add(
            "optics.nominal_working_distance",
            q(&o.nominal_working_distance),
        );
        add("optics.default_position", q(&o.default_position));
        add("optics.moveable", f(&o.moveable));
        add("optics.position", q(&o.position));
        add("optics.rayleigh_length_major", q(&o.rayleigh_length_major));
        add("optics.rayleigh_length_minor", q(&o.rayleigh_length_minor));
        add(
            "optics.thermal_lensing_focal_plane_shift",
            q(&o.thermal_lensing_focal_plane_shift),
        );
        add(
            "optics.thermal_lensing_test_passed",
            f(&o.thermal_lensing_test_passed),
        );
        add(
            "optics.thermal_lensing_threshold",
            q(&o.thermal_lensing_threshold),
        );

        let s = &self.scanner;
        add("scanner.model", t(&s.model));
        add("scanner.serial", t(&s.serial));
        add("scanner.field_size_x", q(&s.field_size_x));
        add("scanner.field_size_y", q(&s.field_size_y));
        add("scanner.field_size_z", q(&s.field_size_z));
        add("scanner.offset_x", q(&s.offset_x));
        add("scanner.offset_y", q(&s.offset_y));
        add("scanner.offset_z", q(&s.offset_z));
        add("scanner.rotation", q(&s.rotation));
        add("scanner.scaling_x", p(&s.scaling_x));
        add("scanner.scaling_y", p(&s.scaling_y));
        add(
            "scanner.x_galvo_to_exit_aperture",
            q(&s.x_galvo_to_exit_aperture),
        );
        add(
            "scanner.y_galvo_to_exit_aperture",
            q(&s.y_galvo_to_exit_aperture),
        );

        for (prefix, g) in [
            ("galvo_x", &self.galvo_x),
            ("galvo_y", &self.galvo_y),
            ("galvo_z", &self.galvo_z),
        ] {
            let name = |what: &str| format!("{prefix}.{what}");
            add(&name("actual_bit_resolution"), w(&g.actual_bit_resolution));
            add(
                &name("commanded_bit_resolution"),
                w(&g.commanded_bit_resolution),
            );
            add(
                &name("protocol"),
                g.protocol.clone().map(ParameterValue::Protocol),
            );
            add(&name("range_of_motion"), q(&g.range_of_motion));
            add(&name("smoothing_kernel"), t(&g.smoothing_kernel));
            add(&name("smoothing_parameters"), p(&g.smoothing_parameters));
            add(&name("tuning_parameters"), t(&g.tuning_parameters));
            add(&name("tuning_type"), t(&g.tuning_type));
        }

        let c = &self.scanner_card;
        add(
            "scanner_card.actual_position_timing_offset",
            w(&c.actual_position_timing_offset),
        );
        add(
            "scanner_card.commanded_position_timing_offset",
            w(&c.commanded_position_timing_offset),
        );
        add("scanner_card.sample_period", q(&c.sample_period));

        let l = &self.light_source;
        add("light_source.model", t(&l.model));
        add("light_source.serial", t(&l.serial));
        add("light_source.wavelength", q(&l.wavelength));
        add(
            "light_source.power_bit_resolution",
            w(&l.power_bit_resolution),
        );
        add("light_source.power_max_actual", q(&l.power_max_actual));
        add("light_source.power_max_nominal", q(&l.power_max_nominal));
        add("light_source.power_min_actual", q(&l.power_min_actual));
        add("light_source.power_min_nominal", q(&l.power_min_nominal));

        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    /// Sets the value of this name, the inverse of [`LaserParameters::values`]: the names are the same, and a value of the wrong kind for its name is refused. **This is
    /// the one place that maps a name to a field**, so an adapter says which attribute holds which name and nothing else.
    pub fn set(&mut self, name: &str, value: ParameterValue) -> Result<(), Error> {
        let refuse = |why: String| {
            Error::new(ErrorKind::ValidationError {
                violations: vec![Violation::new(why)],
            })
        };
        let wrong =
            |kind: &str| refuse(format!("`{name}` holds {kind}, and was given another kind"));
        let q = |v: ParameterValue| match v {
            ParameterValue::Quantity(q) => Ok(Some(q)),
            _ => Err(wrong("a quantity")),
        };
        let p = |v: ParameterValue| match v {
            ParameterValue::Plain(n) => Ok(Some(n)),
            _ => Err(wrong("a plain number")),
        };
        let w = |v: ParameterValue| match v {
            ParameterValue::Whole(n) => Ok(Some(n)),
            _ => Err(wrong("a whole number")),
        };
        let f = |v: ParameterValue| match v {
            ParameterValue::Flag(b) => Ok(Some(b)),
            _ => Err(wrong("a flag")),
        };
        let t = |v: ParameterValue| match v {
            ParameterValue::Text(text) => Ok(Some(text)),
            _ => Err(wrong("text")),
        };
        let o = &mut self.optics;
        let s = &mut self.scanner;
        let c = &mut self.scanner_card;
        let l = &mut self.light_source;
        match name {
            "optics.id" => o.id = t(value)?,
            "optics.scanner_number" => o.scanner_number = t(value)?,
            "optics.metadata" => o.metadata = t(value)?,
            "optics.profile_type" => o.profile_type = t(value)?,
            "optics.waist_definition" => o.waist_definition = t(value)?,
            "optics.waist_major" => o.waist_major = q(value)?,
            "optics.waist_minor" => o.waist_minor = q(value)?,
            "optics.waist_offset_z" => o.waist_offset_z = q(value)?,
            "optics.build_plane_offset_major" => o.build_plane_offset_major = q(value)?,
            "optics.build_plane_offset_minor" => o.build_plane_offset_minor = q(value)?,
            "optics.collimator_focal_length" => o.collimator_focal_length = q(value)?,
            "optics.m2_major" => o.m2_major = p(value)?,
            "optics.m2_minor" => o.m2_minor = p(value)?,
            "optics.major_axis_angle" => o.major_axis_angle = q(value)?,
            "optics.major_waist_d4sigma" => o.major_waist_d4sigma = q(value)?,
            "optics.minor_waist_d4sigma" => o.minor_waist_d4sigma = q(value)?,
            "optics.nominal_working_distance" => o.nominal_working_distance = q(value)?,
            "optics.default_position" => o.default_position = q(value)?,
            "optics.moveable" => o.moveable = f(value)?,
            "optics.position" => o.position = q(value)?,
            "optics.rayleigh_length_major" => o.rayleigh_length_major = q(value)?,
            "optics.rayleigh_length_minor" => o.rayleigh_length_minor = q(value)?,
            "optics.thermal_lensing_focal_plane_shift" => {
                o.thermal_lensing_focal_plane_shift = q(value)?
            }
            "optics.thermal_lensing_test_passed" => o.thermal_lensing_test_passed = f(value)?,
            "optics.thermal_lensing_threshold" => o.thermal_lensing_threshold = q(value)?,
            "scanner.model" => s.model = t(value)?,
            "scanner.serial" => s.serial = t(value)?,
            "scanner.field_size_x" => s.field_size_x = q(value)?,
            "scanner.field_size_y" => s.field_size_y = q(value)?,
            "scanner.field_size_z" => s.field_size_z = q(value)?,
            "scanner.offset_x" => s.offset_x = q(value)?,
            "scanner.offset_y" => s.offset_y = q(value)?,
            "scanner.offset_z" => s.offset_z = q(value)?,
            "scanner.rotation" => s.rotation = q(value)?,
            "scanner.scaling_x" => s.scaling_x = p(value)?,
            "scanner.scaling_y" => s.scaling_y = p(value)?,
            "scanner.x_galvo_to_exit_aperture" => s.x_galvo_to_exit_aperture = q(value)?,
            "scanner.y_galvo_to_exit_aperture" => s.y_galvo_to_exit_aperture = q(value)?,
            "scanner_card.actual_position_timing_offset" => {
                c.actual_position_timing_offset = w(value)?
            }
            "scanner_card.commanded_position_timing_offset" => {
                c.commanded_position_timing_offset = w(value)?
            }
            "scanner_card.sample_period" => c.sample_period = q(value)?,
            "light_source.model" => l.model = t(value)?,
            "light_source.serial" => l.serial = t(value)?,
            "light_source.wavelength" => l.wavelength = q(value)?,
            "light_source.power_bit_resolution" => l.power_bit_resolution = w(value)?,
            "light_source.power_max_actual" => l.power_max_actual = q(value)?,
            "light_source.power_max_nominal" => l.power_max_nominal = q(value)?,
            "light_source.power_min_actual" => l.power_min_actual = q(value)?,
            "light_source.power_min_nominal" => l.power_min_nominal = q(value)?,
            other => {
                let Some((galvo, field)) = other.split_once('.') else {
                    return Err(refuse(format!("`{other}` is not a parameter")));
                };
                let g = match galvo {
                    "galvo_x" => &mut self.galvo_x,
                    "galvo_y" => &mut self.galvo_y,
                    "galvo_z" => &mut self.galvo_z,
                    _ => return Err(refuse(format!("`{other}` is not a parameter"))),
                };
                match field {
                    "actual_bit_resolution" => g.actual_bit_resolution = w(value)?,
                    "commanded_bit_resolution" => g.commanded_bit_resolution = w(value)?,
                    "protocol" => match value {
                        ParameterValue::Protocol(protocol) => g.protocol = Some(protocol),
                        _ => return Err(wrong("a protocol")),
                    },
                    "range_of_motion" => g.range_of_motion = q(value)?,
                    "smoothing_kernel" => g.smoothing_kernel = t(value)?,
                    "smoothing_parameters" => g.smoothing_parameters = p(value)?,
                    "tuning_parameters" => g.tuning_parameters = t(value)?,
                    "tuning_type" => g.tuning_type = t(value)?,
                    _ => return Err(refuse(format!("`{other}` is not a parameter"))),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentences_of_the_two_notes_are_distinct_and_name_no_version() {
        // IP-12 phase 12B: an origin says one thing, so the two sentences differ; and the Model names no version, whichever file the note is about
        assert_ne!(SCAN_HEAD_OFFSET_SIGN_OPPOSITE, BIT_RESOLUTION_GLOBAL);
        for sentence in [SCAN_HEAD_OFFSET_SIGN_OPPOSITE, BIT_RESOLUTION_GLOBAL] {
            assert!(!sentence.is_empty());
            assert!(
                !sentence.contains("V16") && !sentence.contains("V17") && !sentence.contains("V18")
            );
        }
        // the sign note says that the relation is inferred and that the value is not changed
        assert!(
            SCAN_HEAD_OFFSET_SIGN_OPPOSITE.contains("inferred")
                && SCAN_HEAD_OFFSET_SIGN_OPPOSITE.contains("as written")
        );
    }

    #[test]
    fn a_snapshot_that_says_nothing_lists_no_value_and_a_value_is_listed_by_name() {
        let mut parameters = LaserParameters::new();
        assert!(parameters.values().is_empty());
        parameters.scanner.field_size_x = Some(Quantity::from_file(600.0, None));
        parameters.galvo_z.protocol = Some(GalvoProtocol::Sl2100);
        parameters.scanner_card.actual_position_timing_offset = Some(-8);
        let names: Vec<String> = parameters.values().into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [
                "galvo_z.protocol",
                "scanner.field_size_x",
                "scanner_card.actual_position_timing_offset"
            ]
        );
    }

    #[test]
    fn set_is_the_inverse_of_values_and_refuses_a_name_it_does_not_know_or_a_wrong_kind() {
        let mut parameters = LaserParameters::new();
        parameters
            .set(
                "light_source.wavelength",
                ParameterValue::Quantity(Quantity::from_file(1070.0, Some("nm"))),
            )
            .unwrap();
        parameters
            .set("galvo_y.tuning_type", ParameterValue::Text("PID".into()))
            .unwrap();
        parameters
            .set(
                "galvo_x.protocol",
                ParameterValue::Protocol(GalvoProtocol::Xy2100),
            )
            .unwrap();
        // what was set is what is listed
        let mut copy = LaserParameters::new();
        for (name, value) in parameters.values() {
            copy.set(&name, value).unwrap();
        }
        assert_eq!(copy, parameters);
        // a name that is not a parameter, a galvo that is not an axis, and a value of the wrong kind
        assert!(parameters
            .set("scanner.colour", ParameterValue::Flag(true))
            .is_err());
        assert!(parameters
            .set("galvo_w.tuning_type", ParameterValue::Text("x".into()))
            .is_err());
        assert!(parameters
            .set("scanner.rotation", ParameterValue::Plain(1.0))
            .is_err());
        assert!(parameters
            .set("galvo_x.protocol", ParameterValue::Text("SL2-100".into()))
            .is_err());
    }

    #[test]
    fn an_equation_is_found_by_the_name_of_a_constant_and_never_by_its_place() {
        let line = Equation::new(EquationKind::Linear)
            .with_constant("b", 50.5)
            .with_constant("a", 107.5);
        assert_eq!(line.constant("a"), Some(107.5));
        assert_eq!(line.constant("b"), Some(50.5));
        assert_eq!(line.constant("c0"), None);
        // the same constants in the other order are the same line
        let swapped = Equation::new(EquationKind::Linear)
            .with_constant("a", 107.5)
            .with_constant("b", 50.5);
        assert_eq!(swapped.constant("a"), line.constant("a"));
        assert_eq!(swapped.constant("b"), line.constant("b"));
    }

    #[test]
    fn a_protocol_or_an_algorithm_with_no_name_is_kept_as_the_file_spells_it() {
        assert_ne!(
            GalvoProtocol::Other("xy2-100".into()),
            GalvoProtocol::Xy2100
        );
        assert_ne!(EquationKind::Other("Linear".into()), EquationKind::Linear);
    }
}
