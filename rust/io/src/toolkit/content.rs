//! What a file's own content says about a failure and about a type (IP-12 phase 12A; from the laser reader of IP-13 phase 13D, decision D13-21).

use build_log_model::{Error, ErrorKind, NumericType};

use crate::hdf5::Class;

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// Can this failure be reported on the laser or the channel it happened in? A file that is not what its version says it is can; damage to the file itself cannot.
pub fn is_about_the_content(error: &Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Corrupt { .. }
            | ErrorKind::OutOfWidth { .. }
            | ErrorKind::UnsupportedInVersion { .. }
    )
}

/// The numeric type that a class is, if it is one.
pub fn numeric_type(class: &Class) -> Result<NumericType, Error> {
    match class {
        Class::Float { bits: 32, .. } => Ok(NumericType::F32),
        Class::Float { bits: 64, .. } => Ok(NumericType::F64),
        Class::Integer {
            bits: 8,
            signed: true,
            ..
        } => Ok(NumericType::I8),
        Class::Integer {
            bits: 16,
            signed: true,
            ..
        } => Ok(NumericType::I16),
        Class::Integer {
            bits: 32,
            signed: true,
            ..
        } => Ok(NumericType::I32),
        Class::Integer {
            bits: 64,
            signed: true,
            ..
        } => Ok(NumericType::I64),
        Class::Integer {
            bits: 8,
            signed: false,
            ..
        } => Ok(NumericType::U8),
        Class::Integer {
            bits: 16,
            signed: false,
            ..
        } => Ok(NumericType::U16),
        Class::Integer {
            bits: 32,
            signed: false,
            ..
        } => Ok(NumericType::U32),
        Class::Integer {
            bits: 64,
            signed: false,
            ..
        } => Ok(NumericType::U64),
        other => Err(corrupt(format!(
            "a channel stored as {other:?}, which is not a number"
        ))),
    }
}
