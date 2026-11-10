//! The steps of a read, before and after the blocks: the dataset, its stored type and length, and the range (IP-06 phase 6B).

use std::ops::Range;

use build_log_model::{
    ColumnData, ColumnRange, ColumnRequest, Error, ErrorKind, NumericType, Precision, ReadContext,
};
use hdf5::dataset::Dataset;
use hdf5::types::{FloatSize, IntSize, TypeDescriptor};

use super::blocks::{collect, Source};
use super::cache::{Cache, Key};
use super::convert::Native;
use super::{Identity, Located, Locator};
use crate::hdf5::{block_rows, has_null_dataspace, Region};

/// **The one list of what the engine reads** (IP-06 phase 6I, decision D07-3): the types a column is stored in, and so the variants a result can have before a precision is asked for.
/// The enum, the list of all of them, the name each has in the Model, what HDF5 calls it and how it is spelt in an error are written together, so that they cannot differ. What
/// the engine can read and what a report of availability says it can read (`can_read`) are the same list, and adding a type is one line here (and its `Element`, its `Native`
/// and the arms of the two reads, which the compiler asks for).
macro_rules! stored_types {
    ($($variant:ident($elem:ty): $numeric:ident = $descriptor:pat => $name:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(super) enum Stored {
            $($variant),+
        }

        impl Stored {
            /// Every type the engine reads.
            pub(super) const ALL: &'static [Stored] = &[$(Stored::$variant),+];

            /// A column of no value of this type: what a dataset with a null dataspace, or an empty region, reads as.
            pub(super) fn empty_column(self) -> ColumnData {
                match self {
                    $(Stored::$variant => <$elem as Native>::column(Vec::new())),+
                }
            }

            /// What the Model calls the type.
            pub(super) fn numeric_type(self) -> NumericType {
                match self {
                    $(Stored::$variant => NumericType::$numeric),+
                }
            }

            /// How an error spells the type.
            fn name(self) -> &'static str {
                match self {
                    $(Stored::$variant => $name),+
                }
            }

            /// The stored type that a dataset's HDF5 type is, if the engine reads it.
            fn of_descriptor(descriptor: &TypeDescriptor) -> Option<Stored> {
                match descriptor {
                    $($descriptor => Some(Stored::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

stored_types! {
    I16(i16): I16 = TypeDescriptor::Integer(IntSize::U2) => "int16",
    I32(i32): I32 = TypeDescriptor::Integer(IntSize::U4) => "int32",
    U64(u64): U64 = TypeDescriptor::Unsigned(IntSize::U8) => "uint64",
    F32(f32): F32 = TypeDescriptor::Float(FloatSize::U4) => "float32",
    F64(f64): F64 = TypeDescriptor::Float(FloatSize::U8) => "float64",
    U8(u8): U8 = TypeDescriptor::Unsigned(IntSize::U1) => "uint8",
}

/// Does the engine read a column stored as this type? It is the list that `stored_type` reads by, so that what a report of availability says and what a read does cannot differ.
pub fn can_read(dtype: NumericType) -> bool {
    Stored::ALL.iter().any(|s| s.numeric_type() == dtype)
}

/// The list of what a column may be stored as, spelt for an error.
fn readable_names() -> String {
    let names: Vec<String> = Stored::ALL
        .iter()
        .map(|s| format!("`{}`", s.name()))
        .collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => names.join(""),
    }
}

fn unsupported(version: &str, feature: String) -> Error {
    Error::new(ErrorKind::UnsupportedInVersion {
        version: version.to_string(),
        feature,
    })
}

/// What the dataset stores, or the error that says it is not a type that a column has.
pub(super) fn stored_type(dataset: &Dataset, version: &str) -> Result<Stored, Error> {
    let descriptor = dataset
        .dtype()
        .and_then(|d| d.to_descriptor())
        .map_err(|e| {
            unsupported(
                version,
                format!("a column of a datatype that is not read ({e})"),
            )
        })?;
    Stored::of_descriptor(&descriptor).ok_or_else(|| {
        unsupported(
            version,
            format!(
                "a column stored as {descriptor:?} (a column is {})",
                readable_names()
            ),
        )
    })
}

/// The number of values of the column, if the dataset is one-dimensional. A dataset with a null dataspace has none (the caller says so: it has no dimension, which is not the damage that a
/// table is).
fn length(dataset: &Dataset) -> Result<usize, Error> {
    match dataset.shape().as_slice() {
        [length] => Ok(*length),
        other => Err(Error::new(ErrorKind::Corrupt {
            detail: format!(
                "a column is one-dimensional, and the dataset that holds it has {} dimensions",
                other.len()
            ),
        })),
    }
}

/// The values a range selects of a column of `length` values.
fn resolve(range: &ColumnRange, length: usize, version: &str) -> Result<Range<usize>, Error> {
    let invalid = |what: String| Error::new(ErrorKind::InvalidIndex { what });
    match range {
        ColumnRange::All => Ok(0..length),
        ColumnRange::Indices(range) => {
            if range.start > range.end || range.end > length as u64 {
                return Err(invalid(format!(
                    "the indices {}..{} are outside a column of {length} values",
                    range.start, range.end
                )));
            }
            // inside a column that is in memory, so both ends fit
            Ok(range.start as usize..range.end as usize)
        }
        ColumnRange::Time { .. } => Err(unsupported(
            version,
            "a time range (it needs a segment table, and these files store none)".to_string(),
        )),
        #[allow(unreachable_patterns)]
        _ => Err(unsupported(
            version,
            "a kind of range that this reader does not know".to_string(),
        )),
    }
}

/// The generation of the blocks of a file that is opened for reading and does not change (decision D06-7). A source that can grow has its own.
const GENERATION: u64 = 0;

/// A read, without the location that `read_column` adds to every error. With a cache, the blocks of the column are looked for in it and kept in it as the budget of the context says.
pub(super) fn read(
    request: &ColumnRequest,
    locator: &dyn Locator,
    cache: Option<&Cache>,
    context: &ReadContext,
) -> Result<ColumnData, Error> {
    context.check_cancelled()?;
    let Located { dataset } = locator.locate(request)?;
    let identity = Identity {
        layer: request.layer,
        laser: request.laser.clone(),
        channel: request.channel.clone(),
    };
    read_dataset(
        &dataset,
        locator.version(),
        identity,
        &request.range,
        request.precision,
        cache,
        context,
    )
}

/// The values `range` of the column that `dataset` holds, at `precision`, as `read` does once its locator has found the dataset. `identity` names the column for the cache, and
/// must be different for every column that shares one cache.
pub(super) fn read_dataset(
    dataset: &Dataset,
    version: &str,
    identity: Identity,
    range: &ColumnRange,
    precision: Precision,
    cache: Option<&Cache>,
    context: &ReadContext,
) -> Result<ColumnData, Error> {
    let stored = stored_type(dataset, version)?;
    // a dataset with a null dataspace has a type, no dimension and no value (IP-07 phase 7A, D07-2): its column is empty, of the stored type and at the precision asked for. A range is still
    // checked against its length, which is zero, so that `0..0` is the empty column and `0..1` is outside it.
    let null = has_null_dataspace(dataset)?;
    let length = if null { 0 } else { length(dataset)? };
    let range = resolve(range, length, version)?;
    if null {
        return stored.empty_column().with_precision(precision);
    }
    let mut source = Source {
        size: block_rows(dataset, &Region::whole())?,
        dataset,
        cache,
        key: Key {
            layer: identity.layer,
            laser: identity.laser,
            channel: identity.channel,
            block: 0,
            generation: GENERATION,
        },
        length,
    };
    match stored {
        Stored::I16 => collect::<i16>(&mut source, range, precision, context),
        Stored::I32 => collect::<i32>(&mut source, range, precision, context),
        Stored::U64 => collect::<u64>(&mut source, range, precision, context),
        Stored::F32 => collect::<f32>(&mut source, range, precision, context),
        Stored::F64 => collect::<f64>(&mut source, range, precision, context),
        Stored::U8 => collect::<u8>(&mut source, range, precision, context),
    }
}
