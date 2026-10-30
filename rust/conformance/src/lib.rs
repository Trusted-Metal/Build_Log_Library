//! `build-log-conformance`: what the Common Data Model must show for each committed miniature, checked against the independent expectation files.
//!
//! * **Layer:** a development-only crate beside the four layers. It is not part of the library that consumers depend on.
//! * **May depend on:** `build-log-model` only (and `serde_json`, to read the expectation files, and `sha2`, to fingerprint the columns it reads). Not the IO layer, not Processing, not the facade and not the test kit.
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
//! * [`check_columns`]: every way in which a [`ColumnSource`](build_log_model::ColumnSource) serves a stored column that is not what its **fingerprint** in the expectation says (the count, the
//!   stored type, the extremes, the ends and the SHA-256 of the values), so that the samples themselves stay out of the expectation files (IP-06 phase 6A).
//!
//! * [`check_event_columns`]: the same for the **events**: every way in which a source serves the columns of a place of an OPC UA series (its times, its values as text, a task's start and
//!   stop) that is not what the **fingerprint of that place** says, and every way in which the typed view disagrees with the type the series declares (IP-06 phase 6F).
//!
//! * [`check_image_regions`]: the same for the **camera frames**: every way in which a source serves a frame (its dimensions and the **fingerprint of its pixels**, taken row by row) that is not
//!   what its fingerprint says, and every way in which a rectangle of it is not the same rectangle of the whole frame, so that a rows-and-columns mix-up is found (IP-06 phase 6G).
//!
//! The assertions say *what the CDM must show*, never how an adapter reads a file, so one adapter per version is held to the same facts.

#![forbid(unsafe_code)]

mod check;
mod columns;
mod event_columns;
mod expectation;
mod image_regions;
mod names;
mod parameters;
mod reports;
mod synthesise;

pub use check::{check, check_parts, Part};
pub use columns::{check_columns, column_expectations, ColumnExpectation, Fingerprint};
pub use event_columns::{
    check_event_columns, event_expectations, EventExpectation, EventFingerprint,
};
pub use expectation::{expectations_dir, load_all, Expectation, EXPECTATIONS_VERSION};
pub use image_regions::{check_image_regions, frame_expectations, FrameExpectation};
pub use reports::{check_availability, check_validation};
pub use synthesise::synthesise;
