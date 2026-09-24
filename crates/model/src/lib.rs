//! Plain Rust types mirroring the Build Log HDF5 schema.
//!
//! Types here are derived empirically from a reference fixture
//! (`fixtures/reference_build_log_opcua_and_clearbox_sensors.h5`) rather
//! than a pre-existing spec. See `docs/models/derivation.md` for the
//! fixture-path-to-type mapping, and treat these shapes as pre-peer-review
//! until noted otherwise.

mod build;
mod by_layer;
mod corrections;
mod laser;
mod sensor;

pub use build::Build;
pub use by_layer::ByLayer;
pub use corrections::{
    CalibrationCurve, CorrectionFile, Corrections, EquationConstant, InverseCorrectionGrid,
};
pub use laser::{Laser, LaserMonitoringLayer};
pub use sensor::{
    EventRecord, GrayscaleImage, ImagePair, OpcUaReading, Sensor, SensorCalibration, SensorKind,
};
