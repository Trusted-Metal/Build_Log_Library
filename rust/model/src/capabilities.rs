//! Versioning as a consumer sees it: what a version can do, and which versions are in play (IP-04 phase 4A; decisions D04-5 and D04-12).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** library (`Capabilities`, `VersionReport`); the enums `Feature` and `Direction`.
//! * **Requirement:** BL-AR-08 (three version numbers, readable at runtime) and BL-VA-12 (capability flags per adapter set).
//!
//! # What this module claims
//!
//! * **Capabilities are per feature and per direction.** A version may be able to read events and not write them, so each [`Direction`] has its own set
//!   of [`Feature`]s. A consumer asks before it tries: tauri before it writes ("can V17 hold images?"), Vision before it draws ("does this file's version
//!   record events?"). An adapter builds its `Capabilities` once, with the builder; a consumer only asks.
//! * **A version report names three versions and the adapter.** The *schema version* is the key of the adapter in use (a string, as G3 decided:
//!   `"17"`), the *CDM version* is the version of this crate, and the *package version* is the version of the facade that the application depends on.
//!   They are unrelated numbers and are reported separately (BL-AR-08).
//! * **This is data, not behaviour.** The registry that decides which adapter handles a file is in the IO layer (IP-04 phase 4C); the Model only holds
//!   what a consumer reads of the decision.

use std::collections::BTreeSet;
use std::fmt;

/// The version of the Common Data Model: the version of this crate (semver of the public model).
pub const CDM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Something a version of the file format may or may not be able to hold. Each is a feature of the data, not of a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// The table of runs of consecutive stored samples of a gated capture (BL-CD-10).
    Segments,
    /// OPC UA events and task events.
    Events,
    /// Camera images.
    Images,
    /// The quality outcomes of a layer or a laser (including whether a laser completed).
    Outcomes,
    /// Sensor series.
    Sensors,
    /// The integrity digests of the data.
    Digests,
    /// Blobs that are kept as they are, such as the correction file.
    OpaqueBlobs,
}

/// Whether a version is being read or written.
// closed: a version is read or it is written, and nothing else
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    Read,
    Write,
}

/// What a version can do, by feature and by direction. Nothing is supported until an adapter says so.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Capabilities {
    read: BTreeSet<Feature>,
    write: BTreeSet<Feature>,
}

impl Capabilities {
    /// A version that can do nothing yet; the builder adds what it can.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same, and it can also do `feature` in `direction`. Saying it twice changes nothing.
    pub fn with(mut self, direction: Direction, feature: Feature) -> Self {
        self.set_mut(direction).insert(feature);
        self
    }

    /// The same, and it can also do each of `features` in `direction`.
    pub fn with_all(
        mut self,
        direction: Direction,
        features: impl IntoIterator<Item = Feature>,
    ) -> Self {
        self.set_mut(direction).extend(features);
        self
    }

    /// Can the version do `feature` in `direction`?
    pub fn supports(&self, direction: Direction, feature: Feature) -> bool {
        self.set(direction).contains(&feature)
    }

    /// The features the version can do in `direction`, in the order the features are declared (the same order every time).
    pub fn features(&self, direction: Direction) -> Vec<Feature> {
        self.set(direction).iter().copied().collect()
    }

    fn set(&self, direction: Direction) -> &BTreeSet<Feature> {
        match direction {
            Direction::Read => &self.read,
            Direction::Write => &self.write,
        }
    }

    fn set_mut(&mut self, direction: Direction) -> &mut BTreeSet<Feature> {
        match direction {
            Direction::Read => &mut self.read,
            Direction::Write => &mut self.write,
        }
    }
}

/// The three version numbers and the adapter in use, as one call reports them (BL-AR-08).
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VersionReport {
    /// The version of the file format: the key of the adapter in use, a string such as `"17"` (decision G3).
    pub schema_version: String,
    /// The version of the Common Data Model, which is the version of this crate ([`CDM_VERSION`]).
    pub cdm_version: String,
    /// The version of the package that the application depends on: the facade crate, which supplies it.
    pub package_version: String,
    /// The adapter in use, by name.
    pub adapter: String,
}

impl VersionReport {
    /// A report for the adapter `adapter`, whose key is `schema_version`, as seen from the package `package_version`. The CDM version is this crate's own.
    pub fn new(
        schema_version: impl Into<String>,
        adapter: impl Into<String>,
        package_version: impl Into<String>,
    ) -> Self {
        VersionReport {
            schema_version: schema_version.into(),
            cdm_version: CDM_VERSION.to_string(),
            package_version: package_version.into(),
            adapter: adapter.into(),
        }
    }
}

impl fmt::Display for VersionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "schema {}, CDM {}, package {}, adapter {}",
            self.schema_version, self.cdm_version, self.package_version, self.adapter
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_per_feature_and_per_direction() {
        // a version that can read events and images but write only images
        let caps = Capabilities::new()
            .with_all(Direction::Read, [Feature::Images, Feature::Events])
            .with(Direction::Write, Feature::Images);
        assert!(caps.supports(Direction::Read, Feature::Events));
        assert!(caps.supports(Direction::Read, Feature::Images));
        assert!(
            !caps.supports(Direction::Write, Feature::Events),
            "reading a feature says nothing about writing it"
        );
        assert!(caps.supports(Direction::Write, Feature::Images));
        // anything not said is not supported, in either direction
        for direction in [Direction::Read, Direction::Write] {
            assert!(!caps.supports(direction, Feature::Digests));
        }
        // the lists come in the order the features are declared, whatever order they were added in
        assert_eq!(
            caps.features(Direction::Read),
            vec![Feature::Events, Feature::Images]
        );
        assert_eq!(caps.features(Direction::Write), vec![Feature::Images]);
        // saying a thing twice changes nothing, and a new value supports nothing
        assert_eq!(caps.clone().with(Direction::Read, Feature::Events), caps);
        assert!(Capabilities::new().features(Direction::Read).is_empty());
        assert_eq!(Capabilities::new(), Capabilities::default());
    }

    #[test]
    fn a_version_report_names_the_three_versions_and_the_adapter() {
        let report = VersionReport::new("17", "v17", "1.2.3");
        assert_eq!(report.schema_version, "17");
        assert_eq!(report.adapter, "v17");
        assert_eq!(report.package_version, "1.2.3");
        // the CDM version is this crate's own, whatever the caller says about the package
        assert_eq!(report.cdm_version, CDM_VERSION);
        assert!(!CDM_VERSION.is_empty());
        // the three are separate numbers in one line, for an About box or a log
        let line = report.to_string();
        for part in ["schema 17", "package 1.2.3", "adapter v17", CDM_VERSION] {
            assert!(line.contains(part), "{line}");
        }
    }
}
