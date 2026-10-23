//! The registration unit of one version (IP-04 phase 4C, tasks T04.3, decisions D04-3 to D04-5).
//!
//! An [`AdapterSet`] is everything the registry needs to know about a version, **in one value, so that the surfaces cannot drift apart** (BL-VA-03): the
//! key, the place of the marker, the capabilities, and the reader, the writer and the patcher, each of which is **provided or declared unsupported with a
//! reason**. Its builder refuses to build while a surface is left undeclared.
//!
//! Only the reader can be provided in this plan: the writer and the patcher are typed `Surface<Infallible>` for now, which has no provided value, so a
//! version can only declare them unsupported until IP-08 and IP-11 give them a constructor type. A version that never has a writer (V16) declares it
//! unsupported for good.

use std::convert::Infallible;

use build_log_model::{BuildSource, Capabilities, Error, ErrorKind, Violation};
use hdf5::Group;

use crate::dispatch::VersionKey;

/// A reader: given the mount of a file (the group that holds the library's data), the build as a source of metadata and columns. The mount is a group, so
/// that the adapter can open the groups and datasets below it: **every path it reads is relative to the mount** (decision D04-9).
pub type ReaderFn = fn(&Group) -> Result<Box<dyn BuildSource>, Error>;

/// A probe: does the structure under a mount look like this version? It runs only when the caller asks for it (decision D04-8), for a file that has no
/// marker.
pub type ProbeFn = fn(&Group) -> bool;

/// Where the version marker is: the group that is the mount, and the attribute on it. This is layout knowledge, and so it is the adapter's data and not the
/// dispatcher's code (decision D04-3).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MarkerPlace {
    /// The name of the mount, a child of the file's root.
    pub mount: String,
    /// The name of the attribute on the mount.
    pub attribute: String,
}

impl MarkerPlace {
    pub fn new(mount: impl Into<String>, attribute: impl Into<String>) -> Self {
        MarkerPlace {
            mount: mount.into(),
            attribute: attribute.into(),
        }
    }
}

/// One surface of a version: provided, or declared unsupported with the reason.
#[derive(Debug, Clone)]
pub enum Surface<T> {
    Provided(T),
    Unsupported { reason: String },
}

impl<T> Surface<T> {
    pub fn is_provided(&self) -> bool {
        matches!(self, Surface::Provided(_))
    }

    /// The reason it is unsupported; `None` if it is provided.
    pub fn unsupported_reason(&self) -> Option<&str> {
        match self {
            Surface::Provided(_) => None,
            Surface::Unsupported { reason } => Some(reason),
        }
    }
}

/// Everything the registry needs to know about one version.
#[derive(Debug, Clone)]
pub struct AdapterSet {
    key: VersionKey,
    name: String,
    marker: MarkerPlace,
    capabilities: Capabilities,
    reader: Surface<ReaderFn>,
    writer: Surface<Infallible>,
    patcher: Surface<Infallible>,
    probe: Option<ProbeFn>,
}

impl AdapterSet {
    /// Starts a set for the version `key`, named `name` in reports, whose marker is at `marker`.
    pub fn builder(
        key: impl Into<String>,
        name: impl Into<String>,
        marker: MarkerPlace,
    ) -> AdapterSetBuilder {
        AdapterSetBuilder {
            key: key.into(),
            name: name.into(),
            marker,
            capabilities: Capabilities::new(),
            reader: None,
            writer: None,
            patcher: None,
            probe: None,
        }
    }

    pub fn key(&self) -> &VersionKey {
        &self.key
    }

    /// The adapter's name, as a version report gives it.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn marker(&self) -> &MarkerPlace {
        &self.marker
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn reader(&self) -> &Surface<ReaderFn> {
        &self.reader
    }

    pub fn writer(&self) -> &Surface<Infallible> {
        &self.writer
    }

    pub fn patcher(&self) -> &Surface<Infallible> {
        &self.patcher
    }

    pub fn probe(&self) -> Option<ProbeFn> {
        self.probe
    }
}

/// Builds an [`AdapterSet`]. `build` refuses while any of the reader, the writer and the patcher is neither provided nor declared unsupported.
#[derive(Debug, Clone)]
pub struct AdapterSetBuilder {
    key: String,
    name: String,
    marker: MarkerPlace,
    capabilities: Capabilities,
    reader: Option<Surface<ReaderFn>>,
    writer: Option<Surface<Infallible>>,
    patcher: Option<Surface<Infallible>>,
    probe: Option<ProbeFn>,
}

impl AdapterSetBuilder {
    pub fn capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The version has a reader, this one.
    pub fn reader(mut self, reader: ReaderFn) -> Self {
        self.reader = Some(Surface::Provided(reader));
        self
    }

    pub fn without_reader(mut self, reason: impl Into<String>) -> Self {
        self.reader = Some(Surface::Unsupported {
            reason: reason.into(),
        });
        self
    }

    /// The version has no writer, for this reason. (A writer can be provided from IP-08 on.)
    pub fn without_writer(mut self, reason: impl Into<String>) -> Self {
        self.writer = Some(Surface::Unsupported {
            reason: reason.into(),
        });
        self
    }

    /// The version has no patcher, for this reason. (A patcher can be provided from IP-11 on.)
    pub fn without_patcher(mut self, reason: impl Into<String>) -> Self {
        self.patcher = Some(Surface::Unsupported {
            reason: reason.into(),
        });
        self
    }

    /// A probe for files of this version that have no marker (the legacy family registers one).
    pub fn probe(mut self, probe: ProbeFn) -> Self {
        self.probe = Some(probe);
        self
    }

    /// The set, or a `ValidationError` listing **every** problem: an invalid key, an empty name or marker place, a surface left undeclared, a reason that
    /// says nothing.
    pub fn build(self) -> Result<AdapterSet, Error> {
        let mut violations = Vec::new();
        let key = match VersionKey::new(&self.key) {
            Ok(key) => Some(key),
            Err(e) => {
                violations.push(Violation::new(format!(
                    "the key {:?} is not a version: {e}",
                    self.key
                )));
                None
            }
        };
        if self.name.trim().is_empty() {
            violations.push(Violation::new("the adapter has no name"));
        }
        if self.marker.mount.trim().is_empty() || self.marker.attribute.trim().is_empty() {
            violations.push(Violation::new(
                "the place of the marker is not complete: it needs a mount and an attribute",
            ));
        }
        fn declared<T>(surface: &Option<Surface<T>>, what: &str, violations: &mut Vec<Violation>) {
            match surface {
                None => violations.push(Violation::new(format!(
                    "the {what} is not declared: provide it, or declare it unsupported with a reason"
                ))),
                Some(Surface::Unsupported { reason }) if reason.trim().is_empty() => {
                    violations.push(Violation::new(format!(
                        "the {what} is declared unsupported without a reason"
                    )))
                }
                Some(_) => {}
            }
        }
        declared(&self.reader, "reader", &mut violations);
        declared(&self.writer, "writer", &mut violations);
        declared(&self.patcher, "patcher", &mut violations);
        match (key, self.reader, self.writer, self.patcher) {
            (Some(key), Some(reader), Some(writer), Some(patcher)) if violations.is_empty() => {
                Ok(AdapterSet {
                    key,
                    name: self.name,
                    marker: self.marker,
                    capabilities: self.capabilities,
                    reader,
                    writer,
                    patcher,
                    probe: self.probe,
                })
            }
            _ => Err(Error::new(ErrorKind::ValidationError { violations })),
        }
    }
}
