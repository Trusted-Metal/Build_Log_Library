//! The ways V17 stores a value, each read as one value (IP-13 phase 13B, decision D13-2).
//!
//! The files of V17 do not agree on how they write a number or a string: tauri's current output writes a number as an array of one element and text as a
//! variable-length string, the March files write a scalar and a fixed-length string. The primitive says how each attribute is stored, and these functions
//! accept **every form the files show**, so that one reader serves both flavors and no detector is needed.
//!
//! * **A number** may be a scalar or an array of one element, of any width of integer, or `float32` or `float64`. A `float32` is widened exactly.
//! * **Text** may be variable-length or fixed-length. Fixed-length bytes must be valid text and are returned as they are: the primitive has already left out
//!   the zero bytes that fill the string, and no file we hold pads with spaces.
//! * **An attribute the object does not have is `None`**, never a default; an array of no elements is the same as not having it.
//! * **An attribute that holds several values where one belongs is `Corrupt`** (decision D13-15): the first is never taken silently.
//!
//! None of these functions names an attribute: the caller does, from the layout module.

use build_log_model::{Error, ErrorKind};
use hdf5::Location;

use crate::hdf5::{read_attribute, strict_text, Stored, Value};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The attribute as stored, or `None` if the object does not have it.
fn stored(location: &Location, name: &str) -> Result<Option<Stored>, Error> {
    match read_attribute(location, name) {
        Ok(stored) => Ok(Some(stored)),
        Err(error) if matches!(error.kind(), ErrorKind::NotPresent { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn count(value: &Value) -> usize {
    match value {
        Value::I8(v) => v.len(),
        Value::I16(v) => v.len(),
        Value::I32(v) => v.len(),
        Value::I64(v) => v.len(),
        Value::U8(v) => v.len(),
        Value::U16(v) => v.len(),
        Value::U32(v) => v.len(),
        Value::U64(v) => v.len(),
        Value::F32(v) => v.len(),
        Value::F64(v) => v.len(),
        Value::Bytes(v) => v.len(),
        Value::Text(v) => v.len(),
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::U64(_) => "a whole number",
        Value::F32(_) | Value::F64(_) => "a floating-point number",
        Value::Bytes(_) => "fixed-length bytes",
        Value::Text(_) => "text",
    }
}

/// The attribute, if it holds exactly one value: `None` for an attribute that is absent or holds no value, `Corrupt` for one that holds several.
fn single(location: &Location, name: &str) -> Result<Option<Stored>, Error> {
    let Some(stored) = stored(location, name)? else {
        return Ok(None);
    };
    match count(&stored.value) {
        0 => Ok(None),
        1 => Ok(Some(stored)),
        several => Err(corrupt(format!(
            "the attribute `{name}` holds {several} values where one belongs; the first is not taken"
        ))),
    }
}

fn wrong_kind(name: &str, found: &Value, wanted: &str) -> Error {
    corrupt(format!(
        "the attribute `{name}` holds {} where {wanted} belongs",
        describe(found)
    ))
}

/// A whole number, of any width, as a scalar or an array of one element.
pub fn optional_integer(location: &Location, name: &str) -> Result<Option<i64>, Error> {
    let Some(stored) = single(location, name)? else {
        return Ok(None);
    };
    match &stored.value {
        Value::I8(v) => Ok(Some(i64::from(v[0]))),
        Value::I16(v) => Ok(Some(i64::from(v[0]))),
        Value::I32(v) => Ok(Some(i64::from(v[0]))),
        Value::I64(v) => Ok(Some(v[0])),
        Value::U8(v) => Ok(Some(i64::from(v[0]))),
        Value::U16(v) => Ok(Some(i64::from(v[0]))),
        Value::U32(v) => Ok(Some(i64::from(v[0]))),
        Value::U64(v) => i64::try_from(v[0]).map(Some).map_err(|_| {
            Error::new(ErrorKind::OutOfWidth {
                value: v[0].to_string(),
                stored_as: "a signed 64-bit integer".to_string(),
            })
        }),
        other => Err(wrong_kind(name, other, "a whole number")),
    }
}

/// A floating-point number (`float32` is widened exactly), as a scalar or an array of one element.
pub fn optional_float(location: &Location, name: &str) -> Result<Option<f64>, Error> {
    let Some(stored) = single(location, name)? else {
        return Ok(None);
    };
    match &stored.value {
        Value::F32(v) => Ok(Some(f64::from(v[0]))),
        Value::F64(v) => Ok(Some(v[0])),
        other => Err(wrong_kind(name, other, "a floating-point number")),
    }
}

/// Text, variable-length or fixed-length. An empty string is a value (the alloy of the current output): only an attribute with no element is `None`.
pub fn optional_text(location: &Location, name: &str) -> Result<Option<String>, Error> {
    let Some(stored) = single(location, name)? else {
        return Ok(None);
    };
    match &stored.value {
        Value::Text(v) => Ok(Some(v[0].clone())),
        Value::Bytes(v) => strict_text(&v[0])
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| {
                corrupt(format!(
                    "the attribute `{name}` is a fixed-length string whose bytes are not text"
                ))
            }),
        other => Err(wrong_kind(name, other, "text")),
    }
}

/// Text that the object must have: an absent attribute is `Corrupt`.
pub fn required_text(location: &Location, name: &str) -> Result<String, Error> {
    optional_text(location, name)?
        .ok_or_else(|| corrupt(format!("the attribute `{name}` is missing")))
}
