//! A region of a two-dimensional frame (IP-06 phase 6G, task T06.7, decision D06-13): the rectangle of pixels that a request names, read as stored.
//!
//! * **A frame is a dataset of two dimensions, rows and then columns**, in the order the file stores them: a region's rows are the first dimension and its columns the second, and a frame that is not
//!   square is not its own transpose. A dataset of another number of dimensions is `Corrupt`: it is not a frame.
//! * **The region is checked against the frame before any pixel is read**: a region that is not inside it (a range that ends past the frame, or starts after it ends) is `InvalidIndex`, and the error
//!   **says the frame's dimensions**. A region of no rows or no columns is empty and is not an error; no pixel is read for it.
//! * **The pixels come out as stored**, row-major (the region's rows, and the columns of each), typed by what the dataset holds like any other column: nothing is converted, and a pixel type that
//!   no column has is `UnsupportedInVersion`.
//! * **The read is the ranged reader of IP-03 in two dimensions** (`hdf5::read_blocks` with a range for each dimension): the token is checked before each block, progress is reported after it, and
//!   **a cancelled read returns `Cancelled` and no pixels**. A region of a frame is decoded **in the chunks that hold it**: a frame that is one chunk (the 25.7 MB frames of the cameras) is decoded
//!   whole for any region of it, which is decision G17's finding and not specially treated here.
//! * **Nothing is kept**: a frame is large, and a viewer that wants it again holds what it was given.
//! * **A result that cannot be allocated is a typed error**, as for a column (decision D06-17).
//!
//! No group, dataset or attribute name of any version appears here: the adapter says which dataset holds a frame.

use std::ops::Range;

use build_log_model::{ColumnData, Error, ErrorKind, ImageData, ImageRegion, ReadContext};
use hdf5::dataset::Dataset;

use super::convert::{reserve, Native};
use super::engine::{stored_type, Stored};
use crate::hdf5::{read_blocks, Region};

/// The values of a rectangle of `dataset`, row-major, through the ranged reader of IP-03.
fn rectangle<T: Native>(
    dataset: &Dataset,
    rows: Range<usize>,
    columns: Range<usize>,
    context: &ReadContext,
) -> Result<ColumnData, Error> {
    let mut values: Vec<T> = Vec::new();
    reserve(&mut values, rows.len().saturating_mul(columns.len()))?;
    read_blocks::<T>(
        dataset,
        &Region::new(vec![rows, columns]),
        context,
        |block| {
            values.extend(block.values);
            Ok(())
        },
    )?;
    Ok(T::column(values))
}

/// Reads the region `region` of the frame `dataset` holds (the whole frame if there is none), as stored. `version` is the key of the version whose file this is, which an error names when the pixel type
/// is not one that a column has.
pub fn read_frame(
    dataset: &Dataset,
    version: &str,
    region: Option<&ImageRegion>,
    context: &ReadContext,
) -> Result<ImageData, Error> {
    context.check_cancelled()?;
    let (frame_rows, frame_columns) = match dataset.shape().as_slice() {
        [rows, columns] => (*rows as u64, *columns as u64),
        other => {
            return Err(Error::new(ErrorKind::Corrupt {
                detail: format!(
                    "a frame is two-dimensional, and the dataset that holds it has {} dimensions",
                    other.len()
                ),
            }))
        }
    };
    let wanted = match region {
        Some(region) => region.clone(),
        None => ImageRegion::whole(frame_rows, frame_columns),
    };
    if !wanted.is_inside(frame_rows, frame_columns) {
        return Err(Error::new(ErrorKind::InvalidIndex {
            what: format!(
                "the region of rows {}..{} and columns {}..{} is outside a frame of {frame_rows} rows by {frame_columns} columns",
                wanted.rows.start, wanted.rows.end, wanted.columns.start, wanted.columns.end
            ),
        }));
    }
    let stored = stored_type(dataset, version)?;
    // inside a frame that is in the file, so both ends fit
    let rows = wanted.rows.start as usize..wanted.rows.end as usize;
    let columns = wanted.columns.start as usize..wanted.columns.end as usize;
    let empty = wanted.pixels() == 0;
    let pixels = if empty {
        stored.empty_column()
    } else {
        match stored {
            Stored::I16 => rectangle::<i16>(dataset, rows, columns, context)?,
            Stored::I32 => rectangle::<i32>(dataset, rows, columns, context)?,
            Stored::U64 => rectangle::<u64>(dataset, rows, columns, context)?,
            Stored::F32 => rectangle::<f32>(dataset, rows, columns, context)?,
            Stored::F64 => rectangle::<f64>(dataset, rows, columns, context)?,
            Stored::U8 => rectangle::<u8>(dataset, rows, columns, context)?,
        }
    };
    ImageData::new(wanted, pixels)
}

#[cfg(test)]
mod tests {
    use build_log_model::ErrorKind;
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
            "build-log-io-frames-{}-{label}.h5",
            std::process::id()
        ));
        let file = File::create(&path).expect("a file");
        Made(path, file)
    }

    #[test]
    fn a_dataset_that_is_not_two_dimensional_is_corrupt_and_a_pixel_type_that_no_column_has_is_unsupported(
    ) {
        let file = made("kinds");
        let line = file
            .1
            .new_dataset::<u8>()
            .shape(4)
            .create("line")
            .expect("a dataset");
        let cube = file
            .1
            .new_dataset::<u8>()
            .shape([2, 2, 3])
            .create("cube")
            .expect("a dataset");
        let wide = file
            .1
            .new_dataset::<u16>()
            .shape([2, 2])
            .create("wide")
            .expect("a dataset");
        let context = ReadContext::new();
        for dataset in [&line, &cube] {
            let error = read_frame(dataset, "test-version", None, &context).unwrap_err();
            assert!(matches!(error.kind(), ErrorKind::Corrupt { .. }), "{error}");
        }
        let error = read_frame(&wide, "test-version", None, &context).unwrap_err();
        match error.kind() {
            ErrorKind::UnsupportedInVersion { version, .. } => assert_eq!(version, "test-version"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_frame_of_16_bit_integers_reads_as_stored() {
        // IP-06 phase 6I: `int16` is a type a column has, so a frame of it is read as stored too, with both ends of its range and with the rows and the columns told apart
        let file = made("narrow");
        let pixels: Vec<i16> = vec![i16::MIN, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, i16::MAX];
        let frame = file
            .1
            .new_dataset::<i16>()
            .shape([3, 4])
            .create("narrow")
            .expect("a dataset");
        frame.write_raw(&pixels).expect("its values");
        let context = ReadContext::new();
        let whole = read_frame(&frame, "test-version", None, &context).expect("the frame");
        assert_eq!(
            whole.pixels,
            ColumnData::I16(build_log_model::Shared::new(pixels))
        );
        // rows 1..3 and columns 2..4 of a frame of 3 by 4: (1,2) (1,3) (2,2) (2,3)
        let part = read_frame(
            &frame,
            "test-version",
            Some(&ImageRegion::new(1..3, 2..4)),
            &context,
        )
        .expect("a region");
        assert_eq!(
            part.pixels,
            ColumnData::I16(build_log_model::Shared::new(vec![3, 4, 7, i16::MAX]))
        );
        let none = read_frame(
            &frame,
            "test-version",
            Some(&ImageRegion::new(1..1, 0..4)),
            &context,
        )
        .expect("an empty region");
        assert!(matches!(none.pixels, ColumnData::I16(_)) && none.pixels.is_empty());
    }

    #[test]
    fn a_frame_is_typed_by_what_is_stored_whether_it_has_pixels_or_none() {
        let file = made("types");
        let floats: Vec<f32> = (0..12).map(|i| i as f32 * 0.5).collect();
        let frame = file
            .1
            .new_dataset::<f32>()
            .shape([3, 4])
            .create("floats")
            .expect("a dataset");
        frame.write_raw(&floats).expect("its values");
        let context = ReadContext::new();
        // rows 1..3 and columns 1..3 of a frame of 3 by 4: (1,1) (1,2) (2,1) (2,2) = 5 6 9 10 half units
        let part = read_frame(
            &frame,
            "test-version",
            Some(&ImageRegion::new(1..3, 1..3)),
            &context,
        )
        .expect("a region");
        assert_eq!(
            part.pixels,
            ColumnData::F32(build_log_model::Shared::new(vec![2.5, 3.0, 4.5, 5.0]))
        );
        // an empty region has no pixels and is of the stored type
        let none = read_frame(
            &frame,
            "test-version",
            Some(&ImageRegion::new(2..2, 0..4)),
            &context,
        )
        .expect("an empty region");
        assert!(matches!(none.pixels, ColumnData::F32(_)) && none.pixels.is_empty());
        // a region of the transpose of the frame's own shape is outside it, and the error names the dimensions
        let error = read_frame(
            &frame,
            "test-version",
            Some(&ImageRegion::new(0..4, 0..3)),
            &context,
        )
        .unwrap_err();
        assert!(error.to_string().contains("3 rows by 4 columns"), "{error}");
    }
}
