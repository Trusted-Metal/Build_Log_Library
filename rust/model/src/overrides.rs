//! The user's settings that a request carries: the timing offsets, the Z calibration and the spot-size beam (IP-27 phase 27D).
//!
//! * **Layer:** the Model: plain value objects, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** caller-options (`TimingOverride`, `ZCalibrationOverride`, `SpotSizeOverride`).
//! * **Requirement:** BL-PR-03 (parameters are immutable, hashable value objects scoped to the request; the library keeps no hidden global overrides).
//!
//! # What an override is
//!
//! A value that the caller sets **replaces the file's value for this request**, and a value that it does not set leaves the file's (and then the library's resolution of it) as it is.
//! Every field is therefore an `Option`: `None` is "not overridden", which is not a zero. **These objects carry what Vision's machine settings carry today, and no more** (decision
//! D27-18): there is **no Z-offset override**, because Vision has none; one would be an added field later, not a change to these.
//!
//! * [`TimingOverride`]: the offset of the commanded and of the actual positions, **in sample-period steps** (a step is one sample period, not a number of seconds).
//! * [`ZCalibrationOverride`]: the field size of Z (millimetres) and the bit resolution of the commanded and of the actual Z.
//! * [`SpotSizeOverride`]: the beam, in the units that Vision's settings use (the waist **diameter** in micrometres, the Rayleigh length and the lengths in millimetres, the angles in degrees).
//!
//! # What this module claims
//!
//! * **Immutable.** A value is built once with the `with_*` methods and read through its fields; nothing changes it afterwards.
//! * **Hashable, with equal meaning equal bits.** Two overrides are equal when every value has the same bit pattern, so a `NaN` equals the same `NaN` and `0.0` is not `-0.0`: a request
//!   that is compared or used as a key never finds two settings equal that would give different results. (A `NaN` override is representable, because Vision accepts one: it counts as 0 there.)
//! * **Listable by name**, as the parameter snapshot is: [`TimingOverride::values`] and its siblings give every value that is set, by name, in name order. That list is what a serializer
//!   (the canonical JSON of IP-10, decision D02-10) reads, so these objects are ready to be serialized, and **this module serializes nothing itself**: the Model has no serializer yet.
//! * **Never validated here.** An override is what the user typed. Whether a value is usable is the resolver's to say, and it says so with a reason.

use std::hash::{Hash, Hasher};

use crate::params::ParameterValue;

/// The bits of an optional number, so that two settings are equal only if they are the same bits.
fn bits(value: Option<f64>) -> Option<u64> {
    value.map(f64::to_bits)
}

/// The settings that are set, by name, as the snapshot lists its values.
fn listed(
    plain: &[(&'static str, Option<f64>)],
    whole: &[(&'static str, Option<i64>)],
) -> Vec<(&'static str, ParameterValue)> {
    let mut found: Vec<(&'static str, ParameterValue)> = plain
        .iter()
        .filter_map(|(name, value)| value.map(|v| (*name, ParameterValue::Plain(v))))
        .chain(
            whole
                .iter()
                .filter_map(|(name, value)| value.map(|v| (*name, ParameterValue::Whole(v)))),
        )
        .collect();
    found.sort_by(|a, b| a.0.cmp(b.0));
    found
}

/// The timing offsets that the user set, **in sample-period steps**: `-21.0` shifts the commanded positions 21 sample periods earlier. A value that is not set is the file's.
// constructed-by: caller-options
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct TimingOverride {
    /// The offset of the commanded positions, in sample-period steps.
    pub commanded_offset_steps: Option<f64>,
    /// The offset of the actual positions, in sample-period steps.
    pub actual_offset_steps: Option<f64>,
}

impl TimingOverride {
    /// No override: every value is the file's.
    pub fn new() -> Self {
        TimingOverride::default()
    }

    pub fn with_commanded_offset_steps(mut self, steps: f64) -> Self {
        self.commanded_offset_steps = Some(steps);
        self
    }

    pub fn with_actual_offset_steps(mut self, steps: f64) -> Self {
        self.actual_offset_steps = Some(steps);
        self
    }

    /// Whether nothing is overridden.
    pub fn is_empty(&self) -> bool {
        self.values().is_empty()
    }

    /// Every value that is set, by name, in name order.
    pub fn values(&self) -> Vec<(&'static str, ParameterValue)> {
        listed(
            &[
                ("actual_offset_steps", self.actual_offset_steps),
                ("commanded_offset_steps", self.commanded_offset_steps),
            ],
            &[],
        )
    }

    fn key(&self) -> [Option<u64>; 2] {
        [
            bits(self.commanded_offset_steps),
            bits(self.actual_offset_steps),
        ]
    }
}

impl PartialEq for TimingOverride {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for TimingOverride {}

impl Hash for TimingOverride {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// The Z calibration that the user set: the field size of Z in millimetres and the bit resolution of the commanded and of the actual Z. **There is no offset here** (decision D27-18): Vision
/// has no override of the Z offset, which is the file's. A value that is not set is the file's.
// constructed-by: caller-options
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ZCalibrationOverride {
    /// The field size of Z, in millimetres.
    pub field_size_z_mm: Option<f64>,
    /// The bit resolution of the commanded Z.
    pub commanded_bit_resolution_z: Option<i64>,
    /// The bit resolution of the actual Z.
    pub actual_bit_resolution_z: Option<i64>,
}

impl ZCalibrationOverride {
    /// No override: every value is the file's.
    pub fn new() -> Self {
        ZCalibrationOverride::default()
    }

    pub fn with_field_size_z_mm(mut self, millimetres: f64) -> Self {
        self.field_size_z_mm = Some(millimetres);
        self
    }

    pub fn with_commanded_bit_resolution_z(mut self, bits: i64) -> Self {
        self.commanded_bit_resolution_z = Some(bits);
        self
    }

    pub fn with_actual_bit_resolution_z(mut self, bits: i64) -> Self {
        self.actual_bit_resolution_z = Some(bits);
        self
    }

    /// Whether nothing is overridden.
    pub fn is_empty(&self) -> bool {
        self.values().is_empty()
    }

    /// Every value that is set, by name, in name order.
    pub fn values(&self) -> Vec<(&'static str, ParameterValue)> {
        listed(
            &[("field_size_z_mm", self.field_size_z_mm)],
            &[
                ("actual_bit_resolution_z", self.actual_bit_resolution_z),
                (
                    "commanded_bit_resolution_z",
                    self.commanded_bit_resolution_z,
                ),
            ],
        )
    }

    fn key(&self) -> (Option<u64>, Option<i64>, Option<i64>) {
        (
            bits(self.field_size_z_mm),
            self.commanded_bit_resolution_z,
            self.actual_bit_resolution_z,
        )
    }
}

impl PartialEq for ZCalibrationOverride {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for ZCalibrationOverride {}

impl Hash for ZCalibrationOverride {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// The beam that the user set, in the units that Vision's settings use: **the waist diameter in micrometres, the Rayleigh length and the lengths in millimetres, the angles in degrees.** A
/// value that is not set falls through to the file's. The scanner rotation is here because spot size reads it (the major axis is given in the scanner frame).
// constructed-by: caller-options
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct SpotSizeOverride {
    /// The waist **diameter** along the major axis, in micrometres.
    pub waist_major_um: Option<f64>,
    /// The waist **diameter** along the minor axis, in micrometres.
    pub waist_minor_um: Option<f64>,
    /// The Rayleigh length along the major axis, in millimetres.
    pub rayleigh_major_mm: Option<f64>,
    /// The Rayleigh length along the minor axis, in millimetres.
    pub rayleigh_minor_mm: Option<f64>,
    /// The beam quality factor M-squared along the major axis, from which a Rayleigh length is derived when there is none.
    pub m2_major: Option<f64>,
    /// The beam quality factor M-squared along the minor axis.
    pub m2_minor: Option<f64>,
    /// Where the waist is relative to the commanded focus, in millimetres.
    pub waist_offset_z_mm: Option<f64>,
    /// The thermal-lensing shift of the focal plane, in millimetres.
    pub thermal_shift_mm: Option<f64>,
    /// The orientation of the major axis in the scanner frame, in degrees.
    pub major_axis_angle_deg: Option<f64>,
    /// The rotation of the scanner, in degrees.
    pub scanner_rotation_deg: Option<f64>,
}

impl SpotSizeOverride {
    /// No override: every value is the file's.
    pub fn new() -> Self {
        SpotSizeOverride::default()
    }

    pub fn with_waist_major_um(mut self, value: f64) -> Self {
        self.waist_major_um = Some(value);
        self
    }

    pub fn with_waist_minor_um(mut self, value: f64) -> Self {
        self.waist_minor_um = Some(value);
        self
    }

    pub fn with_rayleigh_major_mm(mut self, value: f64) -> Self {
        self.rayleigh_major_mm = Some(value);
        self
    }

    pub fn with_rayleigh_minor_mm(mut self, value: f64) -> Self {
        self.rayleigh_minor_mm = Some(value);
        self
    }

    pub fn with_m2_major(mut self, value: f64) -> Self {
        self.m2_major = Some(value);
        self
    }

    pub fn with_m2_minor(mut self, value: f64) -> Self {
        self.m2_minor = Some(value);
        self
    }

    pub fn with_waist_offset_z_mm(mut self, value: f64) -> Self {
        self.waist_offset_z_mm = Some(value);
        self
    }

    pub fn with_thermal_shift_mm(mut self, value: f64) -> Self {
        self.thermal_shift_mm = Some(value);
        self
    }

    pub fn with_major_axis_angle_deg(mut self, value: f64) -> Self {
        self.major_axis_angle_deg = Some(value);
        self
    }

    pub fn with_scanner_rotation_deg(mut self, value: f64) -> Self {
        self.scanner_rotation_deg = Some(value);
        self
    }

    /// Whether nothing is overridden.
    pub fn is_empty(&self) -> bool {
        self.values().is_empty()
    }

    /// Every value that is set, by name, in name order.
    pub fn values(&self) -> Vec<(&'static str, ParameterValue)> {
        listed(
            &[
                ("major_axis_angle_deg", self.major_axis_angle_deg),
                ("m2_major", self.m2_major),
                ("m2_minor", self.m2_minor),
                ("rayleigh_major_mm", self.rayleigh_major_mm),
                ("rayleigh_minor_mm", self.rayleigh_minor_mm),
                ("scanner_rotation_deg", self.scanner_rotation_deg),
                ("thermal_shift_mm", self.thermal_shift_mm),
                ("waist_major_um", self.waist_major_um),
                ("waist_minor_um", self.waist_minor_um),
                ("waist_offset_z_mm", self.waist_offset_z_mm),
            ],
            &[],
        )
    }

    fn key(&self) -> [Option<u64>; 10] {
        [
            bits(self.waist_major_um),
            bits(self.waist_minor_um),
            bits(self.rayleigh_major_mm),
            bits(self.rayleigh_minor_mm),
            bits(self.m2_major),
            bits(self.m2_minor),
            bits(self.waist_offset_z_mm),
            bits(self.thermal_shift_mm),
            bits(self.major_axis_angle_deg),
            bits(self.scanner_rotation_deg),
        ]
    }
}

impl PartialEq for SpotSizeOverride {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for SpotSizeOverride {}

impl Hash for SpotSizeOverride {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::collections::HashSet;

    use super::*;

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn a_new_override_overrides_nothing_and_lists_nothing() {
        assert!(TimingOverride::new().is_empty());
        assert!(ZCalibrationOverride::new().is_empty());
        assert!(SpotSizeOverride::new().is_empty());
        assert!(TimingOverride::new().values().is_empty());
        assert_eq!(TimingOverride::default(), TimingOverride::new());
        assert_eq!(ZCalibrationOverride::default(), ZCalibrationOverride::new());
        assert_eq!(SpotSizeOverride::default(), SpotSizeOverride::new());
    }

    #[test]
    fn a_value_that_is_set_is_listed_by_name_in_name_order() {
        let timing = TimingOverride::new()
            .with_commanded_offset_steps(-21.0)
            .with_actual_offset_steps(3.5);
        assert!(!timing.is_empty());
        assert_eq!(
            timing.values(),
            vec![
                ("actual_offset_steps", ParameterValue::Plain(3.5)),
                ("commanded_offset_steps", ParameterValue::Plain(-21.0)),
            ]
        );
        let z = ZCalibrationOverride::new()
            .with_field_size_z_mm(100.0)
            .with_commanded_bit_resolution_z(16)
            .with_actual_bit_resolution_z(18);
        assert_eq!(
            z.values(),
            vec![
                ("actual_bit_resolution_z", ParameterValue::Whole(18)),
                ("commanded_bit_resolution_z", ParameterValue::Whole(16)),
                ("field_size_z_mm", ParameterValue::Plain(100.0)),
            ]
        );
        // a value that is set to zero is set: zero is not "not overridden"
        let zero = SpotSizeOverride::new().with_thermal_shift_mm(0.0);
        assert_eq!(
            zero.values(),
            vec![("thermal_shift_mm", ParameterValue::Plain(0.0))]
        );
    }

    #[test]
    fn the_spot_size_override_has_the_beam_the_angles_and_the_scanner_rotation() {
        let all = SpotSizeOverride::new()
            .with_waist_major_um(66.0)
            .with_waist_minor_um(68.0)
            .with_rayleigh_major_mm(3.1)
            .with_rayleigh_minor_mm(3.2)
            .with_m2_major(1.1)
            .with_m2_minor(1.2)
            .with_waist_offset_z_mm(0.457)
            .with_thermal_shift_mm(1.178)
            .with_major_axis_angle_deg(5.0)
            .with_scanner_rotation_deg(-2.0);
        let names: Vec<&str> = all.values().iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "m2_major",
                "m2_minor",
                "major_axis_angle_deg",
                "rayleigh_major_mm",
                "rayleigh_minor_mm",
                "scanner_rotation_deg",
                "thermal_shift_mm",
                "waist_major_um",
                "waist_minor_um",
                "waist_offset_z_mm",
            ]
        );
    }

    #[test]
    fn equal_means_equal_bits_so_a_nan_equals_itself_and_zero_is_not_minus_zero() {
        let nan = SpotSizeOverride::new().with_thermal_shift_mm(f64::NAN);
        assert_eq!(nan, nan.clone());
        assert_eq!(hash_of(&nan), hash_of(&nan.clone()));
        let zero = TimingOverride::new().with_commanded_offset_steps(0.0);
        let minus_zero = TimingOverride::new().with_commanded_offset_steps(-0.0);
        assert_ne!(zero, minus_zero);
        // and "not set" is not zero
        assert_ne!(TimingOverride::new(), zero);
        assert_ne!(
            ZCalibrationOverride::new(),
            ZCalibrationOverride::new().with_commanded_bit_resolution_z(0)
        );
    }

    #[test]
    fn equal_overrides_hash_equal_and_a_set_holds_each_once() {
        let a = ZCalibrationOverride::new()
            .with_field_size_z_mm(100.0)
            .with_actual_bit_resolution_z(18);
        let b = ZCalibrationOverride::new()
            .with_actual_bit_resolution_z(18)
            .with_field_size_z_mm(100.0);
        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
        let set: HashSet<ZCalibrationOverride> =
            [a.clone(), b, a.with_commanded_bit_resolution_z(16)].into();
        assert_eq!(set.len(), 2);
        let spot: HashSet<SpotSizeOverride> = [
            SpotSizeOverride::new().with_m2_major(1.1),
            SpotSizeOverride::new().with_m2_major(1.1),
            SpotSizeOverride::new().with_m2_minor(1.1),
        ]
        .into();
        assert_eq!(spot.len(), 2);
    }

    #[test]
    fn an_override_is_a_value_that_can_be_sent_and_shared() {
        fn assert_traits<T: Clone + Eq + Hash + Default + Send + Sync + 'static>() {}
        assert_traits::<TimingOverride>();
        assert_traits::<ZCalibrationOverride>();
        assert_traits::<SpotSizeOverride>();
    }
}
