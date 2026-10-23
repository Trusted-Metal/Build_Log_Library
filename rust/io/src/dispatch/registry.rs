//! The registry and exact-match resolution (IP-04 phase 4C, tasks T04.4 and T04.5, decisions D04-6 and D04-7).
//!
//! * **The registry is data** (BL-VA-04): a map from version to [`AdapterSet`]. Every function takes `&Registry`; production uses [`production`], a lazily
//!   initialised static; tests build their own and inject fake versions without touching global state. [`Registry::supported_versions`] is derived from
//!   the keys and never from a separate list.
//! * **The production registry holds the real versions only.** Each adapter adds one line to [`build_production`]: V17 (IP-13), then V18 (IP-05) and V16
//!   (IP-12). The mock version is registered only by injection and is never in it.
//! * **Exact match** (BL-VA-10): an unknown version is `UnsupportedVersion { found, supported }`; `open` preserves the on-disk version, and nothing upgrades
//!   or downgrades a file; `create(version)` selects the adapter, and a version without a writer answers `UnsupportedInVersion`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use build_log_model::{Error, ErrorKind, Violation};

use super::marker::VersionKey;
use crate::adapters::{AdapterSet, MarkerPlace, Surface};

/// The adapter sets by version.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    sets: BTreeMap<VersionKey, AdapterSet>,
}

impl Registry {
    /// A registry with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a version. A key that is already registered is a `ValidationError`: two sets for one version would make the choice between them a guess.
    pub fn register(&mut self, set: AdapterSet) -> Result<(), Error> {
        if self.sets.contains_key(set.key()) {
            return Err(Error::new(ErrorKind::ValidationError {
                violations: vec![Violation::new(format!(
                    "the version {} is registered twice",
                    set.key()
                ))],
            }));
        }
        self.sets.insert(set.key().clone(), set);
        Ok(())
    }

    /// The same, for building a registry in one expression.
    pub fn with(mut self, set: AdapterSet) -> Result<Self, Error> {
        self.register(set)?;
        Ok(self)
    }

    pub fn get(&self, key: &VersionKey) -> Option<&AdapterSet> {
        self.sets.get(key)
    }

    /// The registered sets, in the order of their versions.
    pub fn sets(&self) -> impl Iterator<Item = &AdapterSet> {
        self.sets.values()
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// The versions that can be opened, derived from the registry's keys, in the order of their versions (`16`, `17`, `18`, `100`).
    pub fn supported_versions(&self) -> Vec<String> {
        self.sets.keys().map(|key| key.to_string()).collect()
    }

    /// The places where the registered adapters keep their marker, each once, in a fixed order. The dispatcher looks at these, and knows no others.
    pub fn marker_places(&self) -> Vec<MarkerPlace> {
        let mut places: Vec<MarkerPlace> = self.sets.values().map(|s| s.marker().clone()).collect();
        places.sort();
        places.dedup();
        places
    }
}

/// The registry of the real versions, built on first use.
pub fn production() -> &'static Registry {
    static PRODUCTION: OnceLock<Registry> = OnceLock::new();
    PRODUCTION.get_or_init(build_production)
}

/// The production list: one line for each version that has an adapter. Adding a version is a folder in `adapters/` and a line here, and nothing else
/// (BL-VA-06); a guard checks that every folder is named here once (phase 4E).
fn build_production() -> Registry {
    let mut registry = Registry::new();
    registry
        .register(crate::adapters::v17::adapter_set())
        .expect("the production list names each version once");
    // IP-05 adds the V18 adapter here and IP-12 the V16 adapter.
    registry
}

/// Finds the adapter set for a version, exactly. An unknown version is `UnsupportedVersion`, which carries what was found and what is supported.
pub fn resolve<'r>(registry: &'r Registry, key: &VersionKey) -> Result<&'r AdapterSet, Error> {
    registry.get(key).ok_or_else(|| {
        Error::new(ErrorKind::UnsupportedVersion {
            found: key.to_string(),
            supported: registry.supported_versions(),
        })
    })
}

/// Finds the adapter set to create a file of a version. An unknown version is `UnsupportedVersion`; a version whose writer is declared unsupported is
/// `UnsupportedInVersion`, with its reason. (Until IP-08 no version can provide a writer, so every registered version answers the second.)
pub fn resolve_for_create<'r>(
    registry: &'r Registry,
    key: &VersionKey,
) -> Result<&'r AdapterSet, Error> {
    let set = resolve(registry, key)?;
    match set.writer() {
        Surface::Provided(never) => match *never {},
        Surface::Unsupported { reason } => Err(Error::new(ErrorKind::UnsupportedInVersion {
            version: key.to_string(),
            feature: format!("creating a file ({reason})"),
        })),
    }
}
