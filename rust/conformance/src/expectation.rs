//! The expectation files of `testdata/expectations`, read.
//!
//! An expectation file says what the CDM must show for one miniature. It is written by an independent script from the miniature alone (IP-02 phase 2K), so
//! that a mistake in an adapter is not also a mistake in what it is checked against.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The version of the expectation files that this crate reads.
pub const EXPECTATIONS_VERSION: i64 = 1;

/// One expectation file, parsed.
#[derive(Debug, Clone)]
pub struct Expectation {
    root: Value,
}

impl Expectation {
    /// Parses the text of an expectation file.
    pub fn parse(text: &str) -> Result<Expectation, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        match root.get("expectations_version").and_then(Value::as_i64) {
            Some(EXPECTATIONS_VERSION) => Ok(Expectation { root }),
            other => Err(format!(
                "expectations_version is {other:?}, and this crate reads version {EXPECTATIONS_VERSION}"
            )),
        }
    }

    pub fn load(path: &Path) -> Result<Expectation, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Expectation::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The name of the miniature this file is about, such as `v17_mock.h5`.
    pub fn miniature(&self) -> &str {
        self.root["miniature"]["file"].as_str().unwrap_or("")
    }

    pub(crate) fn root(&self) -> &Value {
        &self.root
    }
}

/// `testdata/expectations` of this repository.
pub fn expectations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("testdata")
        .join("expectations")
}

/// Every expectation file of the repository, by file name.
pub fn load_all() -> Result<Vec<Expectation>, String> {
    let dir = expectations_dir();
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    paths.iter().map(|path| Expectation::load(path)).collect()
}

// ---------------------------------------------------------------- reading values out of the JSON, with an error that says where

pub(crate) fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("the expectation has no `{key}`"))
}

pub(crate) fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    field(value, key)?
        .as_str()
        .ok_or_else(|| format!("`{key}` is not text"))
}

pub(crate) fn opt_text(value: &Value, key: &str) -> Result<Option<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("`{key}` is not text or null")),
    }
}

pub(crate) fn int(value: &Value, key: &str) -> Result<i64, String> {
    field(value, key)?
        .as_i64()
        .ok_or_else(|| format!("`{key}` is not an integer"))
}

pub(crate) fn opt_int(value: &Value, key: &str) -> Result<Option<i64>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("`{key}` is not an integer or null")),
    }
}

pub(crate) fn float(value: &Value, key: &str) -> Result<f64, String> {
    field(value, key)?
        .as_f64()
        .ok_or_else(|| format!("`{key}` is not a number"))
}

pub(crate) fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    field(value, key)?
        .as_array()
        .ok_or_else(|| format!("`{key}` is not a list"))
}
