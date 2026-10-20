//! Plain Rust types mirroring the Build Log HDF5 schema.
//!
//! Types here are derived empirically from a reference fixture
//! (`fixtures/reference_build_log_opcua_and_clearbox_sensors.h5`) rather
//! than a pre-existing spec. See `docs/models/derivation.md` for the
//! fixture-path-to-type mapping, and treat these shapes as pre-peer-review
//! until noted otherwise.
//!
//! * **Layer:** the Model, the bottom of the library. Plain data types, no I/O.
//! * **May depend on:** no other crate of this workspace (in tests only: `build-log-testkit`), and **no HDF5**. It must build for `wasm32`.
//! * **Who depends on it:** every other layer, and the `build-log` facade, which re-exports it at its root.

#![forbid(unsafe_code)]

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
