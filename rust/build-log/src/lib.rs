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
//! | `hdf5` (default) | the IO layer, as the module `io` |
//! | `processing` | the Processing layer, as the module `processing` |
//! | `jobs` | placeholder for the job layer (IP-15); implies `hdf5` |
//! | `specta` | placeholder for TypeScript type generation (IP-02) |
//! | `static-hdf5` | build libhdf5 from source instead of linking the system library; implies `hdf5` |
//!
//! **No feature changes the on-disk schema.** A feature adds or removes code, never what a file means or contains.
//! With `--no-default-features` the crate is the Model only.
//!
//! This is a skeleton: the curated public surface arrives with the dispatch core (IP-04) and the adapters.

#![forbid(unsafe_code)]

pub use build_log_model::*;

/// The IO layer (HDF5 files to and from the Model). Present with the `hdf5` feature.
#[cfg(feature = "hdf5")]
pub use build_log_io as io;

/// The Processing layer (derived computation over Model data). Present with the `processing` feature.
#[cfg(feature = "processing")]
pub use build_log_processing as processing;
