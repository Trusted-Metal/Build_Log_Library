//! The beam model of a laser, resolved from what the file stores and what the user sets (IP-27 phase 27H, decisions D27-6, D27-18, D27-21 and D27-23).
//!
//! * **Layer:** Processing. It works on Model values only (the parameter snapshot and the user's overrides), so it runs without HDF5.
//! * **Requirement:** BL-PR-03 (every value says where it came from), BL-PR-04 (the arithmetic is fixed once, documented and tested).
//!
//! # The shape: the same as the time shift (D27-23)
//!
//! 1. **Read.** [`RecordedBeam::from_parameters`] takes the beam values as the snapshot holds them: a quantity with its unit (or an unknown one), a number, or nothing. **The new waist name is read
//!    first and the legacy `*_Waist_D4Sigma` only when the new one is not there, even when the new one holds zero** (Vision's rule, a marked quirk). A laser with **no parameters at all** has no
//!    beam to read: [`resolve_beam_of`] says [`BeamUnavailable::NoTrain`].
//! 2. **Units.** [`BeamInput::from_recorded`] brings each quantity to the unit the resolver works in (a waist in micrometres, the other lengths in millimetres, angles in degrees) **with Vision's
//!    table and arithmetic** ([`crate::beam_units`]); a value with no usable unit takes the unit the caller assumes.
//! 3. **Resolve.** [`resolve_beam`] is **Vision's `resolve_beam`, over plain numbers**, algorithm version 1 with its quirks. It returns the beam with the **origin of each value** and structured
//!    **findings** (the circular beam, a derived Rayleigh length, a stored one that disagrees) where Vision returns note texts; **the adapter renders Vision's texts from them.**
//!
//! # What the file does not say: the caller's, in one object
//!
//! [`BeamAssumptions`] is explicit, immutable and hashable, and every field is required, so that each decision is visible at the call site; the library has no default for any of them. It holds the
//! **seven answers** that Vision gives today: the unit assumed for a waist (micrometres), for the other lengths (millimetres), for an angle (degrees) and for a wavelength (nanometres); **the
//! value of an absent offset, shift, threshold, angle or rotation (0)**; **the M² assumed when none is known** (1), for deriving a Rayleigh length; and **what one missing waist means** (a circular
//! beam from the other). A value that is filled in from them has the origin [`Origin::CallerSupplied`].
//!
//! # What stays Vision's, and what is carried as data
//!
//! * **The first-train fallback:** a laser with no parameter snapshot (a V17 laser with no train) uses the first train's beam in Vision. The library never picks a train: it says
//!   [`BeamUnavailable::NoTrain`] and the caller asks again with another laser's parameters.
//! * **The thermal-lensing flag** is carried as recorded ([`ResolvedBeam::thermal_test`], with the shift and the threshold as stored) and raises **no finding** (D27-18): Vision renders its note and
//!   its warning.
//! * **A wavelength or a rotation whose file states a unit** is read as a number in the caller's assumed unit, as Vision reads it (a marked quirk of version 1); the unit that the file states is in
//!   the [`RecordedBeam`], as data, for any consumer that wants a guard.
//!
//! # Version 1 keeps these behaviours of Vision's (marked `quirk` in its spec of the chunk; the owner decides at phase 27L)
//!
//! * A NaN override of the waist offset, the thermal shift, the axis angle or the rotation **counts as 0 and does not fall through** to the stored value, where a zero, negative or NaN override of a
//!   waist, a Rayleigh length or M² does fall through.
//! * The mismatch note for a stored Rayleigh length is raised **only when an M² is known**, though the derived length it is compared with assumes one.
//! * The first attribute name found wins even when it holds zero.

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::fmt;

use build_log_model::{LaserParameters, Origin, Quantity, SpotSizeOverride};

use crate::beam_units::{
    angle_unit_stated, convert_angle, convert_length, length_unit_stated, wavelength_in_nanometres,
    AngleUnit, LengthUnit,
};
use crate::calibration::Resolved;

/// The relative disagreement between a stored Rayleigh length and the one implied by the waist, M² and wavelength, above which it is a finding.
const RAYLEIGH_MISMATCH_TOLERANCE: f64 = 0.2;

/// One of the two axes of the beam's ellipse.
// closed: an ellipse has a major and a minor axis, and a finding names one of them
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BeamAxis {
    Major,
    Minor,
}

impl fmt::Display for BeamAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BeamAxis::Major => "major",
            BeamAxis::Minor => "minor",
        })
    }
}

/// What it means that one of the two waists is missing.
// closed: the caller either lets the other axis stand for both, or says there is no beam
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OneWaistMissing {
    /// A circular beam: the waist that there is stands for both (Vision's choice), and a [`BeamFinding::CircularBeamAssumed`] says so.
    AssumeCircular,
    /// No beam: [`BeamUnavailable::OneWaistMissing`].
    Unavailable,
}

/// **What the caller decides where the file is silent.** Every field is required; the library has no default for any of them. A value that is filled in from here has the origin
/// [`Origin::CallerSupplied`]. The object is immutable, compares and hashes by the bits of its numbers, so it can be part of a cache key.
// constructed-by: caller-options
#[derive(Debug, Clone, Copy)]
pub struct BeamAssumptions {
    /// The unit a waist is taken in when the file states none that the table knows. Vision: micrometres.
    pub waist_unit: LengthUnit,
    /// The unit the other lengths (Rayleigh length, waist offset, thermal shift and threshold) are taken in when the file states none. Vision: millimetres.
    pub length_unit: LengthUnit,
    /// The unit an angle is taken in when the file states none. Vision: degrees. **The scanner rotation is taken in it whatever the file states** (version 1).
    pub angle_unit: AngleUnit,
    /// The unit a wavelength is taken in: **whatever the file states** (version 1, Vision's quirk). Vision: nanometres.
    pub wavelength_unit: LengthUnit,
    /// The value of an offset, a shift, a threshold, an angle or a rotation that the file does not have. Vision: 0.
    pub absent_value: f64,
    /// The M² assumed, to derive a Rayleigh length, when neither the user nor the file gives a usable one. Vision: 1.
    pub m2_when_none: f64,
    /// What it means that one of the two waists is missing. Vision: a circular beam.
    pub one_waist_missing: OneWaistMissing,
}

impl BeamAssumptions {
    #[allow(clippy::type_complexity)]
    fn key(
        &self,
    ) -> (
        LengthUnit,
        LengthUnit,
        AngleUnit,
        LengthUnit,
        u64,
        u64,
        OneWaistMissing,
    ) {
        (
            self.waist_unit,
            self.length_unit,
            self.angle_unit,
            self.wavelength_unit,
            self.absent_value.to_bits(),
            self.m2_when_none.to_bits(),
            self.one_waist_missing,
        )
    }
}

impl PartialEq for BeamAssumptions {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for BeamAssumptions {}

impl std::hash::Hash for BeamAssumptions {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

// ---------------------------------------------------------------- what the file recorded

/// The beam values of one laser as its snapshot holds them: quantities with their units, numbers, and nothing for what the file does not have.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecordedBeam {
    waist_major: Option<Quantity>,
    waist_minor: Option<Quantity>,
    rayleigh_major: Option<Quantity>,
    rayleigh_minor: Option<Quantity>,
    m2_major: Option<f64>,
    m2_minor: Option<f64>,
    wavelength: Option<Quantity>,
    waist_offset: Option<Quantity>,
    thermal_shift: Option<Quantity>,
    thermal_threshold: Option<Quantity>,
    thermal_test_passed: Option<bool>,
    major_axis_angle: Option<Quantity>,
    scanner_rotation: Option<Quantity>,
    /// The origin that the reader recorded for a value that is there, by the name of the value in [`BeamInput`].
    origins: BTreeMap<&'static str, Origin>,
}

impl RecordedBeam {
    /// What the parameter snapshot of a laser holds for its beam. **The first name found wins for a waist:** the new name (`Beam_Waist_*`), and the legacy `*_Waist_D4Sigma` only when the new one is
    /// not there, **even when the new one holds zero**. The thermal shift is the file's `Thermal_Lensing_Focal_Plane_Shift`: **the alias `Thermal_Lensing_Focal_Shift`, which Vision reads first, is not
    /// typed by the library and is not read** (decision D27-23: left until a producer is known).
    pub fn from_parameters(parameters: &LaserParameters) -> Self {
        let optics = &parameters.optics;
        let mut origins = BTreeMap::new();
        let mut note = |field: &'static str, present: bool, name: &str| {
            if present {
                if let Some(origin) = parameters.origins.get(name) {
                    origins.insert(field, origin.clone());
                }
            }
        };
        let (waist_major, waist_major_name) = match &optics.waist_major {
            Some(q) => (Some(q.clone()), "optics.waist_major"),
            None => (
                optics.major_waist_d4sigma.clone(),
                "optics.major_waist_d4sigma",
            ),
        };
        let (waist_minor, waist_minor_name) = match &optics.waist_minor {
            Some(q) => (Some(q.clone()), "optics.waist_minor"),
            None => (
                optics.minor_waist_d4sigma.clone(),
                "optics.minor_waist_d4sigma",
            ),
        };
        note("waist_major_um", waist_major.is_some(), waist_major_name);
        note("waist_minor_um", waist_minor.is_some(), waist_minor_name);
        note(
            "rayleigh_major_mm",
            optics.rayleigh_length_major.is_some(),
            "optics.rayleigh_length_major",
        );
        note(
            "rayleigh_minor_mm",
            optics.rayleigh_length_minor.is_some(),
            "optics.rayleigh_length_minor",
        );
        note("m2_major", optics.m2_major.is_some(), "optics.m2_major");
        note("m2_minor", optics.m2_minor.is_some(), "optics.m2_minor");
        note(
            "wavelength_nm",
            parameters.light_source.wavelength.is_some(),
            "light_source.wavelength",
        );
        note(
            "waist_offset_z_mm",
            optics.waist_offset_z.is_some(),
            "optics.waist_offset_z",
        );
        note(
            "thermal_shift_mm",
            optics.thermal_lensing_focal_plane_shift.is_some(),
            "optics.thermal_lensing_focal_plane_shift",
        );
        note(
            "thermal_threshold_mm",
            optics.thermal_lensing_threshold.is_some(),
            "optics.thermal_lensing_threshold",
        );
        note(
            "major_axis_angle_deg",
            optics.major_axis_angle.is_some(),
            "optics.major_axis_angle",
        );
        note(
            "scanner_rotation_deg",
            parameters.scanner.rotation.is_some(),
            "scanner.rotation",
        );
        RecordedBeam {
            waist_major,
            waist_minor,
            rayleigh_major: optics.rayleigh_length_major.clone(),
            rayleigh_minor: optics.rayleigh_length_minor.clone(),
            m2_major: optics.m2_major,
            m2_minor: optics.m2_minor,
            wavelength: parameters.light_source.wavelength.clone(),
            waist_offset: optics.waist_offset_z.clone(),
            thermal_shift: optics.thermal_lensing_focal_plane_shift.clone(),
            thermal_threshold: optics.thermal_lensing_threshold.clone(),
            thermal_test_passed: optics.thermal_lensing_test_passed,
            major_axis_angle: optics.major_axis_angle.clone(),
            scanner_rotation: parameters.scanner.rotation.clone(),
            origins,
        }
    }

    /// The major waist **diameter** as recorded, with its unit.
    pub fn waist_major(&self) -> Option<&Quantity> {
        self.waist_major.as_ref()
    }

    /// The minor waist **diameter** as recorded, with its unit.
    pub fn waist_minor(&self) -> Option<&Quantity> {
        self.waist_minor.as_ref()
    }

    pub fn rayleigh_major(&self) -> Option<&Quantity> {
        self.rayleigh_major.as_ref()
    }

    pub fn rayleigh_minor(&self) -> Option<&Quantity> {
        self.rayleigh_minor.as_ref()
    }

    pub fn m2_major(&self) -> Option<f64> {
        self.m2_major
    }

    pub fn m2_minor(&self) -> Option<f64> {
        self.m2_minor
    }

    /// The wavelength as recorded, **with the unit the file states, if any** (version 1 does not apply it: see the module documentation).
    pub fn wavelength(&self) -> Option<&Quantity> {
        self.wavelength.as_ref()
    }

    pub fn waist_offset(&self) -> Option<&Quantity> {
        self.waist_offset.as_ref()
    }

    pub fn thermal_shift(&self) -> Option<&Quantity> {
        self.thermal_shift.as_ref()
    }

    pub fn thermal_threshold(&self) -> Option<&Quantity> {
        self.thermal_threshold.as_ref()
    }

    pub fn thermal_test_passed(&self) -> Option<bool> {
        self.thermal_test_passed
    }

    pub fn major_axis_angle(&self) -> Option<&Quantity> {
        self.major_axis_angle.as_ref()
    }

    /// The scanner rotation as recorded, **with the unit the file states, if any** (version 1 does not apply it).
    pub fn scanner_rotation(&self) -> Option<&Quantity> {
        self.scanner_rotation.as_ref()
    }
}

// ---------------------------------------------------------------- the numbers the resolver works on

/// The beam values as plain numbers in the units the resolver works in. **A waist, a Rayleigh length, an M² or a wavelength that is zero, negative or not finite is missing** (Vision's own
/// convention, because its `OpticalTrain` cannot say "absent"); [`BeamInput::from_recorded`] writes `0.0` for one that the file does not have. A hand-built input starts at zero everywhere
/// (`Default`).
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BeamInput {
    /// The waist **diameter** along the major axis, D4σ, in micrometres.
    pub waist_major_um: f64,
    /// The waist **diameter** along the minor axis, in micrometres.
    pub waist_minor_um: f64,
    /// The Rayleigh length along the major axis, in millimetres.
    pub rayleigh_major_mm: f64,
    /// The Rayleigh length along the minor axis, in millimetres.
    pub rayleigh_minor_mm: f64,
    /// The beam quality factor M² along the major axis.
    pub m2_major: f64,
    /// The beam quality factor M² along the minor axis.
    pub m2_minor: f64,
    /// The wavelength, in nanometres.
    pub wavelength_nm: f64,
    /// Where the waist is relative to the commanded focus, in millimetres.
    pub waist_offset_z_mm: f64,
    /// The thermal-lensing shift of the focal plane, in millimetres.
    pub thermal_shift_mm: f64,
    /// The thermal-lensing acceptance threshold, in millimetres (only used in a note).
    pub thermal_threshold_mm: f64,
    /// The result of the thermal-lensing test, when one was recorded.
    pub thermal_test_passed: Option<bool>,
    /// The orientation of the major axis in the scanner frame, in degrees.
    pub major_axis_angle_deg: f64,
    /// The rotation of the scanner, in degrees.
    pub scanner_rotation_deg: f64,
    /// Where a value did not simply come from the file, by its field name: the caller's answer, or what the reader recorded.
    origins: BTreeMap<&'static str, Origin>,
}

/// The unit-bearing length of a recorded value, or what stands in for it.
fn length_value(
    origins: &mut BTreeMap<&'static str, Origin>,
    field: &'static str,
    quantity: Option<&Quantity>,
    assumed: LengthUnit,
    target: LengthUnit,
    absent: Option<f64>,
) -> f64 {
    match quantity {
        Some(q) => {
            // the file states no unit that the table knows: the number is read in the caller's unit
            if !length_unit_stated(q) {
                origins.insert(field, Origin::CallerSupplied);
            }
            convert_length(q, assumed, target)
        }
        None => match absent {
            Some(value) => {
                origins.insert(field, Origin::CallerSupplied);
                value
            }
            None => 0.0,
        },
    }
}

impl BeamInput {
    /// The beam values that the file recorded, brought to the resolver's units with Vision's table and arithmetic, and **the caller's answers where the file is silent**
    /// (a value so filled in has the origin [`Origin::CallerSupplied`]). A waist, Rayleigh length, M² or wavelength that is absent is `0.0`: missing.
    pub fn from_recorded(recorded: &RecordedBeam, assumptions: &BeamAssumptions) -> Self {
        let a = assumptions;
        let mut origins = recorded.origins.clone();
        let waist_major_um = length_value(
            &mut origins,
            "waist_major_um",
            recorded.waist_major.as_ref(),
            a.waist_unit,
            LengthUnit::Micrometre,
            None,
        );
        let waist_minor_um = length_value(
            &mut origins,
            "waist_minor_um",
            recorded.waist_minor.as_ref(),
            a.waist_unit,
            LengthUnit::Micrometre,
            None,
        );
        let rayleigh_major_mm = length_value(
            &mut origins,
            "rayleigh_major_mm",
            recorded.rayleigh_major.as_ref(),
            a.length_unit,
            LengthUnit::Millimetre,
            None,
        );
        let rayleigh_minor_mm = length_value(
            &mut origins,
            "rayleigh_minor_mm",
            recorded.rayleigh_minor.as_ref(),
            a.length_unit,
            LengthUnit::Millimetre,
            None,
        );
        let waist_offset_z_mm = length_value(
            &mut origins,
            "waist_offset_z_mm",
            recorded.waist_offset.as_ref(),
            a.length_unit,
            LengthUnit::Millimetre,
            Some(a.absent_value),
        );
        let thermal_shift_mm = length_value(
            &mut origins,
            "thermal_shift_mm",
            recorded.thermal_shift.as_ref(),
            a.length_unit,
            LengthUnit::Millimetre,
            Some(a.absent_value),
        );
        let thermal_threshold_mm = length_value(
            &mut origins,
            "thermal_threshold_mm",
            recorded.thermal_threshold.as_ref(),
            a.length_unit,
            LengthUnit::Millimetre,
            Some(a.absent_value),
        );
        let major_axis_angle_deg = match recorded.major_axis_angle.as_ref() {
            Some(q) => {
                if !angle_unit_stated(q) {
                    origins.insert("major_axis_angle_deg", Origin::CallerSupplied);
                }
                convert_angle(q, a.angle_unit)
            }
            None => {
                origins.insert("major_axis_angle_deg", Origin::CallerSupplied);
                a.absent_value
            }
        };
        // the rotation and the wavelength are numbers in the caller's unit, whatever the file states (version 1)
        let scanner_rotation_deg = match recorded.scanner_rotation.as_ref() {
            Some(q) => q.value * a.angle_unit.to_degrees(),
            None => {
                origins.insert("scanner_rotation_deg", Origin::CallerSupplied);
                a.absent_value
            }
        };
        let wavelength_nm = recorded.wavelength.as_ref().map_or(0.0, |q| {
            wavelength_in_nanometres(q.value, a.wavelength_unit)
        });
        BeamInput {
            waist_major_um,
            waist_minor_um,
            rayleigh_major_mm,
            rayleigh_minor_mm,
            m2_major: recorded.m2_major.unwrap_or(0.0),
            m2_minor: recorded.m2_minor.unwrap_or(0.0),
            wavelength_nm,
            waist_offset_z_mm,
            thermal_shift_mm,
            thermal_threshold_mm,
            thermal_test_passed: recorded.thermal_test_passed,
            major_axis_angle_deg,
            scanner_rotation_deg,
            origins,
        }
    }

    /// Where the value of `field` (its name in this struct, such as `"waist_major_um"`) did not simply come from the file: the caller's answer ([`Origin::CallerSupplied`]), or what the reader
    /// recorded for it. `None` is the file's own value as recorded, and also a hand-built input's.
    pub fn origin_of(&self, field: &str) -> Option<&Origin> {
        self.origins.get(field)
    }
}

// ---------------------------------------------------------------- what comes out

/// Something the resolver did that a person looking at the beam would want to know. Vision turns each into the note it shows.
// open-domain: the observations grow with the resolver; a consumer renders the ones it knows
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BeamFinding {
    /// One waist was missing and the other stands for both: a circular beam. `missing` is the axis that had none.
    CircularBeamAssumed { missing: BeamAxis },
    /// A Rayleigh length that was not given was derived as `π·w0² / (M²·λ)`. `m2_assumed` is the M² that was assumed because none was known, if one was.
    RayleighDerived {
        axis: BeamAxis,
        derived_mm: f64,
        m2_assumed: Option<f64>,
    },
    /// A stored Rayleigh length differs from the one implied by the waist, M² and wavelength by more than 20%; the stored one is kept. **Only when an M² is known** (version 1).
    RayleighMismatch {
        axis: BeamAxis,
        stored_mm: f64,
        derived_mm: f64,
    },
}

/// Why a laser has no beam.
// open-domain: the reasons grow with the resolver; a consumer matches on the ones it knows
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BeamUnavailable {
    /// The laser has **no parameter snapshot at all**: no optical train (a V17 laser whose train is missing). Vision then uses the first train's beam; the library never picks a train, so the caller
    /// asks again with another laser's parameters. **A V16 laser is not this case:** its snapshot is there (from its own calibration data) and holds no waist, so it is [`BeamUnavailable::NoWaist`].
    NoTrain,
    /// Neither a stored nor an overridden waist.
    NoWaist,
    /// One waist is missing and [`BeamAssumptions::one_waist_missing`] says that means no beam.
    OneWaistMissing { missing: BeamAxis },
    /// A Rayleigh length is neither stored, overridden nor derivable (no wavelength).
    NoRayleigh { axis: BeamAxis },
}

impl fmt::Display for BeamUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BeamUnavailable::NoTrain => f.write_str("the laser has no optical-train parameters"),
            BeamUnavailable::NoWaist => f.write_str("no beam waist is stored or set"),
            BeamUnavailable::OneWaistMissing { missing } => {
                write!(f, "the {missing} beam waist is not stored or set")
            }
            BeamUnavailable::NoRayleigh { axis } => write!(
                f,
                "the {axis} Rayleigh length is not stored or set and cannot be derived (no wavelength)"
            ),
        }
    }
}

impl std::error::Error for BeamUnavailable {}

/// The thermal-lensing test **as recorded**, with the shift and the threshold as stored (not as overridden): what Vision's note and warning say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalTest {
    /// The result of the test: `None` when none was recorded.
    pub passed: Option<bool>,
    /// The stored focal-plane shift, in millimetres.
    pub shift_mm: f64,
    /// The stored acceptance threshold, in millimetres.
    pub threshold_mm: f64,
}

/// The beam of a laser: the Gaussian-beam parameters with where each came from, and what was found on the way.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedBeam {
    waist_radius_major_um: Resolved<f64>,
    waist_radius_minor_um: Resolved<f64>,
    rayleigh_major_mm: Resolved<f64>,
    rayleigh_minor_mm: Resolved<f64>,
    waist_offset_mm: Resolved<f64>,
    thermal_shift_mm: Resolved<f64>,
    focus_shift_mm: f64,
    major_axis_angle: Resolved<f64>,
    scanner_rotation: Resolved<f64>,
    axis_angle_deg: f64,
    thermal_test: ThermalTest,
    findings: Vec<BeamFinding>,
}

impl ResolvedBeam {
    /// The waist radius along the major axis, in micrometres: the override, the stored value, or (for a circular beam) the other axis' with the origin [`Origin::Inferred`].
    pub fn waist_radius_major_um(&self) -> &Resolved<f64> {
        &self.waist_radius_major_um
    }

    pub fn waist_radius_minor_um(&self) -> &Resolved<f64> {
        &self.waist_radius_minor_um
    }

    /// The Rayleigh length along the major axis, in millimetres: the override, the stored value, or the derived one, which has the origin [`Origin::Inferred`].
    pub fn rayleigh_major_mm(&self) -> &Resolved<f64> {
        &self.rayleigh_major_mm
    }

    pub fn rayleigh_minor_mm(&self) -> &Resolved<f64> {
        &self.rayleigh_minor_mm
    }

    /// The waist offset that was used, in millimetres (a value that is not finite is 0).
    pub fn waist_offset_mm(&self) -> &Resolved<f64> {
        &self.waist_offset_mm
    }

    /// The thermal shift that was used, in millimetres (a value that is not finite is 0).
    pub fn thermal_shift_mm(&self) -> &Resolved<f64> {
        &self.thermal_shift_mm
    }

    /// The distance between the commanded focus and the beam waist, in millimetres: the waist offset plus the thermal shift.
    pub fn focus_shift_mm(&self) -> f64 {
        self.focus_shift_mm
    }

    /// The major-axis angle that was used, in degrees, before the scanner rotation is added and the sum is folded.
    pub fn major_axis_angle_deg(&self) -> &Resolved<f64> {
        &self.major_axis_angle
    }

    /// The scanner rotation that was used, in degrees.
    pub fn scanner_rotation_deg(&self) -> &Resolved<f64> {
        &self.scanner_rotation
    }

    /// The orientation of the major axis in the build frame, in `[0, 180)` degrees: the axis angle plus the rotation, folded (an ellipse is unchanged by a half turn).
    pub fn axis_angle_deg(&self) -> f64 {
        self.axis_angle_deg
    }

    /// The thermal-lensing test as recorded.
    pub fn thermal_test(&self) -> ThermalTest {
        self.thermal_test
    }

    /// What was found, in the order it was made: the waist, the major axis' Rayleigh length, the minor axis'.
    pub fn findings(&self) -> &[BeamFinding] {
        &self.findings
    }
}

// ---------------------------------------------------------------- the resolver

fn positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

/// A number with where it came from.
type Valued = (f64, Option<Origin>);

/// The override if it is positive and finite, else the stored value if it is.
fn pick_positive(user: Option<f64>, stored: f64, stored_origin: Option<&Origin>) -> Option<Valued> {
    user.and_then(positive)
        .map(|v| (v, Some(Origin::Overridden)))
        .or_else(|| positive(stored).map(|v| (v, stored_origin.cloned())))
}

/// The value that an offset, a shift, an angle or a rotation counts as: the override **if there is one, whatever it is**, else the stored value; a value that is not finite is 0.
fn used(user: Option<f64>, stored: f64, stored_origin: Option<&Origin>) -> (f64, Option<Origin>) {
    match user {
        Some(v) => (finite_or_zero(v), Some(Origin::Overridden)),
        None => (finite_or_zero(stored), stored_origin.cloned()),
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_rayleigh(
    axis: BeamAxis,
    override_mm: Option<f64>,
    stored_mm: f64,
    stored_origin: Option<&Origin>,
    m2_override: Option<f64>,
    m2_stored: f64,
    waist_radius_um: f64,
    wavelength_nm: f64,
    m2_when_none: f64,
    findings: &mut Vec<BeamFinding>,
) -> Result<Valued, BeamUnavailable> {
    let m2 = m2_override
        .and_then(positive)
        .or_else(|| positive(m2_stored));
    let derived = positive(wavelength_nm).map(|lambda_nm| {
        let w0_mm = waist_radius_um * 1e-3;
        PI * w0_mm * w0_mm / (m2.unwrap_or(m2_when_none) * lambda_nm * 1e-6)
    });

    if let Some(v) = override_mm.and_then(positive) {
        return Ok((v, Some(Origin::Overridden)));
    }
    if let Some(v) = positive(stored_mm) {
        if let (Some(d), Some(_)) = (derived, m2) {
            if ((v - d) / d).abs() > RAYLEIGH_MISMATCH_TOLERANCE {
                findings.push(BeamFinding::RayleighMismatch {
                    axis,
                    stored_mm: v,
                    derived_mm: d,
                });
            }
        }
        return Ok((v, stored_origin.cloned()));
    }
    match derived {
        Some(d) => {
            findings.push(BeamFinding::RayleighDerived {
                axis,
                derived_mm: d,
                m2_assumed: m2.is_none().then_some(m2_when_none),
            });
            Ok((d, Some(Origin::Inferred)))
        }
        None => Err(BeamUnavailable::NoRayleigh { axis }),
    }
}

/// The beam of a laser: the stored values and the user's overrides, resolved. **Vision's `resolve_beam`, to the last bit, over plain numbers** (see the module documentation for what it
/// keeps), with the origin of each value and structured findings in place of note texts. What the file does not say is answered by `assumptions`; nothing is defaulted here.
pub fn resolve_beam(
    input: &BeamInput,
    overrides: &SpotSizeOverride,
    assumptions: &BeamAssumptions,
) -> Result<ResolvedBeam, BeamUnavailable> {
    let mut findings = Vec::new();

    let major = pick_positive(
        overrides.waist_major_um,
        input.waist_major_um,
        input.origin_of("waist_major_um"),
    );
    let minor = pick_positive(
        overrides.waist_minor_um,
        input.waist_minor_um,
        input.origin_of("waist_minor_um"),
    );
    let circular = |from: Valued| (from.0, Some(Origin::Inferred));
    let (major_d, minor_d) = match (major, minor) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => match assumptions.one_waist_missing {
            OneWaistMissing::AssumeCircular => {
                findings.push(BeamFinding::CircularBeamAssumed {
                    missing: BeamAxis::Minor,
                });
                (a.clone(), circular(a))
            }
            OneWaistMissing::Unavailable => {
                return Err(BeamUnavailable::OneWaistMissing {
                    missing: BeamAxis::Minor,
                })
            }
        },
        (None, Some(b)) => match assumptions.one_waist_missing {
            OneWaistMissing::AssumeCircular => {
                findings.push(BeamFinding::CircularBeamAssumed {
                    missing: BeamAxis::Major,
                });
                (circular(b.clone()), b)
            }
            OneWaistMissing::Unavailable => {
                return Err(BeamUnavailable::OneWaistMissing {
                    missing: BeamAxis::Major,
                })
            }
        },
        (None, None) => return Err(BeamUnavailable::NoWaist),
    };
    let w_major = major_d.0 / 2.0;
    let w_minor = minor_d.0 / 2.0;

    let zr_major = resolve_rayleigh(
        BeamAxis::Major,
        overrides.rayleigh_major_mm,
        input.rayleigh_major_mm,
        input.origin_of("rayleigh_major_mm"),
        overrides.m2_major,
        input.m2_major,
        w_major,
        input.wavelength_nm,
        assumptions.m2_when_none,
        &mut findings,
    )?;
    let zr_minor = resolve_rayleigh(
        BeamAxis::Minor,
        overrides.rayleigh_minor_mm,
        input.rayleigh_minor_mm,
        input.origin_of("rayleigh_minor_mm"),
        overrides.m2_minor,
        input.m2_minor,
        w_minor,
        input.wavelength_nm,
        assumptions.m2_when_none,
        &mut findings,
    )?;

    let offset = used(
        overrides.waist_offset_z_mm,
        input.waist_offset_z_mm,
        input.origin_of("waist_offset_z_mm"),
    );
    let shift = used(
        overrides.thermal_shift_mm,
        input.thermal_shift_mm,
        input.origin_of("thermal_shift_mm"),
    );
    let focus_shift_mm = offset.0 + shift.0;

    // the scanner rotation turns the whole field, the beam ellipse included; an ellipse is unchanged by a half turn, so the sum is folded into [0, 180)
    let angle = used(
        overrides.major_axis_angle_deg,
        input.major_axis_angle_deg,
        input.origin_of("major_axis_angle_deg"),
    );
    let rotation = used(
        overrides.scanner_rotation_deg,
        input.scanner_rotation_deg,
        input.origin_of("scanner_rotation_deg"),
    );
    let axis_angle_deg = (angle.0 + rotation.0).rem_euclid(180.0);

    Ok(ResolvedBeam {
        waist_radius_major_um: Resolved::new(w_major, major_d.1),
        waist_radius_minor_um: Resolved::new(w_minor, minor_d.1),
        rayleigh_major_mm: Resolved::new(zr_major.0, zr_major.1),
        rayleigh_minor_mm: Resolved::new(zr_minor.0, zr_minor.1),
        waist_offset_mm: Resolved::new(offset.0, offset.1),
        thermal_shift_mm: Resolved::new(shift.0, shift.1),
        focus_shift_mm,
        major_axis_angle: Resolved::new(angle.0, angle.1),
        scanner_rotation: Resolved::new(rotation.0, rotation.1),
        axis_angle_deg,
        thermal_test: ThermalTest {
            passed: input.thermal_test_passed,
            shift_mm: input.thermal_shift_mm,
            threshold_mm: input.thermal_threshold_mm,
        },
        findings,
    })
}

/// The beam of a laser from its parameter snapshot: [`RecordedBeam::from_parameters`], [`BeamInput::from_recorded`] and [`resolve_beam`] in one call. **A laser with no parameters has no
/// beam** ([`BeamUnavailable::NoTrain`]): the library never picks another laser's train for it.
pub fn resolve_beam_of(
    parameters: Option<&LaserParameters>,
    overrides: &SpotSizeOverride,
    assumptions: &BeamAssumptions,
) -> Result<ResolvedBeam, BeamUnavailable> {
    let parameters = parameters.ok_or(BeamUnavailable::NoTrain)?;
    let input = BeamInput::from_recorded(&RecordedBeam::from_parameters(parameters), assumptions);
    resolve_beam(&input, overrides, assumptions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vision() -> BeamAssumptions {
        BeamAssumptions {
            waist_unit: LengthUnit::Micrometre,
            length_unit: LengthUnit::Millimetre,
            angle_unit: AngleUnit::Degree,
            wavelength_unit: LengthUnit::Nanometre,
            absent_value: 0.0,
            m2_when_none: 1.0,
            one_waist_missing: OneWaistMissing::AssumeCircular,
        }
    }

    /// The sample build's train: a 66 x 68 um waist, a 3 mm Rayleigh length, M2 of 1 and 1070 nm.
    fn sample() -> BeamInput {
        BeamInput {
            waist_major_um: 66.0,
            waist_minor_um: 68.0,
            rayleigh_major_mm: 3.0,
            rayleigh_minor_mm: 3.0,
            m2_major: 1.0,
            m2_minor: 1.0,
            wavelength_nm: 1070.0,
            ..BeamInput::default()
        }
    }

    fn none() -> SpotSizeOverride {
        SpotSizeOverride::new()
    }

    fn q(value: f64, unit: Option<&str>) -> Quantity {
        Quantity::from_file(value, unit)
    }

    fn snapshot() -> LaserParameters {
        let mut p = LaserParameters::new();
        p.optics.waist_major = Some(q(66.7297, Some("\u{3bc}m")));
        p.optics.waist_minor = Some(q(68.1252, Some("\u{3bc}m")));
        p.optics.rayleigh_length_major = Some(q(3.13428, Some("mm")));
        p.optics.rayleigh_length_minor = Some(q(3.14442, Some("mm")));
        p.optics.m2_major = Some(1.04282);
        p.optics.m2_minor = Some(1.08339);
        p.optics.waist_offset_z = Some(q(0.457069, Some("mm")));
        p.optics.thermal_lensing_focal_plane_shift = Some(q(1.17839, Some("mm")));
        p.optics.thermal_lensing_threshold = Some(q(0.784837, Some("mm")));
        p.optics.thermal_lensing_test_passed = Some(false);
        p.optics.major_axis_angle = Some(q(0.0, Some("degrees")));
        p.light_source.wavelength = Some(q(1070.0, Some("nm")));
        p.scanner.rotation = Some(q(0.0, None));
        p
    }

    #[test]
    fn the_stored_values_resolve_to_the_radii_and_nothing_is_found() {
        let beam = resolve_beam(&sample(), &none(), &vision()).unwrap();
        assert_eq!(beam.waist_radius_major_um().value(), 33.0);
        assert_eq!(beam.waist_radius_minor_um().value(), 34.0);
        assert_eq!(beam.rayleigh_major_mm().value(), 3.0);
        assert_eq!(beam.focus_shift_mm(), 0.0);
        assert_eq!(beam.axis_angle_deg(), 0.0);
        assert!(beam.findings().is_empty());
        // the file's own values have no origin
        assert_eq!(beam.waist_radius_major_um().origin(), None);
        assert_eq!(beam.rayleigh_minor_mm().origin(), None);
    }

    #[test]
    fn a_missing_waist_is_a_circular_beam_when_the_caller_says_so_and_no_beam_when_it_does_not() {
        let mut input = sample();
        input.waist_minor_um = 0.0;
        let beam = resolve_beam(&input, &none(), &vision()).unwrap();
        assert_eq!(beam.waist_radius_minor_um().value(), 33.0);
        assert_eq!(
            beam.waist_radius_minor_um().origin(),
            Some(&Origin::Inferred)
        );
        assert_eq!(beam.waist_radius_major_um().origin(), None);
        assert_eq!(
            beam.findings(),
            [BeamFinding::CircularBeamAssumed {
                missing: BeamAxis::Minor
            }]
        );
        let mut strict = vision();
        strict.one_waist_missing = OneWaistMissing::Unavailable;
        assert_eq!(
            resolve_beam(&input, &none(), &strict).unwrap_err(),
            BeamUnavailable::OneWaistMissing {
                missing: BeamAxis::Minor
            }
        );
        // neither waist: no beam, whatever the caller says
        input.waist_major_um = f64::NAN;
        for assumptions in [vision(), strict] {
            assert_eq!(
                resolve_beam(&input, &none(), &assumptions).unwrap_err(),
                BeamUnavailable::NoWaist
            );
        }
    }

    #[test]
    fn an_override_of_a_waist_falls_through_when_it_is_not_positive_and_finite_and_is_the_users_when_it_is(
    ) {
        let input = sample();
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let beam = resolve_beam(&input, &none().with_waist_major_um(bad), &vision()).unwrap();
            assert_eq!(beam.waist_radius_major_um().value(), 33.0, "{bad}");
            assert_eq!(beam.waist_radius_major_um().origin(), None);
        }
        let beam = resolve_beam(&input, &none().with_waist_major_um(60.0), &vision()).unwrap();
        assert_eq!(beam.waist_radius_major_um().value(), 30.0);
        assert_eq!(
            beam.waist_radius_major_um().origin(),
            Some(&Origin::Overridden)
        );
    }

    #[test]
    fn a_rayleigh_length_that_is_missing_is_derived_and_says_whether_the_m2_was_assumed() {
        let mut input = sample();
        input.rayleigh_major_mm = 0.0;
        input.rayleigh_minor_mm = 0.0;
        input.m2_major = 1.1;
        input.m2_minor = 0.0;
        let beam = resolve_beam(&input, &none(), &vision()).unwrap();
        let major = PI * 0.033 * 0.033 / (1.1 * 1070.0 * 1e-6);
        assert_eq!(beam.rayleigh_major_mm().value(), major);
        assert_eq!(beam.rayleigh_major_mm().origin(), Some(&Origin::Inferred));
        let [first, second] = beam.findings() else {
            panic!("{:?}", beam.findings())
        };
        assert_eq!(
            first,
            &BeamFinding::RayleighDerived {
                axis: BeamAxis::Major,
                derived_mm: major,
                m2_assumed: None
            }
        );
        // the minor axis has no M2: the caller's is assumed, and the finding says so
        assert!(matches!(
            second,
            BeamFinding::RayleighDerived {
                axis: BeamAxis::Minor,
                m2_assumed: Some(m2),
                ..
            } if *m2 == 1.0
        ));
        // a caller that assumes another M2 gets another length, and the finding says which
        let mut other = vision();
        other.m2_when_none = 2.0;
        let beam = resolve_beam(&input, &none(), &other).unwrap();
        assert!(matches!(
            beam.findings()[1],
            BeamFinding::RayleighDerived {
                m2_assumed: Some(m2),
                ..
            } if m2 == 2.0
        ));
    }

    #[test]
    fn a_rayleigh_length_that_cannot_be_derived_without_a_wavelength_names_its_axis_and_the_major_comes_first(
    ) {
        let mut input = sample();
        input.rayleigh_minor_mm = 0.0;
        input.wavelength_nm = 0.0;
        assert_eq!(
            resolve_beam(&input, &none(), &vision()).unwrap_err(),
            BeamUnavailable::NoRayleigh {
                axis: BeamAxis::Minor
            }
        );
        input.rayleigh_major_mm = 0.0;
        assert_eq!(
            resolve_beam(&input, &none(), &vision()).unwrap_err(),
            BeamUnavailable::NoRayleigh {
                axis: BeamAxis::Major
            }
        );
        assert!(BeamUnavailable::NoRayleigh {
            axis: BeamAxis::Minor
        }
        .to_string()
        .contains("minor Rayleigh length"));
    }

    #[test]
    fn a_stored_rayleigh_length_far_from_the_derived_one_is_kept_and_noted_only_when_an_m2_is_known(
    ) {
        let mut input = sample();
        input.rayleigh_major_mm = 5.0;
        let beam = resolve_beam(&input, &none(), &vision()).unwrap();
        assert_eq!(beam.rayleigh_major_mm().value(), 5.0);
        assert!(matches!(
            beam.findings(),
            [BeamFinding::RayleighMismatch { axis: BeamAxis::Major, stored_mm, .. }] if *stored_mm == 5.0
        ));
        // no M2 known: the same disagreement is not noted (version 1)
        input.m2_major = 0.0;
        let beam = resolve_beam(&input, &none(), &vision()).unwrap();
        assert!(beam.findings().is_empty());
        // within 20%: not noted
        input.m2_major = 1.0;
        input.rayleigh_major_mm = 3.3;
        assert!(resolve_beam(&input, &none(), &vision())
            .unwrap()
            .findings()
            .is_empty());
    }

    #[test]
    fn the_focus_shift_is_the_offset_plus_the_shift_and_a_nan_override_counts_as_zero_without_falling_through(
    ) {
        let mut input = sample();
        input.waist_offset_z_mm = 2.0;
        input.thermal_shift_mm = 1.0;
        let beam = resolve_beam(&input, &none(), &vision()).unwrap();
        assert_eq!(beam.focus_shift_mm(), 3.0);
        // an override of zero is used (it does not fall through), and so is a negative one
        let zero = none().with_waist_offset_z_mm(0.0);
        assert_eq!(
            resolve_beam(&input, &zero, &vision())
                .unwrap()
                .focus_shift_mm(),
            1.0
        );
        let negative = none().with_waist_offset_z_mm(-3.0);
        assert_eq!(
            resolve_beam(&input, &negative, &vision())
                .unwrap()
                .focus_shift_mm(),
            -2.0
        );
        // a NaN override counts as 0 and the stored 2 mm is lost
        let nan = none().with_waist_offset_z_mm(f64::NAN);
        let beam = resolve_beam(&input, &nan, &vision()).unwrap();
        assert_eq!(beam.focus_shift_mm(), 1.0);
        assert_eq!(beam.waist_offset_mm().value(), 0.0);
        assert_eq!(beam.waist_offset_mm().origin(), Some(&Origin::Overridden));
        // a stored value that is not finite counts as 0
        input.waist_offset_z_mm = f64::NAN;
        assert_eq!(
            resolve_beam(&input, &none(), &vision())
                .unwrap()
                .focus_shift_mm(),
            1.0
        );
    }

    #[test]
    fn the_angle_is_the_axis_angle_plus_the_rotation_folded_into_a_half_turn() {
        let mut input = sample();
        for (angle, rotation, folded) in [
            (30.0, 45.0, 75.0),
            (90.0, 90.0, 0.0),
            (-30.0, 0.0, 150.0),
            (700.0, 25.0, 5.0),
            (0.0, 180.0, 0.0),
        ] {
            input.major_axis_angle_deg = angle;
            input.scanner_rotation_deg = rotation;
            let beam = resolve_beam(&input, &none(), &vision()).unwrap();
            assert_eq!(beam.axis_angle_deg(), folded, "{angle} + {rotation}");
        }
        // the rotation override replaces the file's rotation and is not added to it
        input.major_axis_angle_deg = 30.0;
        input.scanner_rotation_deg = 45.0;
        let beam =
            resolve_beam(&input, &none().with_scanner_rotation_deg(10.0), &vision()).unwrap();
        assert_eq!(beam.axis_angle_deg(), 40.0);
        assert_eq!(
            beam.scanner_rotation_deg().origin(),
            Some(&Origin::Overridden)
        );
    }

    #[test]
    fn the_thermal_test_is_carried_as_recorded_with_the_stored_shift_and_raises_no_finding() {
        let mut input = sample();
        input.thermal_test_passed = Some(false);
        input.thermal_shift_mm = 1.178;
        input.thermal_threshold_mm = 0.785;
        let beam = resolve_beam(&input, &none().with_thermal_shift_mm(0.1), &vision()).unwrap();
        // the shift in use is the override's, the thermal test says the stored one
        assert_eq!(beam.focus_shift_mm(), 0.1);
        assert_eq!(
            beam.thermal_test(),
            ThermalTest {
                passed: Some(false),
                shift_mm: 1.178,
                threshold_mm: 0.785
            }
        );
        assert!(beam.findings().is_empty());
    }

    #[test]
    fn a_laser_with_no_parameters_has_no_beam_and_the_library_picks_no_other_train() {
        assert_eq!(
            resolve_beam_of(None, &none(), &vision()).unwrap_err(),
            BeamUnavailable::NoTrain
        );
        // parameters with no beam in them: no waist, not no train
        let empty = LaserParameters::new();
        assert_eq!(
            resolve_beam_of(Some(&empty), &none(), &vision()).unwrap_err(),
            BeamUnavailable::NoWaist
        );
    }

    #[test]
    fn the_snapshot_is_read_with_the_new_waist_name_first_even_when_it_is_zero() {
        let mut p = LaserParameters::new();
        p.optics.major_waist_d4sigma = Some(q(68.0, None));
        let recorded = RecordedBeam::from_parameters(&p);
        assert_eq!(recorded.waist_major().map(|w| w.value), Some(68.0));
        // the new name is there with zero: it wins, and the legacy 68 is not read
        p.optics.waist_major = Some(q(0.0, None));
        let recorded = RecordedBeam::from_parameters(&p);
        assert_eq!(recorded.waist_major().map(|w| w.value), Some(0.0));
        let input = BeamInput::from_recorded(&recorded, &vision());
        assert_eq!(input.waist_major_um, 0.0);
        // the alias of the thermal shift is not typed, so it is not read: only the plane shift is
        assert!(recorded.thermal_shift().is_none());
    }

    #[test]
    fn a_snapshot_becomes_the_numbers_vision_reads_with_the_origins_of_what_it_filled_in() {
        let mut p = snapshot();
        p.optics.thermal_lensing_threshold = None;
        p.optics.waist_offset_z = None;
        p.scanner.rotation = None;
        let input = BeamInput::from_recorded(&RecordedBeam::from_parameters(&p), &vision());
        assert_eq!(input.waist_major_um, 66.7297 * 1e-3 / 1e-3);
        assert_eq!(input.rayleigh_major_mm, 3.13428 * 1.0 / 1.0);
        assert_eq!(input.wavelength_nm, 1070.0);
        assert_eq!(input.thermal_test_passed, Some(false));
        // what the file does not have is the caller's value, with that origin
        assert_eq!(input.thermal_threshold_mm, 0.0);
        for field in [
            "thermal_threshold_mm",
            "waist_offset_z_mm",
            "scanner_rotation_deg",
        ] {
            assert_eq!(
                input.origin_of(field),
                Some(&Origin::CallerSupplied),
                "{field}"
            );
        }
        // what it does have, with a unit the table knows, has none
        for field in [
            "waist_major_um",
            "rayleigh_major_mm",
            "thermal_shift_mm",
            "major_axis_angle_deg",
            "wavelength_nm",
        ] {
            assert_eq!(input.origin_of(field), None, "{field}");
        }
        // an absent waist, Rayleigh length, M2 or wavelength is zero (missing), and is not the caller's answer
        let bare = BeamInput::from_recorded(
            &RecordedBeam::from_parameters(&LaserParameters::new()),
            &vision(),
        );
        assert_eq!(
            (
                bare.waist_major_um,
                bare.rayleigh_minor_mm,
                bare.m2_major,
                bare.wavelength_nm
            ),
            (0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!(bare.origin_of("waist_major_um"), None);
        assert_eq!(bare.thermal_test_passed, None);
    }

    #[test]
    fn a_value_with_no_unit_is_read_in_the_callers_unit_and_says_so() {
        let mut p = LaserParameters::new();
        p.optics.major_waist_d4sigma = Some(q(68.0, None));
        p.optics.rayleigh_length_major = Some(q(3.0, Some("furlongs")));
        p.optics.major_axis_angle = Some(q(1.0, None));
        let recorded = RecordedBeam::from_parameters(&p);
        let micrometres = BeamInput::from_recorded(&recorded, &vision());
        assert_eq!(micrometres.waist_major_um, 68.0 * 1e-3 / 1e-3);
        assert_eq!(
            micrometres.origin_of("waist_major_um"),
            Some(&Origin::CallerSupplied)
        );
        assert_eq!(
            micrometres.origin_of("rayleigh_major_mm"),
            Some(&Origin::CallerSupplied)
        );
        assert_eq!(
            micrometres.origin_of("major_axis_angle_deg"),
            Some(&Origin::CallerSupplied)
        );
        // another caller reads the same numbers in other units
        let mut other = vision();
        other.waist_unit = LengthUnit::Millimetre;
        other.length_unit = LengthUnit::Centimetre;
        other.angle_unit = AngleUnit::Radian;
        let converted = BeamInput::from_recorded(&recorded, &other);
        assert_eq!(converted.waist_major_um, 68.0 * 1.0 / 1e-3);
        assert_eq!(converted.rayleigh_major_mm, 3.0 * 10.0 / 1.0);
        assert_eq!(converted.major_axis_angle_deg, 1.0 * (180.0 / PI));
    }

    #[test]
    fn a_wavelength_and_a_rotation_are_numbers_in_the_callers_unit_whatever_the_file_states() {
        let mut p = LaserParameters::new();
        p.light_source.wavelength = Some(q(1.07, Some("um")));
        p.scanner.rotation = Some(q(1.0, Some("rad")));
        let recorded = RecordedBeam::from_parameters(&p);
        let input = BeamInput::from_recorded(&recorded, &vision());
        // Vision's quirk: 1.07 is read as nanometres, and 1 as degrees, though the file says otherwise
        assert_eq!(input.wavelength_nm, 1.07);
        assert_eq!(input.scanner_rotation_deg, 1.0);
        // the unit the file states is carried as data
        assert_eq!(
            recorded
                .wavelength()
                .and_then(|w| w.unit.spelling.as_deref()),
            Some("um")
        );
        assert_eq!(
            recorded
                .scanner_rotation()
                .and_then(|w| w.unit.spelling.as_deref()),
            Some("rad")
        );
    }

    #[test]
    fn the_whole_snapshot_resolves_as_the_mock_file_does() {
        let beam = resolve_beam_of(Some(&snapshot()), &none(), &vision()).unwrap();
        assert_eq!(
            beam.waist_radius_major_um().value(),
            66.7297 * 1e-3 / 1e-3 / 2.0
        );
        assert_eq!(
            beam.focus_shift_mm(),
            0.457069 * 1.0 / 1.0 + 1.17839 * 1.0 / 1.0
        );
        assert!(beam.findings().is_empty());
        assert_eq!(beam.thermal_test().passed, Some(false));
    }

    #[test]
    fn the_assumptions_compare_and_hash_by_their_bits() {
        use std::collections::HashSet;
        let a = vision();
        let mut b = vision();
        assert_eq!(a, b);
        b.absent_value = -0.0;
        assert_ne!(a, b, "0.0 and -0.0 are different answers");
        let mut c = vision();
        c.m2_when_none = f64::NAN;
        assert_eq!(c, c);
        let mut d = vision();
        d.one_waist_missing = OneWaistMissing::Unavailable;
        let set: HashSet<BeamAssumptions> = [a, b, c, d, a].into_iter().collect();
        assert_eq!(set.len(), 4);
    }
}
