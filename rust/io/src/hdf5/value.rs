//! A value and the encoding it is stored in (IP-03 decisions D03-2 and D03-3).
//!
//! * **The two travel together** ([`Stored`]), so that an adapter that keeps a value can write it back as it was: a fixed string of five bytes stays a
//!   fixed string of five bytes, a `uint64` is not squeezed into an `i64`, and a scalar is not turned into an array of one (BL-ST-06, BL-ST-07).
//! * **A read is exact and never decides what a value means.** A fixed-length string is returned as its raw bytes, padding included: whether those bytes
//!   are text or the eight bytes of a `float64` (as the V17 March files hold them) is the knowledge of an adapter of that version (D03-3).
//! * **Provisional until the attribute reader (3D) and the spike (3B):** the shapes here follow the kinds of Appendix B of the IP-03 plan, measured on the
//!   eight miniatures, and the oracle `testdata/primitives.json` (phase 3A) records one instance of each.

/// The byte order of a stored number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ByteOrder {
    Little,
    Big,
}

/// The character set that a stored string declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Charset {
    Ascii,
    Utf8,
}

/// How a fixed-length string is filled up to its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Padding {
    /// The value ends at the first zero byte; the zero bytes after it are filling.
    NullTerminated,
    /// The value is followed by zero bytes up to the length.
    NullPadded,
    /// The value is followed by spaces up to the length.
    SpacePadded,
    /// Not known. The safe binding says only whether a string is null-padded; it has no call for the other fillings, so a string that is not null-padded is
    /// reported as this until the scoped shim of question Q3-1 is allowed (deferred to the V17 March step, 2026-10-22). The eight miniatures hold
    /// null-terminated strings and null-padded ones, and no other.
    Unknown,
}

/// Whether a stored string has a length of its own or a length for every value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextLength {
    /// Each value has its own length.
    Variable,
    /// Every value takes `bytes` bytes, filled up as `padding` says.
    Fixed { bytes: usize, padding: Padding },
}

/// How a stored string is written: its character set and its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextForm {
    pub charset: Charset,
    pub length: TextLength,
}

/// One named member of a compound type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub class: Class,
}

/// What kind of value is stored, with its width and form.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Class {
    Integer {
        bits: u8,
        signed: bool,
        order: ByteOrder,
    },
    Float {
        bits: u8,
        order: ByteOrder,
    },
    Text(TextForm),
    /// An array as the type of one element (the image-specification attribute is an element that is an array of 64 `uint8`).
    Array {
        dims: Vec<usize>,
        base: Box<Class>,
    },
    /// A record of named members (the equation constants of V18 are a name and a `float64`).
    Compound {
        fields: Vec<Field>,
    },
    /// A class that this library does not read (an enumeration, an opaque or a reference type), by the name HDF5 gives it. Nothing is guessed about it.
    Other(String),
}

/// The shape of the stored space: a scalar is not an array of one element, and the two are kept apart (`File_Version` is one in some files, the other in others).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Shape {
    Scalar,
    Array(Vec<usize>),
}

/// How a value is stored: what it is, and in what shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoding {
    pub class: Class,
    pub shape: Shape,
}

/// The elements of a stored value, in row-major order, exactly as read.
///
/// An element that is itself an array (class [`Class::Array`]) is flattened in place: an attribute of one element that is an array of 64 `uint8` is a
/// `U8` of 64 numbers, and its [`Encoding`] says how they are grouped. A `float32` stays a `float32`: it is widened only when a caller asks.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    /// The elements of a fixed-length string, each as its bytes up to the first zero byte, **never decoded as text**: whether they are text, or the eight
    /// bytes of a `float64` as V17 March stores `Power_Min`, is the adapter's knowledge ([`strict_text`] is a view, not a decision).
    ///
    /// **A limit until question Q3-1 is answered for the March step:** the safe binding converts a fixed string on the way in, so the bytes after the
    /// first zero byte are not seen, and the raw bytes of a number that has a zero byte inside (the bytes of 100.0) come back empty
    /// (`tests/method_check.rs` pins this). Every fixed string of the eight miniatures is returned whole.
    Bytes(Vec<Vec<u8>>),
    /// The elements of a variable-length string.
    Text(Vec<String>),
}

/// A value together with the encoding it was stored in.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub encoding: Encoding,
    pub value: Value,
}

/// The bytes of a fixed-length string as text, if they are valid UTF-8 (ASCII is). It is a view and not a decision: eight bytes that hold a `float64`
/// can look like text (`66 66 66 66 66 26 59 40` is `fffff&Y@`), so only an adapter that knows its version's attribute says what the bytes are.
pub fn strict_text(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes).ok()
}
