//! `build-log-conformance`: what the Common Data Model must show for each committed miniature, checked against the independent expectation files.
//!
//! * **Layer:** a development-only crate beside the four layers. It is not part of the library that consumers depend on.
//! * **May depend on:** `build-log-model` only (and `serde_json`, to read the expectation files). Not the IO layer, not Processing, not the facade and not the test kit.
//! * **Who depends on it:** the IO layer, Processing and the facade, as a **dev**-dependency, to hold each adapter's output to the same facts (IP-05, IP-12, IP-13).
//!
//! # What it holds
//!
//! * [`Expectation`]: one file of `testdata/expectations`, written from a miniature alone by an independent script (IP-02 phase 2K).
//! * [`check`]: every way in which a [`BuildMeta`](build_log_model::BuildMeta) does not conform to an expectation, as lines that say what was found and what was expected.
//!   [`check_parts`] does the same for the [`Part`]s named, so that an adapter built in steps asserts what it implements so far; the parts together are the whole.
//! * [`synthesise`]: a `BuildMeta` made straight from an expectation. Until adapters exist it is the only producer of a CDM, and it is how the assertions are tested: a
//!   CDM built from an expectation passes them, and the same CDM with one thing changed fails them.
//!
//! The assertions say *what the CDM must show*, never how an adapter reads a file, so one adapter per version is held to the same facts.

#![forbid(unsafe_code)]

mod check;
mod expectation;
mod names;
mod synthesise;

pub use check::{check, check_parts, Part};
pub use expectation::{expectations_dir, load_all, Expectation, EXPECTATIONS_VERSION};
pub use synthesise::synthesise;
