//! `build-log`: the **public entry point** of the Build Log Library, the one crate that consumers depend on (by git tag; nothing is published).
//!
//! * **Layer:** the facade. It re-exports the Model at its root and the other layers behind features, and later orchestrates them.
//! * **May depend on:** `build-log-model`; `build-log-io` behind the `hdf5` feature; `build-log-processing` behind the `processing` feature. In tests only: `build-log-testkit`.
//! * **Nothing inside the workspace depends on this crate.** Only outside consumers do.
//!
//! # Features
//!
//! | Feature | What it adds |
//! |---|---|
//! | `hdf5` (default) | opening a build: [`open`], [`open_at`], [`supported_versions`], [`library_versions`]; the HDF5 binding as `hdf5`; the module `io`; `testing` |
//! | `processing` | the Processing layer, as the module `processing` |
//! | `jobs` | placeholder for the job layer (IP-15); implies `hdf5` |
//! | `specta` | placeholder for TypeScript type generation (IP-02) |
//! | `static-hdf5` | build libhdf5 from source instead of linking the system library; implies `hdf5` |
//!
//! **No feature changes the on-disk schema.** A feature adds or removes code, never what a file means or contains.
//! With `--no-default-features` the crate is the Model only.
//!
//! # Opening a build
//!
//! A consumer calls [`open`] with a path (or [`open_at`] with the group that holds the library's data) and gets an [`Opened`]: the build as the Model's own
//! types, the version of the file, the adapter that read it, what that version can do and [`Opened::report`], the three version numbers. The library chooses the
//! adapter from the file's own version marker, so **a consumer holds no code that depends on a version, and a new version needs no change in it**. Nothing here
//! reaches an adapter or a group name: the adapters are not part of this crate's surface, and a guard test checks that.
//!
//! The production registry holds the versions that have an adapter: V17 so far (IP-13), then V18 and V16. [`open`] on a file of another version answers
//! `UnsupportedVersion` and names the versions it can read. [`testing`] holds the mock version, so a consumer can be written and tried against a file of a
//! version that is not real. `examples/mock_consumer.rs` is the shape of an integration.

#![forbid(unsafe_code)]

pub use build_log_model::*;

/// The version of this package: the one an application depends on, and the third number of a [`VersionReport`].
pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The HDF5 binding the library is built on, so that a consumer names the very same types as [`open_at`] (a mount is a [`hdf5::Group`] of it).
#[cfg(feature = "hdf5")]
pub use build_log_io::binding as hdf5;

#[cfg(feature = "hdf5")]
mod open;
#[cfg(feature = "hdf5")]
pub mod testing;

#[cfg(feature = "hdf5")]
pub use build_log_io::dispatch::{Registry, VersionKey};
#[cfg(feature = "hdf5")]
pub use open::{
    library_versions, open, open_at, open_at_with, open_with, supported_versions, LibraryVersions,
    Opened,
};

/// The part of the IO layer that a consumer sees: which libhdf5 is linked and whether it is new enough. The adapters and the dispatch core are not here:
/// [`open`] is the way in.
#[cfg(feature = "hdf5")]
pub mod io {
    pub use build_log_io::{
        check_libhdf5, check_linked_libhdf5, libhdf5_version, Libhdf5Version,
        ParseLibhdf5VersionError, LIBHDF5_MINIMUM,
    };
}

/// The Processing layer (derived computation over Model data). Present with the `processing` feature.
#[cfg(feature = "processing")]
pub use build_log_processing as processing;
