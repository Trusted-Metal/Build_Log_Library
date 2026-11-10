//! Opening a build (IP-04 phase 4F, tasks T04.11 and T04.12; requirements BL-AR-03, BL-AR-08 and BL-VA-12).
//!
//! The facade's `open` is the contract of a consumer: it reads the marker of the file, chooses the adapter registered for that version and gives back the build
//! as the Model's own types. **There is no logic here**: every function is a call to the dispatch core with the production registry, or with one that the caller
//! injects (`*_with`, for a consumer's own tests, see [`crate::testing`]). A new version needs no change in a consumer, because nothing here or there names one.

use std::path::Path;

use build_log_io::binding::Group;
use build_log_io::dispatch;
use build_log_model::{BuildSource, Capabilities, Error, VersionReport, CDM_VERSION};

use crate::{Registry, VersionKey, PACKAGE_VERSION};

/// A build that was opened: the source of its metadata and columns, which version the file is, the adapter that reads it and what that version can do.
#[derive(Debug)]
pub struct Opened {
    inner: dispatch::Opened,
}

impl Opened {
    /// The build, as the Model's own source of metadata and columns.
    pub fn source(&self) -> &dyn BuildSource {
        self.inner.source()
    }

    /// The same, handed over.
    pub fn into_source(self) -> Box<dyn BuildSource> {
        self.inner.into_source()
    }

    /// The version of the file: the key of the adapter that reads it. The file is never upgraded or downgraded.
    pub fn key(&self) -> &VersionKey {
        self.inner.key()
    }

    /// The adapter in use, by name.
    pub fn adapter(&self) -> &str {
        self.inner.adapter()
    }

    /// What this version can read and write.
    pub fn capabilities(&self) -> &Capabilities {
        self.inner.capabilities()
    }

    /// The three version numbers and the adapter in use (BL-AR-08): the version of the file, of the Common Data Model and of this package.
    pub fn report(&self) -> VersionReport {
        self.inner.report(PACKAGE_VERSION)
    }
}

/// What the library is, without a file: the version of the Common Data Model, the version of this package and the versions of a file that it can open.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LibraryVersions {
    /// The version of the Common Data Model.
    pub cdm_version: String,
    /// The version of this package, which is what an application depends on.
    pub package_version: String,
    /// The versions of a file that can be opened, in the order of the versions.
    pub supported_versions: Vec<String>,
}

/// Opens the file at `path`, read-only, and reads it with the adapter of the version it says it is. `Unversioned` if it says nothing, `UnsupportedVersion` if
/// no adapter reads that version.
pub fn open(path: impl AsRef<Path>) -> Result<Opened, Error> {
    open_with(dispatch::production(), path)
}

/// Opens the build whose data is under `mount`, a group of a file the caller holds open. The mount can be anywhere in the file and can be called anything: it is
/// the group that holds the library's data, and the build read from it is the same as from a standalone file.
pub fn open_at(mount: &Group) -> Result<Opened, Error> {
    open_at_with(dispatch::production(), mount)
}

/// [`open`] with a registry of the caller's choosing, which is how a consumer's test opens the mock version ([`crate::testing::mock_registry`]).
pub fn open_with(registry: &Registry, path: impl AsRef<Path>) -> Result<Opened, Error> {
    dispatch::open(registry, path).map(|inner| Opened { inner })
}

/// [`open_at`] with a registry of the caller's choosing.
pub fn open_at_with(registry: &Registry, mount: &Group) -> Result<Opened, Error> {
    dispatch::open_at(registry, mount).map(|inner| Opened { inner })
}

/// The versions of a file that this library can open, in the order of the versions.
pub fn supported_versions() -> Vec<String> {
    dispatch::production().supported_versions()
}

/// The library's own versions and what it can open, without a file.
pub fn library_versions() -> LibraryVersions {
    LibraryVersions {
        cdm_version: CDM_VERSION.to_string(),
        package_version: PACKAGE_VERSION.to_string(),
        supported_versions: supported_versions(),
    }
}
