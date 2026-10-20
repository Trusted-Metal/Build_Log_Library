//! The **IO layer**: reads and writes HDF5 files, translating between a file's own layout and the Model.
//!
//! * **Layer:** between the Model and the files. Version adapters (V16, V17, V18) live here, one module per version, from IP-03 and IP-04 on.
//! * **May depend on:** `build-log-model`, and the HDF5 binding (`hdf5-metno`, pinned once in the workspace manifest). In tests only: `build-log-testkit`.
//! * **Must not depend on:** `build-log-processing` or the `build-log` facade. Nothing above it may reach an HDF5 path or group name through it.
//!
//! The binding is here and proven to link (`tests/link_smoke.rs`); the adapters are not: the first arrives with IP-05 and IP-13.
//! The architecture guards in `rust/build-log/tests` enforce the dependency rules above.
//!
//! # Building without a libhdf5 on the machine
//!
//! The binding links to libhdf5. A machine without it builds with the `static-hdf5` feature, which compiles libhdf5 from source (see the README).

// Unsafe code is denied for every crate by the workspace lints; this crate needs none while it uses the `hdf5-metno` wrapper (decision D01-7),
// so it stays at `deny` rather than `forbid`.

mod version;

pub use version::{libhdf5_version, Libhdf5Version, ParseLibhdf5VersionError};
