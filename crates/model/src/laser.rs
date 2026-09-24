use crate::by_layer::ByLayer;
use crate::corrections::Corrections;

/// Per-laser configuration, corresponding to the attributes on
/// `/Build/Lasers/Laser_N` in the reference fixture. Both `Laser_1` and
/// `Laser_2` are present and share this same attribute set; all fourteen
/// attributes are scalar, and every measurement here has its `_units`
/// companion present (unlike `Build`'s `Layer_Thickness` gap).
///
/// Which laser this is (`Laser_1` vs `Laser_2`) comes from the HDF5 group
/// name, not from an attribute inside it — deliberately not modeled as a
/// field here; that's a mapping/identity concern for whatever reads the
/// group, not part of what the group's own attributes describe.
///
/// Every field here is tracked independently per laser — nothing links
/// Laser_1's and Laser_2's values. `nominal_working_distance`,
/// `rayleigh_length`, and `wavelength` happen to be equal between them in
/// this fixture (both lasers share the same physical spec), while
/// `scan_head_offset_x`/`_y` and `scan_head_rotation` happen to differ
/// (Laser_2 is mounted roughly mirrored and rotated 180° from Laser_1).
/// Neither is a structural guarantee — it's what this fixture's data looks
/// like, not a rule the schema enforces.
///
/// See `docs/models/derivation.md` for the full fixture-path mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct Laser {
    /// Resolution actually achieved — tracked independently of
    /// `commanded_bit_resolution`, not derived from it. Both observed as
    /// `20` in this fixture, but nothing guarantees they always match.
    pub actual_bit_resolution: i32,
    /// Configured/setpoint resolution.
    pub commanded_bit_resolution: i32,
    pub nominal_working_distance: f64,
    pub nominal_working_distance_units: String,
    pub rayleigh_length: f64,
    pub rayleigh_length_units: String,
    pub scan_head_offset_x: f64,
    pub scan_head_offset_x_units: String,
    pub scan_head_offset_y: f64,
    pub scan_head_offset_y_units: String,
    pub scan_head_rotation: f64,
    pub scan_head_rotation_units: String,
    pub wavelength: f64,
    pub wavelength_units: String,
    /// Composed from the `Corrections` child group — everything above
    /// this field mirrors `Laser_N`'s own attributes; this and
    /// `monitoring_data` below come from nested groups instead.
    pub corrections: Corrections,
    pub monitoring_data: ByLayer<LaserMonitoringLayer>,
}

/// One layer's worth of laser position/power monitoring, corresponding to
/// `/Build/Lasers/Laser_N/Monitoring_Data/Layer_M`. All eight dataset
/// fields are parallel arrays of equal length — one entry per sample
/// index, not independent series. Verified counts match the fixture's
/// full-rate ClearBox-native sensor data exactly (727,922 for Layer_1,
/// 719,160 for Layer_2), consistent with sampling at the same rate as
/// those sensors.
///
/// `start_laser_time`/`end_laser_time` are `int64` — plausibly
/// microsecond-epoch timestamps (three orders of magnitude smaller than
/// the OPC-UA sensors' nanosecond-epoch `Source_Ts` values), but that's an
/// inference from magnitude, not confirmed against any documentation.
///
/// Laser_1 and Laser_2's data is byte-identical in this fixture (verified:
/// every dataset, full-array equality, not just matching samples) — a
/// function of how this fixture was constructed, not a rule to model or
/// rely on.
#[derive(Debug, Clone, PartialEq)]
pub struct LaserMonitoringLayer {
    pub start_laser_time: i64,
    pub end_laser_time: i64,
    pub actual_x: Vec<i32>,
    pub actual_y: Vec<i32>,
    pub actual_z: Vec<i32>,
    pub commanded_x: Vec<i32>,
    pub commanded_y: Vec<i32>,
    pub commanded_z: Vec<i32>,
    pub power: Vec<f32>,
    pub state: Vec<u8>,
}
