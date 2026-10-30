//! Regions of the camera frames: a request for a rectangle of a frame, and the pixels that come back (IP-06 phase 6G; decision D06-13; requirement BL-CD-16).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** caller options (`ImageRegion`, `ImageRequest`); library (`ImageData`).
//! * **Requirement:** BL-CD-16 (the camera images), BL-CD-03 (bulk data only through a column accessor).
//!
//! # What this module claims, and what it does not
//!
//! * **The frame is described by the metadata and its pixels are read through [`ColumnSource::read_image`](crate::ColumnSource::read_image).** The build's image channels say which
//!   phases there are, in which layers, and the format of a frame (its rows, its columns and the type of a pixel); a request names a layer, a phase (the channel's key) and a
//!   [`ImageRegion`], and the whole frame if it names none.
//! * **A region is rows and columns, in that order, counted from the top left of the frame as the file stores it.** A region of a frame that is not square is not the region of its
//!   transpose: the rows are the first dimension of the file's dataset, and the columns the second. A region of no rows or no columns is empty and is not an error.
//! * **The pixels are as stored**, row-major (the region's rows, and the columns of each), in the type the file holds them: nothing is converted, scaled or registered to the plate.
//!   **No pyramid and no downsampling** (Vision's DA-15 names them; they wait for IP-17 and the V18 design).
//! * **A region outside the frame is `InvalidIndex`, and the error says the frame's dimensions.**
//!
//! The policy guards in `tests/policy_guards.rs` scan this file: every public struct says who builds it, and the type that holds bulk data says `// bulk-container:`.

use std::ops::Range;

use crate::column::ColumnData;
use crate::error::{Error, ErrorKind, Location};
use crate::identity::{ChannelKey, LayerId};

/// A rectangle of a frame: the rows it covers and the columns, each as a range (`start..end`, the end not included). Rows are counted from the top of the frame as the file stores
/// it and columns from its left.
// constructed-by: caller-options
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ImageRegion {
    pub rows: Range<u64>,
    pub columns: Range<u64>,
}

impl ImageRegion {
    pub fn new(rows: Range<u64>, columns: Range<u64>) -> Self {
        ImageRegion { rows, columns }
    }

    /// The whole of a frame of `rows` by `columns` pixels.
    pub fn whole(rows: u64, columns: u64) -> Self {
        ImageRegion {
            rows: 0..rows,
            columns: 0..columns,
        }
    }

    /// How many rows the region covers.
    pub fn row_count(&self) -> u64 {
        self.rows.end.saturating_sub(self.rows.start)
    }

    /// How many columns the region covers.
    pub fn column_count(&self) -> u64 {
        self.columns.end.saturating_sub(self.columns.start)
    }

    /// How many pixels the region holds.
    pub fn pixels(&self) -> u64 {
        self.row_count().saturating_mul(self.column_count())
    }

    /// Whether the region lies inside a frame of `rows` by `columns` pixels: each range starts no later than it ends and ends no later than the frame does.
    pub fn is_inside(&self, rows: u64, columns: u64) -> bool {
        self.rows.start <= self.rows.end
            && self.rows.end <= rows
            && self.columns.start <= self.columns.end
            && self.columns.end <= columns
    }

    /// Whether `other` lies inside this region (both in the coordinates of the frame).
    pub fn contains(&self, other: &ImageRegion) -> bool {
        self.rows.start <= other.rows.start
            && other.rows.end <= self.rows.end
            && self.columns.start <= other.columns.start
            && other.columns.end <= self.columns.end
            && other.rows.start <= other.rows.end
            && other.columns.start <= other.columns.end
    }
}

/// A request for the pixels of a region of one frame: the layer, the phase (the key of the build's image channel) and the region, the whole frame by default.
// constructed-by: caller-options
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ImageRequest {
    pub layer: LayerId,
    /// The key of the image channel of the build, which names the phase (before or after the exposure).
    pub channel: ChannelKey,
    /// The rectangle wanted, or `None` for the whole frame.
    pub region: Option<ImageRegion>,
}

impl ImageRequest {
    /// A request for the whole frame of a phase in a layer.
    pub fn new(layer: LayerId, channel: ChannelKey) -> Self {
        ImageRequest {
            layer,
            channel,
            region: None,
        }
    }

    pub fn with_region(mut self, region: ImageRegion) -> Self {
        self.region = Some(region);
        self
    }

    /// Where this request points, for an error that must say which frame it was about: the layer, the channel and the operation `read_image`.
    pub fn location(&self) -> Location {
        Location::new()
            .with_layer(self.layer)
            .with_channel(self.channel.clone())
            .with_operation("read_image")
    }
}

/// The pixels of a region of a frame, row-major, as stored, with the region they cover.
// constructed-by: library
// bulk-container: the typed, shared pixels of a region of one frame
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ImageData {
    /// The rectangle of the frame that these pixels cover: the one asked for, or the whole frame.
    pub region: ImageRegion,
    /// The pixels: the region's rows, and in each the region's columns. As many as the region holds, in the type the file stores them.
    pub pixels: ColumnData,
}

impl ImageData {
    /// The pixels of a region. A producer that gets the count wrong is told: `pixels` must hold exactly the pixels of `region`.
    pub fn new(region: ImageRegion, pixels: ColumnData) -> Result<ImageData, Error> {
        if pixels.len() as u64 != region.pixels() {
            return Err(Error::new(ErrorKind::InvalidIndex {
                what: format!(
                    "{} pixels for a region of {} rows by {} columns",
                    pixels.len(),
                    region.row_count(),
                    region.column_count()
                ),
            }));
        }
        Ok(ImageData { region, pixels })
    }

    /// How many rows these pixels have.
    pub fn rows(&self) -> u64 {
        self.region.row_count()
    }

    /// How many columns these pixels have.
    pub fn columns(&self) -> u64 {
        self.region.column_count()
    }

    /// The pixels of `region`, a copy, where `region` is in the coordinates of the frame and lies inside the region these pixels cover. `None` if it does not.
    pub fn crop(&self, region: &ImageRegion) -> Option<ImageData> {
        if !self.region.contains(region) {
            return None;
        }
        let width = self.columns() as usize;
        let from = (region.columns.start - self.region.columns.start) as usize;
        let take = region.column_count() as usize;
        let ranges: Vec<Range<usize>> = (region.rows.start..region.rows.end)
            .map(|row| {
                let start = (row - self.region.rows.start) as usize * width + from;
                start..start + take
            })
            .collect();
        Some(ImageData {
            region: region.clone(),
            pixels: self.pixels.gather(&ranges)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::{
        CancelToken, ColumnRequest, ColumnSource, InMemorySource, ReadContext, Shared,
    };

    /// A frame of `rows` by `columns` whose pixel at (row, column) is `row * 16 + column`, so that a pixel says where it was.
    fn frame(rows: u64, columns: u64) -> ImageData {
        let pixels: Vec<u8> = (0..rows)
            .flat_map(|r| (0..columns).map(move |c| (r * 16 + c) as u8))
            .collect();
        ImageData::new(
            ImageRegion::whole(rows, columns),
            ColumnData::U8(Shared::new(pixels)),
        )
        .expect("a frame")
    }

    fn pixels_of(data: &ImageData) -> Vec<u8> {
        match &data.pixels {
            ColumnData::U8(v) => v.as_slice().to_vec(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_region_counts_its_rows_its_columns_and_its_pixels_and_knows_what_is_inside_a_frame() {
        let region = ImageRegion::new(2..5, 1..9);
        assert_eq!(
            (region.row_count(), region.column_count(), region.pixels()),
            (3, 8, 24)
        );
        assert!(region.is_inside(5, 9) && !region.is_inside(4, 9) && !region.is_inside(5, 8));
        // a reversed range is not inside anything, and an empty one is
        let reversed = Range { start: 3, end: 2 };
        assert!(!ImageRegion::new(reversed, 0..1).is_inside(10, 10));
        assert!(ImageRegion::new(4..4, 0..10).is_inside(10, 10));
        assert_eq!(ImageRegion::new(4..4, 0..10).pixels(), 0);
        assert_eq!(ImageRegion::whole(6, 10), ImageRegion::new(0..6, 0..10));
        // a region contains another only when both its ranges do
        assert!(region.contains(&ImageRegion::new(3..4, 2..9)));
        assert!(!region.contains(&ImageRegion::new(1..4, 2..9)));
        assert!(!region.contains(&ImageRegion::new(3..4, 0..9)));
    }

    #[test]
    fn a_crop_takes_the_rows_and_the_columns_it_names_and_does_not_mix_them_up() {
        // a frame of 5 rows by 12 columns: it is not square, so a region read the other way round would be wrong
        let whole = frame(5, 12);
        let part = whole.crop(&ImageRegion::new(1..4, 3..7)).expect("inside");
        assert_eq!((part.rows(), part.columns()), (3, 4));
        // row 1 is 16..., and its columns 3 to 6 are 19 20 21 22: a pixel says where it was
        assert_eq!(
            pixels_of(&part),
            [19, 20, 21, 22, 35, 36, 37, 38, 51, 52, 53, 54]
        );
        // a crop of a crop is a crop of the frame, in the frame's own coordinates
        let again = part.crop(&ImageRegion::new(2..3, 4..6)).expect("inside");
        assert_eq!(pixels_of(&again), [36, 37]);
        assert_eq!(again, whole.crop(&ImageRegion::new(2..3, 4..6)).unwrap());
        // outside this data, and outside the frame, is not a crop
        assert!(part.crop(&ImageRegion::new(0..2, 3..7)).is_none());
        assert!(whole.crop(&ImageRegion::new(0..6, 0..2)).is_none());
        // a region with no rows or no columns is empty
        assert_eq!(
            whole
                .crop(&ImageRegion::new(2..2, 0..12))
                .unwrap()
                .pixels
                .len(),
            0
        );
        assert_eq!(
            whole
                .crop(&ImageRegion::new(0..5, 4..4))
                .unwrap()
                .pixels
                .len(),
            0
        );
    }

    #[test]
    fn a_producer_that_gets_the_count_of_pixels_wrong_is_told() {
        let error = ImageData::new(
            ImageRegion::whole(2, 3),
            ColumnData::U8(Shared::new(vec![0; 5])),
        )
        .unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::InvalidIndex { .. }),
            "{error}"
        );
    }

    fn source() -> InMemorySource {
        let mut source = InMemorySource::new();
        source.insert_image(LayerId(2), ChannelKey::new(["before"]), frame(5, 12));
        source
    }

    fn ask(region: Option<ImageRegion>) -> ImageRequest {
        let request = ImageRequest::new(LayerId(2), ChannelKey::new(["before"]));
        match region {
            Some(region) => request.with_region(region),
            None => request,
        }
    }

    #[test]
    fn a_source_serves_the_whole_frame_or_a_region_and_says_what_it_has_not() {
        let source = source();
        let context = ReadContext::new();
        let whole = source.read_image(&ask(None), &context).expect("the frame");
        assert_eq!(whole.region, ImageRegion::whole(5, 12));
        let part = source
            .read_image(&ask(Some(ImageRegion::new(1..4, 3..7))), &context)
            .expect("a region");
        assert_eq!(part, whole.crop(&ImageRegion::new(1..4, 3..7)).unwrap());

        // a region outside the frame says the frame's dimensions, a reversed one is outside too, and an empty one is not an error
        let outside = source
            .read_image(&ask(Some(ImageRegion::new(0..9, 0..2))), &context)
            .unwrap_err();
        assert!(matches!(outside.kind(), ErrorKind::InvalidIndex { .. }));
        let said = outside.to_string();
        assert!(
            said.contains("5 rows by 12 columns")
                && said.contains("read_image")
                && said.contains("layer 2"),
            "{said}"
        );
        assert!(source
            .read_image(
                &ask(Some(ImageRegion::new(Range { start: 3, end: 2 }, 0..2))),
                &context
            )
            .is_err());
        assert!(source
            .read_image(&ask(Some(ImageRegion::new(2..2, 0..12))), &context)
            .unwrap()
            .pixels
            .is_empty());

        // a layer or a phase that is not there
        let nobody = source
            .read_image(
                &ImageRequest::new(LayerId(9), ChannelKey::new(["before"])),
                &context,
            )
            .unwrap_err();
        assert!(matches!(nobody.kind(), ErrorKind::NotPresent { .. }));
        let other = source
            .read_image(
                &ImageRequest::new(LayerId(2), ChannelKey::new(["after"])),
                &context,
            )
            .unwrap_err();
        assert!(matches!(other.kind(), ErrorKind::NotPresent { .. }));

        // a cancelled token
        let token = CancelToken::new();
        token.cancel();
        let cancelled = source
            .read_image(&ask(None), &ReadContext::new().with_cancel(token))
            .unwrap_err();
        assert!(matches!(cancelled.kind(), ErrorKind::Cancelled));
    }

    #[test]
    fn a_source_that_has_no_frames_says_so_and_its_other_reads_are_unchanged() {
        // a source written before frames were readable: it implements `read` and nothing else
        struct Bare;
        impl ColumnSource for Bare {
            fn read(&self, _: &ColumnRequest, _: &ReadContext) -> Result<ColumnData, Error> {
                Ok(ColumnData::U8(Shared::new(vec![1])))
            }
        }
        let error = Bare
            .read_image(&ask(None), &ReadContext::new())
            .unwrap_err();
        assert!(matches!(
            error.kind(),
            ErrorKind::UnsupportedInVersion { .. }
        ));
        assert!(error.to_string().contains("camera frames"), "{error}");
        let read = Bare
            .read(
                &ColumnRequest::new(LayerId(1), None, ChannelKey::new(["x"])),
                &ReadContext::new(),
            )
            .expect("an ordinary read");
        assert_eq!(read.len(), 1);
    }

    #[test]
    fn a_request_says_where_it_points_for_an_error() {
        let request = ask(None);
        let said = request.location().to_string();
        assert!(
            said.contains("layer 2") && said.contains("before") && said.contains("read_image"),
            "{said}"
        );
        assert_eq!(request.region, None);
        assert_eq!(
            request.with_region(ImageRegion::new(0..1, 0..1)).region,
            Some(ImageRegion::new(0..1, 0..1))
        );
    }
}
