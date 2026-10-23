//! Opening a file through the registry (IP-04 phase 4D, tasks T04.7 and T04.8, decisions D04-7 to D04-9).
//!
//! * **The mount is the primitive** (BL-AR-09). [`open_at`] takes the group that holds the library's data, wherever it is and whatever it is called; it reads
//!   the marker from it, chooses the adapter and hands it the mount. Every adapter reads its paths relative to the mount, so the same data opened standalone
//!   or nested at any depth gives the same build. [`open`] is a wrapper: it opens the file, finds the mount among the root's children by the places that the
//!   registered adapters declare, and calls [`open_at`].
//! * **Outcomes are typed** (Appendix B of the plan): no marker is `Unversioned`; a version nobody registered is `UnsupportedVersion`; a marker on two mounts is
//!   `Corrupt` and is not resolved by choosing one (the caller can open the mount it wants with [`open_at`]); a marker that is not where its version keeps it
//!   is `Corrupt`. `open` preserves the on-disk version: nothing upgrades or downgrades a file, and it is opened read-only.
//! * **The dispatcher never probes structure by itself** (decision D04-8). [`open_probing`] is the explicit entry point: for a file with no marker, an adapter
//!   that registered a probe may recognise its own version by structure. A probe runs only when the caller asks, and only if no marker was found.

use std::path::Path;

use build_log_model::{BuildSource, Capabilities, Error, ErrorKind, VersionReport};
use hdf5::Group;

use super::marker::{peek_marker, VersionKey};
use super::registry::{resolve, Registry};
use crate::adapters::{AdapterSet, Surface};
use crate::hdf5::{from_binding, open_file};

/// A build that was opened through the registry: the source, which version it is, the adapter that reads it and what that version can do.
pub struct Opened {
    source: Box<dyn BuildSource>,
    key: VersionKey,
    adapter: String,
    capabilities: Capabilities,
}

impl Opened {
    /// The build, as a source of metadata and columns.
    pub fn source(&self) -> &dyn BuildSource {
        self.source.as_ref()
    }

    /// The same, handed over.
    pub fn into_source(self) -> Box<dyn BuildSource> {
        self.source
    }

    /// The version of the file: the key of the adapter that reads it.
    pub fn key(&self) -> &VersionKey {
        &self.key
    }

    /// The adapter in use, by name.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// What this version can read and write.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// The three version numbers and the adapter (BL-AR-08). The package version is the one of the package the application depends on, which is the
    /// facade's to give.
    pub fn report(&self, package_version: impl Into<String>) -> VersionReport {
        VersionReport::new(self.key.to_string(), &self.adapter, package_version)
    }
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("key", &self.key)
            .field("adapter", &self.adapter)
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The marker attributes the registered adapters declare, each once.
fn attributes_of(registry: &Registry) -> Vec<String> {
    let mut names: Vec<String> = registry
        .marker_places()
        .into_iter()
        .map(|place| place.attribute)
        .collect();
    names.sort();
    names.dedup();
    names
}

fn as_strs(names: &[String]) -> Vec<&str> {
    names.iter().map(String::as_str).collect()
}

/// With no adapter registered, no version can be opened, and no marker is looked for.
fn nothing_registered() -> Error {
    Error::new(ErrorKind::UnsupportedVersion {
        found: "(not looked for: no adapter is registered)".to_string(),
        supported: Vec::new(),
    })
}

fn unversioned(registry: &Registry) -> Error {
    let places: Vec<String> = registry
        .marker_places()
        .iter()
        .map(|p| format!("{} on {}", p.attribute, p.mount))
        .collect();
    Error::new(ErrorKind::Unversioned {
        detail: format!(
            "none of the markers the registered versions use is there ({})",
            places.join("; ")
        ),
    })
}

/// Hands the mount to the adapter of `key`. `marker_expected` is true when the version came from a marker, and then the marker must be where this version
/// keeps it; it is false for a version chosen by a probe, whose file has none.
fn hand_over(
    registry: &Registry,
    mount: &Group,
    key: &VersionKey,
    marker_expected: bool,
) -> Result<Opened, Error> {
    let set = resolve(registry, key)?;
    if marker_expected {
        let own = [set.marker().attribute.as_str()];
        if peek_marker(mount, &own)?.as_ref() != Some(key) {
            return Err(corrupt(format!(
                "the version {key} was read from a marker that is not the one the version {key} keeps ({})",
                set.marker().attribute
            )));
        }
    }
    let reader = match set.reader() {
        Surface::Provided(reader) => *reader,
        Surface::Unsupported { reason } => {
            return Err(Error::new(ErrorKind::UnsupportedInVersion {
                version: key.to_string(),
                feature: format!("reading a file ({reason})"),
            }))
        }
    };
    Ok(Opened {
        source: reader(mount)?,
        key: key.clone(),
        adapter: set.name().to_string(),
        capabilities: set.capabilities().clone(),
    })
}

/// Opens the build whose data is under `mount`, whatever the mount is called and wherever it sits: reads the marker, chooses the adapter, hands it the mount.
pub fn open_at(registry: &Registry, mount: &Group) -> Result<Opened, Error> {
    if registry.is_empty() {
        return Err(nothing_registered());
    }
    let attributes = attributes_of(registry);
    match peek_marker(mount, &as_strs(&attributes))? {
        Some(key) => hand_over(registry, mount, &key, true),
        None => Err(unversioned(registry)),
    }
}

/// Opens a file by its path: read-only, with the libhdf5 minimum checked first, the mount found among the root's children by the registered places.
pub fn open(registry: &Registry, path: impl AsRef<Path>) -> Result<Opened, Error> {
    open_in(registry, path.as_ref(), false)
}

/// [`open`], and for a file with no marker an adapter's probe may recognise its version by structure. Only a caller that knows what it is opening should
/// ask for this: a probe is a guess by the adapter that registered it, and it never runs for a file that has a marker.
pub fn open_probing(registry: &Registry, path: impl AsRef<Path>) -> Result<Opened, Error> {
    open_in(registry, path.as_ref(), true)
}

fn open_in(registry: &Registry, path: &Path, probing: bool) -> Result<Opened, Error> {
    let file = open_file(path)?;
    if registry.is_empty() {
        return Err(nothing_registered());
    }
    let root = file
        .group("/")
        .map_err(|e| from_binding("opening the root of a file", e))?;
    let members = root
        .member_names()
        .map_err(|e| from_binding("listing the root of a file", e))?;
    // the mounts that the registered adapters name, with the attributes they look for there
    let places = registry.marker_places();
    let mut mounts: Vec<String> = places.iter().map(|p| p.mount.clone()).collect();
    mounts.dedup();
    let mut marked: Vec<(String, VersionKey, Group)> = Vec::new();
    for mount in mounts {
        if !members.contains(&mount) {
            continue;
        }
        let Ok(group) = root.group(&mount) else {
            continue;
        };
        let attributes: Vec<String> = places
            .iter()
            .filter(|p| p.mount == mount)
            .map(|p| p.attribute.clone())
            .collect();
        if let Some(key) = peek_marker(&group, &as_strs(&attributes))? {
            marked.push((mount, key, group));
        }
    }
    match marked.as_slice() {
        [] => {}
        [(_, key, group)] => return hand_over(registry, group, key, true),
        several => {
            let names: Vec<&str> = several.iter().map(|(m, _, _)| m.as_str()).collect();
            return Err(corrupt(format!(
                "markers on {} mounts ({}): the dispatcher does not choose; open the mount you want with open_at",
                several.len(),
                names.join(", ")
            )));
        }
    }
    if probing {
        if let Some(opened) = probe_pass(registry, &root, &members)? {
            return Ok(opened);
        }
    }
    Err(unversioned(registry))
}

/// For a file with no marker: the adapters that registered a probe look at their own mount. One that matches is the version; two that match are refused.
fn probe_pass(
    registry: &Registry,
    root: &Group,
    members: &[String],
) -> Result<Option<Opened>, Error> {
    let mut matched: Vec<(&AdapterSet, Group)> = Vec::new();
    for set in registry.sets() {
        let Some(probe) = set.probe() else { continue };
        if !members.contains(&set.marker().mount) {
            continue;
        }
        if let Ok(group) = root.group(&set.marker().mount) {
            if probe(&group) {
                matched.push((set, group));
            }
        }
    }
    match matched.as_slice() {
        [] => Ok(None),
        [(set, group)] => hand_over(registry, group, set.key(), false).map(Some),
        several => {
            let keys: Vec<String> = several.iter().map(|(s, _)| s.key().to_string()).collect();
            Err(corrupt(format!(
                "the probes of the versions {} all recognise this file: nothing is chosen",
                keys.join(" and ")
            )))
        }
    }
}
