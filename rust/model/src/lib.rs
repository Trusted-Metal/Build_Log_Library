//! The Common Data Model of the Build Log Library: plain Rust types for a build, its layers, lasers and channels, and the requests that reach their samples.
//!
//! The types are version-neutral: the adapter of each file version fills them, and no type names a group or an attribute of any file. `docs/models/cdm.md`
//! has the map of the modules and the rules for changing them, and the shapes stay provisional until the first consumers have read them.
//!
//! * **Layer:** the Model, the bottom of the library. Plain data types, no I/O.
//! * **May depend on:** no other crate of this workspace (in tests only: `build-log-testkit`), and **no HDF5**. It must build for `wasm32`.
//! * **Who depends on it:** every other layer, and the `build-log` facade, which re-exports it at its root.

#![forbid(unsafe_code)]

// The Common Data Model (IP-02): one module per concept, in the layout of the plan's Appendix A. Consumers never name a module, because every public type is
// re-exported from the crate root (BL-AR-03). The policy guards in `tests/policy_guards.rs` read each of these files.
mod availability;
mod capabilities;
mod channel;
mod column;
mod context;
mod error;
mod event_columns;
mod events;
mod extra;
mod grids;
mod identity;
mod image_regions;
mod json;
mod meta;
mod params;
mod provenance;
mod quality;
mod time;
mod units;
mod validation;

// The Common Data Model, re-exported from the root: consumers never name a module (BL-AR-03).
pub use availability::{
    availability_of, AvailabilityCounts, AvailabilityReport, Item, ItemAvailability, ItemKind,
};
pub use capabilities::{Capabilities, Direction, Feature, VersionReport, CDM_VERSION};
pub use channel::{
    Axis, CapturedEncoding, CapturedWidth, ChannelDType, ChannelDescriptor, ChannelKind,
    ClockClass, EnumLabel, EnumLabels, ImageFormat, NumericType, Renderability,
};
pub use column::{
    BuildSource, CacheStats, CancelToken, ColumnData, ColumnRange, ColumnRequest, ColumnSource,
    InMemorySource, Precision, ProgressSink, ReadContext, Revision, Shared, CANCEL_CHECK_INTERVAL,
};
pub use context::{BuildContext, BuildPlate, PLATE_AS_AXES, PLATE_AS_DIMENSION};
pub use error::{Error, ErrorClass, ErrorKind, Location, Violation};
pub use event_columns::{EventColumns, EventPlace, EventRequest, TypedValues};
pub use events::{EventDataType, EventKind, EventPart, EventSeries};
pub use extra::{
    AttrShape, AttrValue, Dims, Extra, ExtraAttribute, NodeKind, StoredAs, StoredClass,
    TextCharset, TextPadding, UnknownNode, MAX_ATTRIBUTE_BYTES, MAX_DIMS,
};
pub use grids::{CorrectionGrid, GridData, GridInterpretation, GridRequest, GridRole};
pub use identity::{Availability, ByLayer, ChannelKey, LaserKey, LayerId, Presence};
pub use image_regions::{ImageData, ImageRegion, ImageRequest};
pub use meta::{BuildIdentity, BuildMeta, LaserLayerMeta, LaserMeta, LayerMeta};
pub use params::{
    Equation, EquationKind, GalvoParameters, GalvoProtocol, LaserParameters, LightSourceParameters,
    NamedConstant, OpticalTrainParameters, ParameterValue, ScannerCardParameters,
    ScannerParameters, BIT_RESOLUTION_GLOBAL, SCAN_HEAD_OFFSET_SIGN_OPPOSITE,
};
pub use provenance::{JoinStatus, NotReadCause, Origin, Origins, Provenance};
pub use quality::{Outcome, QualityRecord};
pub use time::{
    check_segments, index_range, Accuracy, Instant, Segment, StartMeaning, TimeAnchor, TimeAxis,
    TimeAxisSummary, TimeValue, TimeWindow,
};
pub use units::{parse_embedded, Dimension, Quantity, Unit, UnitSource};
pub use validation::{
    validate_build, Finding, FindingCode, Severity, ValidationCounts, ValidationReport,
};
