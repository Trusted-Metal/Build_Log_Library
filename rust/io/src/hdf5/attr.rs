//! The attribute reader (IP-03 phase 3D, task T03.2): an attribute is read **with the encoding it is stored in**.
//!
//! * **Exact.** An integer keeps its width and sign, a `float32` stays a `float32`, a scalar is not turned into an array of one, and a fixed-length string is
//!   returned as bytes and never decoded (decision D03-3). What a value means is the adapter's business.
//! * **The way each kind is read** is the result of the method check (decision D03-4): the binding's typed API reads every kind of the eight miniatures.
//!   Its one limit is a fixed-length string whose bytes are not text: the safe binding stops at the first zero byte and cannot say how the string is padded
//!   (see [`Value::Bytes`] and [`Padding::Unknown`]). Question Q3-1, the scoped use of `unsafe` that would lift it, is deferred to the V17 March step.
//! * **What it does not read** is named, not guessed: an attribute of a class that is not an integer, a float, a string or the array of bytes of the image
//!   specification is an `UnsupportedInVersion` error that names the class.
//!
//! [`class_of`] is shared with the dataset descriptors (phase 3E): the class of a datatype is the same whether an attribute or a dataset has it.

use build_log_model::Error;
use hdf5::datatype::ByteOrder as BindingOrder;
use hdf5::types::{
    FixedAscii, FixedUnicode, FloatSize, IntSize, TypeDescriptor, VarLenAscii, VarLenUnicode,
};
use hdf5::{Attribute, Datatype, Location};

use super::error::{from_binding, not_present, unsupported};
use super::value::{
    ByteOrder, Charset, Class, Encoding, Field, Padding, Shape, Stored, TextForm, TextLength, Value,
};

/// Reads the attribute `name` of a group or dataset, with its encoding.
///
/// An attribute that the object does not have is a `NotPresent` error.
pub fn read_attribute(location: &Location, name: &str) -> Result<Stored, Error> {
    let names = location
        .attr_names()
        .map_err(|e| from_binding("listing the attributes of an object", e))?;
    if !names.iter().any(|n| n == name) {
        return Err(not_present(format!("the attribute `{name}`")));
    }
    let attr = location
        .attr(name)
        .map_err(|e| from_binding("opening an attribute", e))?;
    let datatype = attr
        .dtype()
        .map_err(|e| from_binding("reading the datatype of an attribute", e))?;
    let descriptor = datatype.to_descriptor().map_err(|e| {
        unsupported(format!(
            "an attribute of a datatype class that is not read ({e})"
        ))
    })?;
    let shape = if attr.is_scalar() {
        Shape::Scalar
    } else {
        Shape::Array(attr.shape())
    };
    let class = class_of(&datatype, &descriptor)?;
    let value = read_value(&attr, &descriptor)?;
    Ok(Stored {
        encoding: Encoding { class, shape },
        value,
    })
}

/// The byte order of a number, which has to be little or big endian.
fn byte_order(datatype: &Datatype) -> Result<ByteOrder, Error> {
    match datatype.byte_order() {
        BindingOrder::LittleEndian => Ok(ByteOrder::Little),
        BindingOrder::BigEndian => Ok(ByteOrder::Big),
        other => Err(unsupported(format!(
            "a number stored in the byte order {other:?}"
        ))),
    }
}

/// The byte order that the binding gives a type made of members (a compound or an array), which has no order of its own: little or big endian if it says
/// so, otherwise little endian, as HDF5 reports for a one-byte integer. The binding does not show the order of each member.
fn order_hint(datatype: &Datatype) -> ByteOrder {
    byte_order(datatype).unwrap_or(ByteOrder::Little)
}

/// The class of a datatype: what kind of value it holds, with its width and form. The same for an attribute and for a dataset.
///
/// A datatype that this library has no class for is an `UnsupportedInVersion` error that names it.
pub(crate) fn class_of(datatype: &Datatype, descriptor: &TypeDescriptor) -> Result<Class, Error> {
    match descriptor {
        TypeDescriptor::Integer(size) => Ok(Class::Integer {
            bits: bits_of(*size),
            signed: true,
            order: byte_order(datatype)?,
        }),
        TypeDescriptor::Unsigned(size) => Ok(Class::Integer {
            bits: bits_of(*size),
            signed: false,
            order: byte_order(datatype)?,
        }),
        TypeDescriptor::Float(size) => Ok(Class::Float {
            bits: match size {
                FloatSize::U4 => 32,
                FloatSize::U8 => 64,
            },
            order: byte_order(datatype)?,
        }),
        TypeDescriptor::VarLenUnicode => Ok(variable_text(Charset::Utf8)),
        TypeDescriptor::VarLenAscii => Ok(variable_text(Charset::Ascii)),
        TypeDescriptor::FixedAscii(bytes) => Ok(fixed_text(
            *bytes,
            Charset::Ascii,
            is_null_padded(datatype, descriptor),
        )),
        TypeDescriptor::FixedUnicode(bytes) => Ok(fixed_text(
            *bytes,
            Charset::Utf8,
            is_null_padded(datatype, descriptor),
        )),
        TypeDescriptor::FixedArray(base, len) => Ok(Class::Array {
            dims: vec![*len],
            base: Box::new(member_class(base, order_hint(datatype), false)?),
        }),
        TypeDescriptor::Compound(compound) => {
            // the strings of a compound are null-padded if the binding's own compound of the same members equals this one
            let null_padded = is_null_padded(datatype, descriptor);
            let order = order_hint(datatype);
            let fields = compound
                .fields
                .iter()
                .map(|f| {
                    Ok(Field {
                        name: f.name.clone(),
                        class: member_class(&f.ty, order, null_padded)?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            Ok(Class::Compound { fields })
        }
        other => Err(unsupported(format!("a datatype of the type {other:?}"))),
    }
}

/// The class of a member of a compound or an array, whose byte order and padding the binding does not show one by one.
fn member_class(
    descriptor: &TypeDescriptor,
    order: ByteOrder,
    null_padded: bool,
) -> Result<Class, Error> {
    match descriptor {
        TypeDescriptor::Integer(size) => Ok(Class::Integer {
            bits: bits_of(*size),
            signed: true,
            order,
        }),
        TypeDescriptor::Unsigned(size) => Ok(Class::Integer {
            bits: bits_of(*size),
            signed: false,
            order,
        }),
        TypeDescriptor::Float(size) => Ok(Class::Float {
            bits: match size {
                FloatSize::U4 => 32,
                FloatSize::U8 => 64,
            },
            order,
        }),
        TypeDescriptor::VarLenUnicode => Ok(variable_text(Charset::Utf8)),
        TypeDescriptor::VarLenAscii => Ok(variable_text(Charset::Ascii)),
        TypeDescriptor::FixedAscii(bytes) => Ok(fixed_text(*bytes, Charset::Ascii, null_padded)),
        TypeDescriptor::FixedUnicode(bytes) => Ok(fixed_text(*bytes, Charset::Utf8, null_padded)),
        other => Err(unsupported(format!("a member of the type {other:?}"))),
    }
}

fn read_value(attr: &Attribute, descriptor: &TypeDescriptor) -> Result<Value, Error> {
    macro_rules! numbers {
        ($t:ty, $variant:ident) => {
            attr.read_raw::<$t>()
                .map(Value::$variant)
                .map_err(|e| from_binding("reading an attribute", e))
        };
    }
    match descriptor {
        TypeDescriptor::Integer(size) => match size {
            IntSize::U1 => numbers!(i8, I8),
            IntSize::U2 => numbers!(i16, I16),
            IntSize::U4 => numbers!(i32, I32),
            IntSize::U8 => numbers!(i64, I64),
        },
        TypeDescriptor::Unsigned(size) => match size {
            IntSize::U1 => numbers!(u8, U8),
            IntSize::U2 => numbers!(u16, U16),
            IntSize::U4 => numbers!(u32, U32),
            IntSize::U8 => numbers!(u64, U64),
        },
        TypeDescriptor::Float(size) => match size {
            FloatSize::U4 => numbers!(f32, F32),
            FloatSize::U8 => numbers!(f64, F64),
        },
        TypeDescriptor::VarLenUnicode => {
            let values = attr
                .read_raw::<VarLenUnicode>()
                .map_err(|e| from_binding("reading an attribute", e))?;
            Ok(Value::Text(
                values.iter().map(|s| s.as_str().to_string()).collect(),
            ))
        }
        TypeDescriptor::VarLenAscii => {
            let values = attr
                .read_raw::<VarLenAscii>()
                .map_err(|e| from_binding("reading an attribute", e))?;
            Ok(Value::Text(
                values.iter().map(|s| s.as_str().to_string()).collect(),
            ))
        }
        TypeDescriptor::FixedAscii(bytes) => read_fixed(attr, *bytes, false),
        TypeDescriptor::FixedUnicode(bytes) => read_fixed(attr, *bytes, true),
        TypeDescriptor::FixedArray(base, len) => match (&**base, *len) {
            (TypeDescriptor::Unsigned(IntSize::U1), 64) => {
                let elements = attr
                    .read_raw::<[u8; 64]>()
                    .map_err(|e| from_binding("reading an attribute", e))?;
                Ok(Value::U8(elements.into_iter().flatten().collect()))
            }
            (other, len) => Err(unsupported(format!(
                "an attribute that is an array of {len} elements of {other:?}"
            ))),
        },
        other => Err(unsupported(format!("an attribute of the type {other:?}"))),
    }
}

fn bits_of(size: IntSize) -> u8 {
    match size {
        IntSize::U1 => 8,
        IntSize::U2 => 16,
        IntSize::U4 => 32,
        IntSize::U8 => 64,
    }
}

fn variable_text(charset: Charset) -> Class {
    Class::Text(TextForm {
        charset,
        length: TextLength::Variable,
    })
}

/// Is a datatype equal to the binding's own null-padded type for the same descriptor? The binding builds fixed-length strings null-padded and has no call
/// that says how an existing one is padded, so this is all it can tell (see [`Padding::Unknown`]).
fn is_null_padded(datatype: &Datatype, descriptor: &TypeDescriptor) -> bool {
    Datatype::from_descriptor(descriptor)
        .map(|candidate| &candidate == datatype)
        .unwrap_or(false)
}

/// The class of a fixed-length string. The binding can say whether it is null-padded and nothing more about the padding.
fn fixed_text(bytes: usize, charset: Charset, null_padded: bool) -> Class {
    Class::Text(TextForm {
        charset,
        length: TextLength::Fixed {
            bytes,
            padding: if null_padded {
                Padding::NullPadded
            } else {
                Padding::Unknown
            },
        },
    })
}

/// Reads a fixed-length string through the typed API, into a type of the smallest capacity that holds it: the file's string is converted into that type,
/// and each element is returned as its bytes up to the first zero byte.
fn read_fixed(attr: &Attribute, bytes: usize, unicode: bool) -> Result<Value, Error> {
    macro_rules! capacity {
        ($cap:literal) => {
            if unicode {
                attr.read_raw::<FixedUnicode<$cap>>()
                    .map(|v| v.iter().map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>())
            } else {
                attr.read_raw::<FixedAscii<$cap>>()
                    .map(|v| v.iter().map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>())
            }
        };
    }
    let elements = match bytes {
        0..=16 => capacity!(16),
        17..=64 => capacity!(64),
        65..=256 => capacity!(256),
        257..=1024 => capacity!(1024),
        _ => {
            return Err(unsupported(format!(
                "a fixed-length string of {bytes} bytes (the limit is 1,024)"
            )))
        }
    }
    .map_err(|e| from_binding("reading an attribute", e))?;
    Ok(Value::Bytes(elements))
}
