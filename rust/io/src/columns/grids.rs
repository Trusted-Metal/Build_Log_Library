//! A whole correction grid (IP-24 phase 24B, task T24.2, decision D24-11): the values of a numeric dataset of one to three dimensions, read as stored.
//!
//! * **A grid is a dataset of one to three dimensions** of a numeric type that a column has, and it is read **whole**: it is about a megabyte, so there is no range and nothing is cached. A dataset
//!   of another number of dimensions, or of a type that no column has, is `UnsupportedInVersion` and names the version.
//! * **The values come out as stored**: in the type the dataset holds, and **row-major in the extents as the file stores them** (the last index running fastest), whatever the dataset's chunks
//!   are. Nothing is converted, scaled or repaired, **and a value that is not a number is returned as it is**: the NaNs of a V17 grid are returned bit for bit.
//! * **The read is the ranged reader of IP-03** (`hdf5::read_blocks` with the whole region): the token is checked before each block, progress is reported after it, and **a cancelled read returns
//!   `Cancelled` and no values**. A result that cannot be allocated is a typed error, as for a column (decision D06-17).
//! * **The engine does not say what a grid is.** Its domain, the order of its axes and which grid it is are not stated by any file, and nothing here decides them.
//!
//! **What is a readable grid is said once, here** ([`describe_grid`]): an adapter that describes a grid in its metadata asks this, so that the metadata never promises a grid that the engine then
//! refuses to read, and [`read_grid`] asks it too.
//!
//! No group, dataset or attribute name of any version appears here: the adapter says which dataset holds a grid.

use build_log_model::{ColumnData, Dims, Error, ErrorKind, GridData, NumericType, ReadContext};
use hdf5::dataset::Dataset;

use super::convert::{reserve, Native};
use super::engine::{stored_type, Stored};
use crate::hdf5::{read_blocks, Region};

/// The most dimensions of a grid that are read: the ranged reader's own limit.
const MOST_DIMENSIONS: usize = 3;

/// Every value of `dataset`, row-major, through the ranged reader of IP-03.
fn whole<T: Native>(
    dataset: &Dataset,
    values_held: usize,
    context: &ReadContext,
) -> Result<ColumnData, Error> {
    let mut values: Vec<T> = Vec::new();
    reserve(&mut values, values_held)?;
    read_blocks::<T>(dataset, &Region::whole(), context, |block| {
        values.extend(block.values);
        Ok(())
    })?;
    Ok(T::column(values))
}

/// The type and the extents of a grid that [`read_grid`] reads: a dataset of one to three dimensions of a numeric type that a column has. **Anything else is `UnsupportedInVersion`** and names `version`.
/// An adapter that describes a grid in its metadata asks this first, and keeps what it cannot describe where it is (in `extra`), so that nothing is declared that cannot be read.
pub fn describe_grid(dataset: &Dataset, version: &str) -> Result<(NumericType, Dims), Error> {
    let shape = dataset.shape();
    if shape.is_empty() || shape.len() > MOST_DIMENSIONS {
        return Err(Error::new(ErrorKind::UnsupportedInVersion {
            version: version.to_string(),
            feature: format!(
                "a grid of {} dimensions (one to {MOST_DIMENSIONS} are read)",
                shape.len()
            ),
        }));
    }
    let stored = stored_type(dataset, version)?;
    let extents: Vec<u64> = shape.iter().map(|&n| n as u64).collect();
    Ok((stored.numeric_type(), Dims::new(&extents)?))
}

/// Reads the grid that `dataset` holds, whole and as stored. `version` is the key of the version whose file this is, which an error names when the grid is not one that is read.
pub fn read_grid(
    dataset: &Dataset,
    version: &str,
    context: &ReadContext,
) -> Result<GridData, Error> {
    context.check_cancelled()?;
    let (_, extents) = describe_grid(dataset, version)?;
    let stored = stored_type(dataset, version)?;
    let count = extents
        .as_slice()
        .iter()
        .fold(1usize, |all, &n| all.saturating_mul(n as usize));
    let values = if count == 0 {
        stored.empty_column()
    } else {
        match stored {
            Stored::I16 => whole::<i16>(dataset, count, context)?,
            Stored::I32 => whole::<i32>(dataset, count, context)?,
            Stored::U64 => whole::<u64>(dataset, count, context)?,
            Stored::F32 => whole::<f32>(dataset, count, context)?,
            Stored::F64 => whole::<f64>(dataset, count, context)?,
            Stored::U8 => whole::<u8>(dataset, count, context)?,
        }
    };
    GridData::new(extents, values)
}

#[cfg(test)]
mod tests {
    use build_log_model::CancelToken;
    use hdf5::File;

    use super::*;

    /// A file of datasets for the tests below, removed when the test ends.
    struct Made(std::path::PathBuf, File);

    impl Drop for Made {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn made(label: &str) -> Made {
        let path = std::env::temp_dir().join(format!(
            "build-log-io-grids-{}-{label}.h5",
            std::process::id()
        ));
        let file = File::create(&path).expect("a file");
        Made(path, file)
    }

    fn bits(column: &ColumnData) -> Vec<u64> {
        match column {
            ColumnData::F64(values) => values.as_slice().iter().map(|v| v.to_bits()).collect(),
            other => panic!("expected float64, got {other:?}"),
        }
    }

    #[test]
    fn a_grid_reads_row_major_in_its_extents_with_a_nan_returned_bit_for_bit() {
        let file = made("nan");
        // 2 by 3 by 2, with two NaN, one of them with a payload that a copy that canonicalised would lose
        let odd_nan = f64::from_bits(0x7ff8_0000_0000_1234);
        let flat: Vec<f64> = vec![
            1.0,
            f64::NAN,
            3.0,
            4.0,
            odd_nan,
            6.0,
            7.0,
            8.0,
            9.0,
            10.0,
            11.0,
            12.0,
        ];
        let dataset = file
            .1
            .new_dataset::<f64>()
            .shape([2, 3, 2])
            .create("grid")
            .expect("a dataset");
        dataset.write_raw(&flat).expect("written");
        let got = read_grid(&dataset, "9", &ReadContext::new()).expect("the grid");
        assert_eq!(got.extents.as_slice(), [2, 3, 2]);
        assert_eq!(
            bits(&got.values),
            flat.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_chunked_grid_reads_in_the_same_order_as_a_contiguous_one() {
        let file = made("chunked");
        let flat: Vec<i32> = (0..2 * 7 * 5).collect();
        let contiguous = file
            .1
            .new_dataset::<i32>()
            .shape([2, 7, 5])
            .create("a")
            .expect("a dataset");
        contiguous.write_raw(&flat).expect("written");
        // chunks that cut the grid in every dimension
        let chunked = file
            .1
            .new_dataset::<i32>()
            .shape([2, 7, 5])
            .chunk([1, 3, 2])
            .create("b")
            .expect("a dataset");
        chunked.write_raw(&flat).expect("written");
        for dataset in [&contiguous, &chunked] {
            let got = read_grid(dataset, "9", &ReadContext::new()).expect("the grid");
            assert_eq!(got.extents.as_slice(), [2, 7, 5]);
            match got.values {
                ColumnData::I32(values) => assert_eq!(values.as_slice(), flat.as_slice()),
                other => panic!("expected int32, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_grid_of_one_dimension_and_one_of_no_value_read_and_the_rest_are_unsupported() {
        let file = made("kinds");
        let line = file
            .1
            .new_dataset::<f32>()
            .shape(4)
            .create("line")
            .expect("a dataset");
        line.write_raw(&[1.0f32, 2.0, 3.0, 4.0]).expect("written");
        assert_eq!(
            read_grid(&line, "9", &ReadContext::new())
                .expect("a line")
                .extents
                .as_slice(),
            [4]
        );
        // an extent of zero is a grid of no value, of the type that is stored
        let none = file
            .1
            .new_dataset::<u8>()
            .shape([0, 3])
            .create("none")
            .expect("a dataset");
        let empty = read_grid(&none, "9", &ReadContext::new()).expect("empty");
        assert!(empty.is_empty() && matches!(empty.values, ColumnData::U8(_)));
        assert_eq!(empty.extents.as_slice(), [0, 3]);
        // four dimensions, a scalar and text are not grids that are read, and the error names the version
        let four = file
            .1
            .new_dataset::<f64>()
            .shape([1, 1, 1, 2])
            .create("four")
            .expect("a dataset");
        let scalar = file
            .1
            .new_dataset::<f64>()
            .shape(())
            .create("scalar")
            .expect("a dataset");
        let text = file
            .1
            .new_dataset::<hdf5::types::FixedAscii<4>>()
            .shape(2)
            .create("text")
            .expect("a dataset");
        for (what, dataset) in [
            ("four dimensions", &four),
            ("a scalar", &scalar),
            ("text", &text),
        ] {
            let error = read_grid(dataset, "9", &ReadContext::new()).expect_err(what);
            match error.kind() {
                ErrorKind::UnsupportedInVersion { version, .. } => {
                    assert_eq!(version, "9", "{what}")
                }
                other => panic!("{what}: expected UnsupportedInVersion, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_grid_is_described_by_what_the_engine_reads_and_by_nothing_else() {
        let file = made("describe");
        let grid = file
            .1
            .new_dataset::<i32>()
            .shape([2, 257, 257])
            .create("grid")
            .expect("a dataset");
        let (stored_as, extents) = describe_grid(&grid, "9").expect("described");
        assert_eq!(
            (stored_as, extents.as_slice()),
            (NumericType::I32, &[2u64, 257, 257][..])
        );
        // a type that no column has (a signed byte) is a grid that is not described, so that no metadata promises it
        let byte = file
            .1
            .new_dataset::<i8>()
            .shape([2, 2])
            .create("byte")
            .expect("a dataset");
        assert!(matches!(
            describe_grid(&byte, "9").expect_err("not a column type").kind(),
            ErrorKind::UnsupportedInVersion { version, .. } if version == "9"
        ));
        // and what is described is what is read: the same extents and the same type
        grid.write_raw(&vec![1i32; 2 * 257 * 257]).expect("written");
        let read = read_grid(&grid, "9", &ReadContext::new()).expect("the grid");
        assert_eq!(read.extents.as_slice(), [2, 257, 257]);
        assert!(matches!(read.values, ColumnData::I32(_)));
    }

    #[test]
    fn a_grid_that_cannot_be_allocated_is_a_typed_error_that_names_its_size_before_anything_is_read(
    ) {
        let file = made("huge");
        // 2 to the 61st values of one byte: more than any machine has, and a dataset takes no room in the file until a value is written
        let huge = file
            .1
            .new_dataset::<u8>()
            .shape([1usize << 21, 1 << 20, 1 << 20])
            .create("huge")
            .expect("a dataset");
        let error = read_grid(&huge, "9", &ReadContext::new()).expect_err("no room");
        match error.kind() {
            ErrorKind::IoError { detail, .. } => assert!(
                detail.contains(&(1u64 << 61).to_string()),
                "the error names the size of the whole grid: {detail}"
            ),
            other => panic!("expected IoError, got {other:?}"),
        }
    }

    #[test]
    fn a_cancelled_read_returns_no_values() {
        let file = made("cancel");
        let dataset = file
            .1
            .new_dataset::<f64>()
            .shape([4, 4])
            .create("grid")
            .expect("a dataset");
        dataset.write_raw(&[0.0f64; 16]).expect("written");
        let token = CancelToken::new();
        token.cancel();
        let error = read_grid(&dataset, "9", &ReadContext::new().with_cancel(token))
            .expect_err("cancelled");
        assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error}");
    }
}
