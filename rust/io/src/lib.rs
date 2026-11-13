//! The **IO layer**: reads and writes HDF5 files, translating between a file's own layout and the Model.
//!
//! * **Layer:** between the Model and the files. Version adapters (V16, V17, V18) live here, one module per version, from IP-03 and IP-04 on.
//! * **May depend on:** `build-log-model`, `build-log-archive` (the `.lbl` archive of V18 on), and the HDF5 binding (`hdf5-metno`, pinned once in the workspace manifest). In tests only: `build-log-testkit`.
//! * **Must not depend on:** `build-log-processing` or the `build-log` facade. Nothing above it may reach an HDF5 path or group name through it.
//!
//! The binding is here and proven to link (`tests/link_smoke.rs`). The first adapter is V17 (IP-13), V16 (IP-12, read-only) is the second and V18 (IP-05) follows, and the code that versions share is in
//! `toolkit/`.
//! The architecture guards in `rust/build-log/tests` enforce the dependency rules above.
//!
//! # Building without a libhdf5 on the machine
//!
//! The binding links to libhdf5. A machine without it builds with the `static-hdf5` feature, which compiles libhdf5 from source (see the README).

// Unsafe code is denied for every crate by the workspace lints; this crate needs none while it uses the `hdf5-metno` wrapper (decision D01-7),
// so it stays at `deny` rather than `forbid`.

/// The adapters (IP-04): one folder per on-disk version and the registration unit of a version. Internal: a consumer reaches an adapter only through the facade.
#[doc(hidden)]
pub mod adapters;
/// The column engine (IP-06): the code every adapter uses to turn a request for a column into values, with its range rules, blocks, precision and errors. It names no
/// layout word. Internal: a consumer reaches columns through the facade's `ColumnSource`.
#[doc(hidden)]
pub mod columns;
/// The container of a build log (V18 on): which kind of file a path is (a bare HDF5 file or an `.lbl` archive), the guard of an archive's source, and the working file a writer creates, sealed into an archive when the build is done.
/// The archive format itself is in the `build-log-archive` crate, which this layer uses and re-exports from here.
pub mod container;
/// The dispatch core (IP-04): the marker reader, the registry and the choice of an adapter. Internal: the facade's `open` is the contract.
#[doc(hidden)]
pub mod dispatch;
/// The HDF5 primitives (IP-03). Internal: the adapters of this crate and its tests use them; they are not part of the facade's contract.
#[doc(hidden)]
pub mod hdf5;
/// The toolkit of the adapters (IP-12): the code that more than one version uses, which names no attribute, no group and no version. Internal: the adapters of this crate use it.
#[doc(hidden)]
pub mod toolkit;
mod version;

/// The HDF5 binding this library is built on, re-exported so that a consumer names the very same types as `open_at` (a mount is a `Group` of it).
pub use ::hdf5 as binding;

pub use version::{
    check_libhdf5, check_linked_libhdf5, libhdf5_version, Libhdf5Version, ParseLibhdf5VersionError,
    LIBHDF5_MINIMUM,
};
