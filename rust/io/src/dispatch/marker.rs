//! The marker reader (IP-04 phase 4B, task T04.2, decision D04-2): the version of a mount, in every form that the files hold.
//!
//! * **A version is a string** (decision G3), matched exactly. An integer marker (of any width, a scalar or an array of one element) gives its decimal;
//!   a text marker gives its text, trimmed of spaces and zero bytes. So `17` stored as an `int32` scalar, as an `int32` array of one and as the text `"17"`
//!   all give the key `17`, and `1.0` is a key too (an unsupported one, for the registry to say).
//! * **A marker that cannot be a version is `Corrupt`:** empty text, a float, an array of more than one element, or a form that is not a number or text.
//!   The reader does not guess.
//! * **It knows no name.** The attribute names to look for are an argument: they are the data that each adapter set declares (decision D04-3).

use std::cmp::Ordering;
use std::fmt;

use build_log_model::{Error, ErrorKind};
use hdf5::Location;

use crate::hdf5::{read_attribute, Stored, Value};

/// The version of a file, as the adapters are keyed: a short text such as `17`. Compared exactly; **sorted by length and then by text**, so that digits sort
/// as numbers (`16`, `17`, `18`, `100`, `9001`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionKey(String);

impl VersionKey {
    /// A key from text: trimmed of spaces and zero bytes, and not empty, and with no control character inside.
    pub fn new(text: &str) -> Result<Self, Error> {
        let trimmed = text.trim_matches(|c: char| c == ' ' || c == '\0');
        if trimmed.is_empty() {
            return Err(corrupt("a version marker that is empty".to_string()));
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(corrupt(format!(
                "a version marker with a control character in it: {trimmed:?}"
            )));
        }
        Ok(VersionKey(trimmed.to_string()))
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialOrd for VersionKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionKey {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0.len(), &self.0).cmp(&(other.0.len(), &other.0))
    }
}

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The key that an attribute's value gives, or why it cannot be one.
fn key_of(name: &str, stored: &Stored) -> Result<VersionKey, Error> {
    let not_a_version =
        |why: &str| corrupt(format!("the marker `{name}` cannot be a version: {why}"));
    macro_rules! one_integer {
        ($v:expr) => {
            match $v.as_slice() {
                [only] => VersionKey::new(&only.to_string()),
                other => Err(not_a_version(&format!(
                    "it holds {} values, not one",
                    other.len()
                ))),
            }
        };
    }
    match &stored.value {
        Value::I8(v) => one_integer!(v),
        Value::I16(v) => one_integer!(v),
        Value::I32(v) => one_integer!(v),
        Value::I64(v) => one_integer!(v),
        Value::U8(v) => one_integer!(v),
        Value::U16(v) => one_integer!(v),
        Value::U32(v) => one_integer!(v),
        Value::U64(v) => one_integer!(v),
        Value::Text(v) => match v.as_slice() {
            [only] => VersionKey::new(only),
            other => Err(not_a_version(&format!(
                "it holds {} texts, not one",
                other.len()
            ))),
        },
        Value::Bytes(v) => match v.as_slice() {
            [only] => match std::str::from_utf8(only) {
                Ok(text) => VersionKey::new(text),
                Err(_) => Err(not_a_version("its bytes are not text")),
            },
            other => Err(not_a_version(&format!(
                "it holds {} texts, not one",
                other.len()
            ))),
        },
        Value::F32(_) | Value::F64(_) => Err(not_a_version("it is a floating-point number")),
    }
}

/// Reads the version marker of a mount.
///
/// `attributes` are the names to look for, one for each place that a registered adapter declares. Returns `Ok(None)` if the mount has none of them, the key if
/// it has one (or several that agree), and `Corrupt` if one cannot be a version or two disagree.
pub fn peek_marker(mount: &Location, attributes: &[&str]) -> Result<Option<VersionKey>, Error> {
    let mut found: Vec<(&str, VersionKey)> = Vec::new();
    for name in attributes {
        match read_attribute(mount, name) {
            Ok(stored) => found.push((name, key_of(name, &stored)?)),
            Err(e) if matches!(e.kind(), ErrorKind::NotPresent { .. }) => {}
            Err(e) if matches!(e.kind(), ErrorKind::UnsupportedInVersion { .. }) => {
                return Err(corrupt(format!(
                    "the marker `{name}` is stored in a form that cannot be a version: {e}"
                )))
            }
            Err(e) => return Err(e),
        }
    }
    let Some((_, first)) = found.first() else {
        return Ok(None);
    };
    if let Some((other, key)) = found.iter().find(|(_, k)| k != first) {
        return Err(corrupt(format!(
            "the markers `{}` and `{other}` give different versions ({first} and {key})",
            found[0].0
        )));
    }
    Ok(Some(first.clone()))
}
