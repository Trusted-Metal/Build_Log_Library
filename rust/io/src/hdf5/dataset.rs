//! Dataset descriptors, compound datasets and variable-length string datasets (IP-03 phase 3E, tasks T03.3 to T03.5).
//!
//! * **[`describe_dataset`]** says how a dataset is stored: its class (the same [`Class`] as an attribute has), its shape and maximum shape (an unlimited
//!   dimension is `None`), its layout, chunks, filters with their parameters, and the fill and allocation settings. It describes a dataset of any class; a class
//!   this library does not read is [`Class::Other`] with the reason, and nothing is guessed.
//! * **[`read_name_value_rows`]** reads a compound dataset of rows of a fixed-length name and a `float64` (V18's equation constants), in the order stored.
//!   The binding converts neither of the two string character sets into the other, so the type it reads into is chosen from the descriptor.
//! * **[`read_strings`]** reads a range of a variable-length string dataset (the values of an OPC UA series).
//!
//! Numeric datasets are read in blocks by phase 3F; nothing here reads samples.

use std::ops::Range;

use build_log_model::{Error, ErrorKind};
use hdf5::dataset::Dataset;
use hdf5::plist::dataset_create::{AllocTime, FillTime, FillValue, Layout};
use hdf5::types::{
    FixedAscii, FixedUnicode, FloatSize, TypeDescriptor, VarLenAscii, VarLenUnicode,
};
use hdf5::{filters::Filter as BindingFilter, H5Type};

use super::attr::class_of;
use super::error::{from_binding, unsupported};
use super::value::Class;

/// How the values of a dataset are laid out in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DatasetLayout {
    /// Stored in the object header.
    Compact,
    /// Stored in one block.
    Contiguous,
    /// Stored in chunks.
    Chunked,
    /// Drawn from datasets of other files.
    Virtual,
}

/// The kind of a filter on a chunked dataset.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FilterKind {
    Deflate,
    Shuffle,
    Fletcher32,
    Szip,
    Nbit,
    ScaleOffset,
    /// A filter this library does not name, by what the binding says of it.
    Other(String),
}

/// A filter and its parameters. The miniatures hold only `Deflate` (its parameter is the compression level); the parameters of the other kinds are read where
/// the binding gives them as numbers and are empty otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Filter {
    pub kind: FilterKind,
    pub parameters: Vec<u32>,
}

/// Whether a dataset has a fill value, as the file says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Fill {
    Undefined,
    Default,
    UserDefined,
}

/// When the fill value is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FillWhen {
    IfSet,
    Alloc,
    Never,
}

/// When the space of a dataset is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Allocation {
    Early,
    Incremental,
    Late,
}

/// How a dataset is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetDescriptor {
    /// What one element is. A dataset of a class this library does not read is [`Class::Other`].
    pub class: Class,
    pub shape: Vec<usize>,
    /// The largest size of each dimension; `None` is unlimited.
    pub max_shape: Vec<Option<usize>>,
    pub layout: DatasetLayout,
    /// The chunk shape of a chunked dataset.
    pub chunks: Option<Vec<usize>>,
    /// In the order they are applied.
    pub filters: Vec<Filter>,
    pub fill: Fill,
    pub fill_when: FillWhen,
    pub allocation: Allocation,
}

/// Describes how a dataset is stored, without reading any of its values.
pub fn describe_dataset(dataset: &Dataset) -> Result<DatasetDescriptor, Error> {
    let datatype = dataset
        .dtype()
        .map_err(|e| from_binding("reading the datatype of a dataset", e))?;
    let class = match datatype.to_descriptor() {
        Ok(descriptor) => {
            class_of(&datatype, &descriptor).unwrap_or_else(|e| Class::Other(e.to_string()))
        }
        Err(e) => Class::Other(e.to_string()),
    };
    let space = dataset
        .space()
        .map_err(|e| from_binding("reading the dataspace of a dataset", e))?;
    let plist = dataset
        .create_plist()
        .map_err(|e| from_binding("reading the creation properties of a dataset", e))?;
    let layout = match dataset.layout() {
        Layout::Compact => DatasetLayout::Compact,
        Layout::Contiguous => DatasetLayout::Contiguous,
        Layout::Chunked => DatasetLayout::Chunked,
        #[allow(unreachable_patterns)]
        _ => DatasetLayout::Virtual,
    };
    Ok(DatasetDescriptor {
        class,
        shape: space.shape(),
        max_shape: space.maxdims(),
        layout,
        chunks: dataset.chunk(),
        filters: dataset.filters().iter().map(filter_of).collect(),
        fill: match plist.fill_value_defined() {
            FillValue::Undefined => Fill::Undefined,
            FillValue::Default => Fill::Default,
            FillValue::UserDefined => Fill::UserDefined,
        },
        fill_when: match plist.fill_time() {
            FillTime::IfSet => FillWhen::IfSet,
            FillTime::Alloc => FillWhen::Alloc,
            FillTime::Never => FillWhen::Never,
        },
        allocation: match plist.alloc_time() {
            AllocTime::Early => Allocation::Early,
            AllocTime::Incr => Allocation::Incremental,
            AllocTime::Late => Allocation::Late,
        },
    })
}

fn filter_of(filter: &BindingFilter) -> Filter {
    match filter {
        BindingFilter::Deflate(level) => Filter {
            kind: FilterKind::Deflate,
            parameters: vec![u32::from(*level)],
        },
        BindingFilter::Shuffle => Filter {
            kind: FilterKind::Shuffle,
            parameters: Vec::new(),
        },
        BindingFilter::Fletcher32 => Filter {
            kind: FilterKind::Fletcher32,
            parameters: Vec::new(),
        },
        BindingFilter::SZip(..) => Filter {
            kind: FilterKind::Szip,
            parameters: Vec::new(),
        },
        BindingFilter::NBit => Filter {
            kind: FilterKind::Nbit,
            parameters: Vec::new(),
        },
        BindingFilter::ScaleOffset(..) => Filter {
            kind: FilterKind::ScaleOffset,
            parameters: Vec::new(),
        },
        BindingFilter::User(id, parameters) => Filter {
            kind: FilterKind::Other(format!("filter {id}")),
            parameters: parameters.clone(),
        },
        #[allow(unreachable_patterns)]
        other => Filter {
            kind: FilterKind::Other(format!("{other:?}")),
            parameters: Vec::new(),
        },
    }
}

/// One row of a compound dataset of a name and a `float64`.
#[derive(Debug, Clone, PartialEq)]
pub struct NameValue {
    /// The bytes of the name up to the first zero byte, never decoded (see [`Value::Bytes`](super::value::Value::Bytes)).
    pub name: Vec<u8>,
    pub value: f64,
}

/// The compound with a UTF-8 name of 64 bytes, as the laser calibration of V18 has it.
#[derive(H5Type, Clone, Debug)]
#[repr(C)]
struct RowUnicode {
    name: FixedUnicode<64>,
    value: f64,
}

/// The compound with an ASCII name of 64 bytes, as the sensor calibration of V18 has it.
#[derive(H5Type, Clone, Debug)]
#[repr(C)]
struct RowAscii {
    name: FixedAscii<64>,
    value: f64,
}

/// Reads a one-dimensional compound dataset of rows of a fixed-length name (64 bytes, ASCII or UTF-8) and a `float64`, in the order they are stored.
///
/// A dataset of any other shape or type is an `UnsupportedInVersion` error that says what it is.
pub fn read_name_value_rows(dataset: &Dataset) -> Result<Vec<NameValue>, Error> {
    let descriptor = dataset
        .dtype()
        .and_then(|d| d.to_descriptor())
        .map_err(|e| unsupported(format!("a dataset of a datatype that is not read ({e})")))?;
    if dataset.ndim() != 1 {
        return Err(unsupported(format!(
            "a compound dataset of {} dimensions",
            dataset.ndim()
        )));
    }
    let TypeDescriptor::Compound(compound) = &descriptor else {
        return Err(unsupported(format!(
            "a dataset that is not a compound: {descriptor:?}"
        )));
    };
    let members: Vec<(&str, &TypeDescriptor)> = compound
        .fields
        .iter()
        .map(|f| (f.name.as_str(), &f.ty))
        .collect();
    match members.as_slice() {
        [("name", TypeDescriptor::FixedUnicode(64)), ("value", TypeDescriptor::Float(FloatSize::U8))] =>
        {
            let rows = dataset
                .read_raw::<RowUnicode>()
                .map_err(|e| from_binding("reading a compound dataset", e))?;
            Ok(rows
                .iter()
                .map(|r| NameValue {
                    name: r.name.as_bytes().to_vec(),
                    value: r.value,
                })
                .collect())
        }
        [("name", TypeDescriptor::FixedAscii(64)), ("value", TypeDescriptor::Float(FloatSize::U8))] =>
        {
            let rows = dataset
                .read_raw::<RowAscii>()
                .map_err(|e| from_binding("reading a compound dataset", e))?;
            Ok(rows
                .iter()
                .map(|r| NameValue {
                    name: r.name.as_bytes().to_vec(),
                    value: r.value,
                })
                .collect())
        }
        _ => Err(unsupported(format!(
            "a compound dataset that is not a name of 64 bytes and a float64: {descriptor:?}"
        ))),
    }
}

/// Reads the values `range` of a one-dimensional variable-length string dataset.
///
/// A range that is not inside the dataset is an `InvalidIndex` error; a dataset that holds anything but variable-length strings is an
/// `UnsupportedInVersion` error that says what it holds.
pub fn read_strings(dataset: &Dataset, range: Range<usize>) -> Result<Vec<String>, Error> {
    let descriptor = dataset
        .dtype()
        .and_then(|d| d.to_descriptor())
        .map_err(|e| unsupported(format!("a dataset of a datatype that is not read ({e})")))?;
    let unicode = match descriptor {
        TypeDescriptor::VarLenUnicode => true,
        TypeDescriptor::VarLenAscii => false,
        other => {
            return Err(unsupported(format!(
                "a dataset that does not hold variable-length strings: {other:?}"
            )))
        }
    };
    if dataset.ndim() != 1 {
        return Err(unsupported(format!(
            "a string dataset of {} dimensions",
            dataset.ndim()
        )));
    }
    let length = dataset.shape()[0];
    if range.start > range.end || range.end > length {
        return Err(Error::new(ErrorKind::InvalidIndex {
            what: format!(
                "the values {}..{} of a string dataset of {length} values",
                range.start, range.end
            ),
        }));
    }
    if range.is_empty() {
        return Ok(Vec::new());
    }
    let read = "reading a string dataset";
    if unicode {
        let values = dataset
            .read_slice_1d::<VarLenUnicode, _>(range)
            .map_err(|e| from_binding(read, e))?;
        Ok(values.iter().map(|s| s.as_str().to_string()).collect())
    } else {
        let values = dataset
            .read_slice_1d::<VarLenAscii, _>(range)
            .map_err(|e| from_binding(read, e))?;
        Ok(values.iter().map(|s| s.as_str().to_string()).collect())
    }
}
