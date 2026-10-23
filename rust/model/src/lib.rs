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

// The Common Data Model (IP-02): one module per concept, in the layout of the plan's Appendix A. They are empty until the phases that fill them; consumers
// never name a module, because every public type is re-exported from the crate root (BL-AR-03). The policy guards in `tests/policy_guards.rs` read each of
// these files.
mod capabilities;
mod channel;
mod column;
mod error;
mod events;
mod extra;
mod identity;
mod json;
mod meta;
mod params;
mod provenance;
mod quality;
mod time;
mod units;

// The pre-cleanup types, shaped like the V18 file and holding whole `Vec`s. They leave in IP-02 phase 2T; the policy guards skip these five files, and the
// list of them in the guards may only get shorter.
mod build;
mod by_layer;
mod corrections;
mod laser;
mod sensor;

// The Common Data Model, re-exported from the root: consumers never name a module (BL-AR-03).
pub use capabilities::{Capabilities, Direction, Feature, VersionReport, CDM_VERSION};
pub use channel::{
    Axis, CapturedEncoding, CapturedWidth, ChannelDType, ChannelDescriptor, ChannelKind,
    ClockClass, EnumLabel, EnumLabels, NumericType, Renderability,
};
pub use column::{
    BuildSource, CancelToken, ColumnData, ColumnRange, ColumnRequest, ColumnSource, InMemorySource,
    Precision, ProgressSink, ReadContext, Revision, Shared, CANCEL_CHECK_INTERVAL,
};
pub use error::{Error, ErrorClass, ErrorKind, Location, Violation};
pub use extra::{
    AttrShape, AttrValue, Dims, Extra, ExtraAttribute, NodeKind, StoredAs, StoredClass,
    TextCharset, TextPadding, UnknownNode, MAX_ATTRIBUTE_BYTES, MAX_DIMS,
};
pub use identity::{Availability, ByLayer, ChannelKey, LaserKey, LayerId, Presence};
pub use meta::{BuildIdentity, BuildMeta, LaserLayerMeta, LaserMeta, LayerMeta};
pub use provenance::{Origin, Origins};
pub use time::{
    check_segments, index_range, Accuracy, Instant, Segment, StartMeaning, TimeAnchor, TimeAxis,
    TimeAxisSummary, TimeValue, TimeWindow,
};
pub use units::{parse_embedded, Dimension, Quantity, Unit, UnitSource};

// The pre-cleanup types (see above). Their `ByLayer` keeps its old name inside the crate and is exported as `LegacyByLayer` until phase 2T, because the CDM's
// `ByLayer` (keyed by `LayerId`) has the root name.
pub use build::Build;
pub use by_layer::ByLayer as LegacyByLayer;
pub use corrections::{
    CalibrationCurve, CorrectionFile, Corrections, EquationConstant, InverseCorrectionGrid,
};
pub use laser::{Laser, LaserMonitoringLayer};
pub use sensor::{
    EventRecord, GrayscaleImage, ImagePair, OpcUaReading, Sensor, SensorCalibration, SensorKind,
};
