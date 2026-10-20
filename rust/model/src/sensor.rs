use crate::by_layer::ByLayer;
use crate::corrections::CalibrationCurve;

/// A single entry under `/Build/Sensors`. Identity (which sensor this is)
/// comes from the HDF5 group name, same as `Laser` — deliberately not a
/// field here.
#[derive(Debug, Clone, PartialEq)]
pub struct Sensor {
    pub kind: SensorKind,
}

/// The four structurally distinct shapes observed under `/Build/Sensors`
/// (42 children, reference fixture, verified 2026-09-24): 35 OPC-UA
/// sensors, 2 ClearBox-native sensors, 4 event/command markers, and 1
/// image-capture record (`Images`). Each carries a genuinely different
/// per-layer data shape — not just different attributes — so this is
/// modeled as variants rather than one struct with a pile of optional
/// fields: an OPC-UA reading is never mixed with image buffers in real
/// data, and this makes that combination unrepresentable rather than
/// something every consumer has to check for at runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum SensorKind {
    OpcUa {
        /// `Calibration_Data`'s one attribute. Always observed empty in
        /// this fixture, across all 35 sensors — kept anyway since it
        /// wasn't asked to be removed, unlike `Scope`/`Signal_Type` (see
        /// `docs/models/derivation.md`).
        calibration_output_units: String,
        monitoring_data: ByLayer<OpcUaReading>,
    },
    ClearBoxNative {
        calibration: SensorCalibration,
        /// Just a flat per-layer reading series — no companion
        /// timestamps, unlike `OpcUaReading`. Presumed regularly sampled
        /// at `calibration.sample_period`, so per-sample timestamps
        /// aren't stored.
        monitoring_data: ByLayer<Vec<f64>>,
    },
    EventMarker {
        monitoring_data: ByLayer<EventRecord>,
    },
    ImageCapture {
        monitoring_data: ByLayer<ImagePair>,
    },
}

/// One layer's worth of OPC-UA readings — a timeseries, not a single
/// value (11 to 33+ samples observed per layer in this fixture).
/// `value` is stored as raw strings, matching the fixture exactly: HDF5
/// dtype is `object` (byte-strings), not a native numeric/bool type, and
/// what a given string actually means (e.g. `"True"`/`"False"` vs a
/// number) isn't recorded anywhere anymore now that `Signal_Type` is
/// gone. Parsing is deliberately left to whatever reads this — Model
/// doesn't presuppose it.
#[derive(Debug, Clone, PartialEq)]
pub struct OpcUaReading {
    pub source_ts: Vec<u64>,
    pub value: Vec<String>,
}

/// ClearBox-native sensor calibration: the shared `CalibrationCurve` core
/// plus fields specific to sensor calibration that `Power_Calibration`
/// doesn't have.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorCalibration {
    pub curve: CalibrationCurve,
    pub input_range: (f64, f64),
    pub output_range: (f64, f64),
    pub sample_period: f64,
}

/// A single event/command record for one layer — not a timeseries.
/// Verified `shape=(1,)` for every dataset field across all four
/// EventMarker sensors in the reference fixture (`AddLayerCommand`,
/// `CameraSnapshotCommand_1`, `ExposeCommand_scan_manager_scanner_1`,
/// `WaitCommand`). `value` is optional: some markers (e.g.
/// `AddLayerCommand`) don't have one; where present, content is opaque
/// (e.g. `WaitCommand`'s was `"{}"` — looks like serialized JSON, not
/// parsed here).
#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    pub z_height: f64,
    pub source_ts: u64,
    pub start_ns: u64,
    pub stop_ns: u64,
    pub value: Option<String>,
}

/// A pre/post-exposure grayscale image pair for one layer. Corresponds to
/// `Images/Monitoring_Data/Layer_N/{Pre_Exposure,Post_Exposure}`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImagePair {
    pub pre_exposure: GrayscaleImage,
    pub post_exposure: GrayscaleImage,
}

/// A single grayscale image buffer. Confirmed `(5068, 5064)` `uint8` in
/// the reference fixture — dimensions tracked per-instance, not assumed
/// fixed. The fixture also carries standard HDF5 Image Specification
/// attrs (`CLASS`, `IMAGE_SUBCLASS`, `IMAGE_VERSION`) on the underlying
/// dataset; not modeled here since they were identical across every
/// image observed, not meaningful per-instance variation.
#[derive(Debug, Clone, PartialEq)]
pub struct GrayscaleImage {
    pub data: Vec<u8>,
    pub height: usize,
    pub width: usize,
}
