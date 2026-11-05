//! The ways a file stores a value, each read as one value (IP-13 phase 13B, decision D13-2; moved to the toolkit by IP-12 phase 12A, decision D12-1).
//!
//! The files of one version do not always agree on how they write a number or a string: one writer's output has a number as an array of one element and text as a
//! variable-length string, another's has a scalar and a fixed-length string. The primitive says how each attribute is stored, and these functions
//! accept **every form the files show**, so that one reader serves every flavor of a version and no detector is needed.
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

use crate::hdf5::{read_attribute, strict_text, Held, Stored};

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

fn count(value: &Held) -> usize {
    match value {
        Held::I8(v) => v.len(),
        Held::I16(v) => v.len(),
        Held::I32(v) => v.len(),
        Held::I64(v) => v.len(),
        Held::U8(v) => v.len(),
        Held::U16(v) => v.len(),
        Held::U32(v) => v.len(),
        Held::U64(v) => v.len(),
        Held::F32(v) => v.len(),
        Held::F64(v) => v.len(),
        Held::Bytes(v) => v.len(),
        Held::Text(v) => v.len(),
    }
}

fn describe(value: &Held) -> &'static str {
    match value {
        Held::I8(_)
        | Held::I16(_)
        | Held::I32(_)
        | Held::I64(_)
        | Held::U8(_)
        | Held::U16(_)
        | Held::U32(_)
        | Held::U64(_) => "a whole number",
        Held::F32(_) | Held::F64(_) => "a floating-point number",
        Held::Bytes(_) => "fixed-length bytes",
        Held::Text(_) => "text",
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

fn wrong_kind(name: &str, found: &Held, wanted: &str) -> Error {
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
        Held::I8(v) => Ok(Some(i64::from(v[0]))),
        Held::I16(v) => Ok(Some(i64::from(v[0]))),
        Held::I32(v) => Ok(Some(i64::from(v[0]))),
        Held::I64(v) => Ok(Some(v[0])),
        Held::U8(v) => Ok(Some(i64::from(v[0]))),
        Held::U16(v) => Ok(Some(i64::from(v[0]))),
        Held::U32(v) => Ok(Some(i64::from(v[0]))),
        Held::U64(v) => i64::try_from(v[0]).map(Some).map_err(|_| {
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
        Held::F32(v) => Ok(Some(f64::from(v[0]))),
        Held::F64(v) => Ok(Some(v[0])),
        other => Err(wrong_kind(name, other, "a floating-point number")),
    }
}

/// A number, whether the file stored it as an integer or as a float (`float32` is widened exactly): the 2026-10-15 file has `int32` where the current output has
/// `float32` for the same value. A whole number that a `float64` cannot hold exactly is `OutOfWidth`, because it would not be the number the file holds.
pub fn optional_number(location: &Location, name: &str) -> Result<Option<f64>, Error> {
    let Some(stored) = single(location, name)? else {
        return Ok(None);
    };
    let out_of_width = |shown: String| {
        Error::new(ErrorKind::OutOfWidth {
            value: shown,
            stored_as: "a 64-bit floating-point number".to_string(),
        })
    };
    let whole = |value: i128| {
        let number = value as f64;
        if number as i128 == value && value.unsigned_abs() <= (1u128 << 53) {
            Ok(Some(number))
        } else {
            Err(out_of_width(value.to_string()))
        }
    };
    match &stored.value {
        Held::F32(v) => Ok(Some(f64::from(v[0]))),
        Held::F64(v) => Ok(Some(v[0])),
        Held::I8(v) => whole(i128::from(v[0])),
        Held::I16(v) => whole(i128::from(v[0])),
        Held::I32(v) => whole(i128::from(v[0])),
        Held::I64(v) => whole(i128::from(v[0])),
        Held::U8(v) => whole(i128::from(v[0])),
        Held::U16(v) => whole(i128::from(v[0])),
        Held::U32(v) => whole(i128::from(v[0])),
        Held::U64(v) => whole(i128::from(v[0])),
        other => Err(wrong_kind(name, other, "a number")),
    }
}

/// What an attribute holds that may be a number or text: a number as a number and text as text. Nothing is decided from the text.
pub enum Written {
    Number(f64),
    Text(String),
}

/// An attribute that is either a number or text, as it is written.
pub fn optional_written(location: &Location, name: &str) -> Result<Option<Written>, Error> {
    let Some(stored) = single(location, name)? else {
        return Ok(None);
    };
    match &stored.value {
        Held::Text(_) | Held::Bytes(_) => Ok(optional_text(location, name)?.map(Written::Text)),
        _ => Ok(optional_number(location, name)?.map(Written::Number)),
    }
}

/// Text, variable-length or fixed-length. An empty string is a value (the alloy of the current output): only an attribute with no element is `None`.
pub fn optional_text(location: &Location, name: &str) -> Result<Option<String>, Error> {
    let Some(stored) = single(location, name)? else {
        return Ok(None);
    };
    match &stored.value {
        Held::Text(v) => Ok(Some(v[0].clone())),
        Held::Bytes(v) => strict_text(&v[0])
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
