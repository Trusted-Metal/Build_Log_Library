/// Corresponds to `/Build/Lasers/Laser_N/Corrections`. The group itself
/// carries no attributes — it's purely a container for the three children
/// below.
///
/// See `docs/models/derivation.md` for the full fixture-path mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct Corrections {
    pub correction_file: CorrectionFile,
    pub inverse_correction_data: InverseCorrectionGrid,
    pub power_calibration: CalibrationCurve,
}

/// Corresponds to `Corrections/Correction_File` — the actual `.fc3`
/// scan-field-correction file, stored inline as raw bytes, plus its
/// provenance metadata.
///
/// `file_size` is redundant with `bytes.len()` (verified: both fixture
/// instances have `file_size` exactly equal to the dataset's own byte
/// count) — kept as a real field for now since removing it wasn't asked
/// for here, unlike the equivalent redundancy already cleaned up on
/// `Inverse_Correction_Data`.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionFile {
    pub bytes: Vec<u8>,
    pub document_created_at: String,
    pub document_id: String,
    pub document_name: String,
    pub document_type: String,
    pub file_size: i32,
    pub original_uri: String,
    pub valid_as_of_date: String,
}

/// Corresponds to `Corrections/Inverse_Correction_Data` — a numeric grid,
/// confirmed `(257, 257, 2)` `float64` in this fixture, with zero attrs
/// (its `dimensions`/`dtype`/`shape` attrs were removed as redundant with
/// the dataset's own intrinsic shape/dtype).
///
/// Deliberately a flat `Vec<f64>` plus its shape, not `ndarray::Array3`:
/// which numeric-array representation is most convenient depends on how
/// the (not-yet-built) IO layer actually reads HDF5 datasets — likely via
/// `hdf5-metno`'s native `ndarray` integration, but that's an IO decision,
/// not a Model one. A flat `Vec` adds no dependency and converts trivially
/// into whatever IO ends up choosing.
///
/// Shape is tracked per-instance, not assumed fixed at `(257, 257, 2)` —
/// only one fixture has been checked.
#[derive(Debug, Clone, PartialEq)]
pub struct InverseCorrectionGrid {
    pub data: Vec<f64>,
    pub shape: (usize, usize, usize),
}

/// A calibration curve: an algorithm mapping input readings to output
/// values, expressed as an equation plus its numeric constants.
/// Corresponds to `Corrections/Power_Calibration`, and shares this exact
/// shape with the core of each ClearBox-native sensor's `Calibration_Data`
/// (see `sensor.rs`'s `SensorCalibration`) — confirmed identical after
/// fixture cleanup renamed `Derived_Output_units` to `Output_units` (see
/// `docs/models/derivation.md`).
///
/// `algorithm_type` is plain `String`, not an enum — deliberately: for
/// `Power_Calibration` specifically, `Linear`/`Polynomial` are a real
/// curated, accepted set (`Polynomial` preferred), but enforcing that is
/// a business rule, not a fact about the fixture's shape, and belongs at
/// a future validation/adapter layer, not baked into this raw mirror
/// type. For ClearBox-native sensors, the set is confirmed open-ended
/// (`"Log-Linear"` observed, more expected) — `String` is simply correct
/// there, not just cautious. One shared type, one representation, for
/// both.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationCurve {
    pub algorithm_equation: String,
    pub algorithm_type: String,
    pub input_units: String,
    pub output_units: String,
    /// Row order is not meaningful — verified non-alphabetical and
    /// non-equation-order in this fixture (Laser_1/LINEAR stores `b`
    /// before `a`). Consumers must match on `name`, never on position.
    pub derivation_equation_constants: Vec<EquationConstant>,
}

/// One row of `Power_Calibration/Derivation_Equation_Constants` — a
/// compound dataset of `(name: fixed 64-byte string, value: float64)`
/// pairs. Row count varies with `algorithm_type` (2 for `LINEAR`, 3 for
/// `POLYNOMIAL` in this fixture).
#[derive(Debug, Clone, PartialEq)]
pub struct EquationConstant {
    pub name: String,
    pub value: f64,
}
