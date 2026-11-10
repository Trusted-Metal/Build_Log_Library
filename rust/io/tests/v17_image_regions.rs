//! The camera frames of V17 (IP-06 phase 6G, task T06.7; decision D06-13): the pixels of a frame, or of a rectangle of it, as stored.
//!
//! * `a_frame_reads_as_stored`: each of the six frames of the 2026-08-18 miniature, read through the source, equals the fingerprint that the independent script wrote, and so does each
//!   rectangle of it that the conformance check reads, and the pixels are those that the file holds when it is read another way.
//! * `a_region_equals_the_slice_of_the_frame_with_rows_and_columns_not_mixed_up`: on a frame that is **not square**, made at test time, a region is the rows and the columns it names, and not their
//!   transpose; seeded rectangles of that frame and of the miniature's equal the crop of the whole frame.
//! * A region outside the frame is `InvalidIndex` and says the frame's dimensions, an empty region is not an error, a cancelled read is `Cancelled` and gives no pixels, progress is reported, and
//!   what the metadata does not have is `NotPresent`. `ImageRegions` is declared. The 25.7 MB frames of a real build are read by region when the large local fixture is there.

mod common;

use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use build_log_conformance::{check_image_regions, frame_expectations, load_all};
use build_log_io::adapters::v17::source::V17Source;
use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildSource, CancelToken, ChannelKey, ColumnData, ColumnRequest, ColumnSource, Direction,
    Error, ErrorKind, Feature, ImageData, ImageRegion, ImageRequest, LayerId, ReadContext,
};
use build_log_testkit::{large_fixture_or_skip, miniature, miniature_path};
use common::{layer, monitoring, pick_any, put_text, temp_h5, Rng};
use hdf5::{File, Group};

/// A source seen as the column source it is, so that the conformance check can hold it to the fingerprints.
struct Through<'a>(&'a dyn BuildSource);

impl ColumnSource for Through<'_> {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        self.0.read(request, context)
    }

    fn read_image(
        &self,
        request: &ImageRequest,
        context: &ReadContext,
    ) -> Result<ImageData, Error> {
        self.0.read_image(request, context)
    }
}

fn phase(name: &str) -> ChannelKey {
    ChannelKey::new([name])
}

fn pixels_of(data: &ImageData) -> Vec<u8> {
    match &data.pixels {
        ColumnData::U8(v) => v.as_slice().to_vec(),
        other => panic!("the pixels are not bytes: {other:?}"),
    }
}

// ---------------------------------------------------------------- the miniature

#[test]
fn a_frame_reads_as_stored() {
    let name = miniature::V17_2026_08_18_IMAGES;
    let opened = open(production(), miniature_path(name)).expect("the images file opens");
    let expectations = load_all().expect("the expectation files");
    let expectation = expectations
        .iter()
        .find(|e| e.miniature() == name)
        .expect("the expectation of the images file");
    // each frame equals its fingerprint, and so does each rectangle of it that the check reads
    let lines = check_image_regions(&Through(opened.source()), expectation);
    assert!(lines.is_empty(), "{}", lines.join("\n  "));
    assert_eq!(
        frame_expectations(expectation).expect("the frames").len(),
        6
    );

    // a frame is 128 rows by 128 columns of bytes, and the file holds the same pixels when it is read another way
    let whole = opened
        .source()
        .read_image(
            &ImageRequest::new(LayerId(1), phase(layout::PRE_EXPOSURE)),
            &ReadContext::new(),
        )
        .expect("the frame");
    assert_eq!(whole.region, ImageRegion::whole(128, 128));
    assert!(matches!(whole.pixels, ColumnData::U8(_)));
    assert_eq!(whole.pixels.len(), 128 * 128);
    let file = File::open(miniature_path(name)).expect("the file");
    let frame = file
        .dataset(&format!(
            "{}/{}/{}1/{}/{}",
            layout::MOUNT,
            layout::MONITORING_DATA,
            layout::LAYER_PREFIX,
            layout::PRE_EXPOSURE,
            layout::RAW
        ))
        .expect("the frame of layer 1");
    assert_eq!(
        pixels_of(&whole),
        frame.read_raw::<u8>().expect("its pixels")
    );
}

// ---------------------------------------------------------------- frames made at test time

/// A source over a file made at test time: `write` fills the layers' group, and the build is read from it.
fn source_over(label: &str, write: impl FnOnce(&Group)) -> (common::TempH5, File, V17Source) {
    let on_disk = temp_h5(label, |mount| {
        put_text(mount, layout::BUILD_ID, "a build");
        write(&monitoring(mount));
    });
    let file = File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let build = v17::read::read_build(&mount).expect("the build reads");
    (on_disk, file, V17Source::new(build, mount))
}

/// A frame of `rows` by `columns` bytes in the image group `phase` of a layer.
fn put_frame(layer: &Group, phase: &str, rows: usize, columns: usize, pixels: &[u8]) {
    layer
        .create_group(phase)
        .and_then(|holder| {
            holder
                .new_dataset::<u8>()
                .shape([rows, columns])
                .create(layout::RAW)
        })
        .expect("a frame")
        .write_raw(pixels)
        .expect("its pixels");
}

/// The pixels of a frame of `rows` by `columns`: each is a different number along the rows and along the columns, and the frame is not symmetric.
fn pattern(rows: usize, columns: usize) -> Vec<u8> {
    (0..rows)
        .flat_map(|r| (0..columns).map(move |c| ((r * 31 + c * 7) % 251) as u8))
        .collect()
}

/// The rows `rows` and the columns `columns` of a frame of `width` columns, taken by hand.
fn by_hand(
    frame: &[u8],
    width: usize,
    rows: std::ops::Range<usize>,
    columns: std::ops::Range<usize>,
) -> Vec<u8> {
    let mut out = Vec::new();
    for r in rows {
        for c in columns.clone() {
            out.push(frame[r * width + c]);
        }
    }
    out
}

fn ask(layer: u32, name: &str) -> ImageRequest {
    ImageRequest::new(LayerId(layer), phase(name))
}

fn region(rows: std::ops::Range<u64>, columns: std::ops::Range<u64>) -> ImageRegion {
    ImageRegion::new(rows, columns)
}

#[test]
fn a_region_equals_the_slice_of_the_frame_with_rows_and_columns_not_mixed_up() {
    // seven rows by thirteen columns: not square, so a region read the other way round would be another one
    let frame = pattern(7, 13);
    let (_disk, _file, source) = source_over("frames-nonsquare", |layers| {
        put_frame(&layer(layers, 1), layout::PRE_EXPOSURE, 7, 13, &frame);
    });
    let context = ReadContext::new();
    let got = source
        .read_image(
            &ask(1, layout::PRE_EXPOSURE).with_region(region(2..5, 3..11)),
            &context,
        )
        .expect("a region");
    assert_eq!((got.rows(), got.columns()), (3, 8));
    assert_eq!(got.region, region(2..5, 3..11));
    assert_eq!(pixels_of(&got), by_hand(&frame, 13, 2..5, 3..11));
    // the transpose of that rectangle (eight rows from the third, three columns from the fourth) is not the same pixels, and is not even inside this frame
    let turned = source
        .read_image(
            &ask(1, layout::PRE_EXPOSURE).with_region(region(3..11, 2..5)),
            &context,
        )
        .unwrap_err();
    assert!(
        matches!(turned.kind(), ErrorKind::InvalidIndex { .. }),
        "{turned}"
    );
    // a region of one row, one column and one pixel, at the corners
    for (rows, columns) in [(0..1, 0..13), (0..7, 12..13), (6..7, 12..13), (0..1, 0..1)] {
        let part = source
            .read_image(
                &ask(1, layout::PRE_EXPOSURE).with_region(region(rows.clone(), columns.clone())),
                &context,
            )
            .expect("a region at the edge");
        assert_eq!(
            pixels_of(&part),
            by_hand(
                &frame,
                13,
                rows.start as usize..rows.end as usize,
                columns.start as usize..columns.end as usize
            )
        );
    }
}

#[test]
fn seeded_rectangles_equal_the_crop_of_the_whole_frame() {
    let mut rng = Rng(0x00c0_ffee);
    let mut checked = 0;

    // the frame made at test time, which is not square
    let frame = pattern(23, 41);
    let (_disk, _file, source) = source_over("frames-property", |layers| {
        put_frame(&layer(layers, 1), layout::PRE_EXPOSURE, 23, 41, &frame);
    });
    let whole = source
        .read_image(&ask(1, layout::PRE_EXPOSURE), &ReadContext::new())
        .expect("the frame");
    for _ in 0..150 {
        let (rows, columns) = (pick_any(&mut rng, 23), pick_any(&mut rng, 41));
        let wanted = region(
            rows.start as u64..rows.end as u64,
            columns.start as u64..columns.end as u64,
        );
        let got = source
            .read_image(
                &ask(1, layout::PRE_EXPOSURE).with_region(wanted.clone()),
                &ReadContext::new(),
            )
            .expect("a rectangle");
        assert_eq!(got, whole.crop(&wanted).expect("a crop"), "{wanted:?}");
        assert_eq!(
            pixels_of(&got),
            by_hand(&frame, 41, rows, columns),
            "{wanted:?}"
        );
        checked += 1;
    }

    // the six frames of the miniature
    let opened = open(
        production(),
        miniature_path(miniature::V17_2026_08_18_IMAGES),
    )
    .expect("the images file opens");
    for layer_id in 1..=3 {
        for name in [layout::PRE_EXPOSURE, layout::POST_EXPOSURE] {
            let whole = opened
                .source()
                .read_image(&ask(layer_id, name), &ReadContext::new())
                .expect("a frame");
            for _ in 0..30 {
                let (rows, columns) = (pick_any(&mut rng, 128), pick_any(&mut rng, 128));
                let wanted = region(
                    rows.start as u64..rows.end as u64,
                    columns.start as u64..columns.end as u64,
                );
                let got = opened
                    .source()
                    .read_image(
                        &ask(layer_id, name).with_region(wanted.clone()),
                        &ReadContext::new(),
                    )
                    .expect("a rectangle");
                assert_eq!(
                    got,
                    whole.crop(&wanted).expect("a crop"),
                    "layer {layer_id} {name} {wanted:?}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 300, "{checked} rectangles");
}

#[test]
fn a_region_outside_the_frame_is_invalid_index_and_an_empty_region_is_not_an_error() {
    let (_disk, _file, source) = source_over("frames-outside", |layers| {
        put_frame(
            &layer(layers, 1),
            layout::PRE_EXPOSURE,
            7,
            13,
            &pattern(7, 13),
        );
    });
    let context = ReadContext::new();
    let read = |rows: std::ops::Range<u64>, columns: std::ops::Range<u64>| {
        source.read_image(
            &ask(1, layout::PRE_EXPOSURE).with_region(region(rows, columns)),
            &context,
        )
    };
    // past the rows, past the columns, the transpose of the whole frame, and a range that ends before it starts
    for (rows, columns) in [
        (0..8, 0..13),
        (0..7, 0..14),
        (0..13, 0..7),
        (Range { start: 5, end: 3 }, 0..13),
        (0..7, Range { start: 9, end: 2 }),
    ] {
        let error = read(rows.clone(), columns.clone()).unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::InvalidIndex { .. }),
            "{rows:?} {columns:?}: {error}"
        );
        let said = error.to_string();
        assert!(
            said.contains("7 rows by 13 columns"),
            "the frame's dimensions: {said}"
        );
        assert!(
            said.contains("layer 1") && said.contains("read_image"),
            "{said}"
        );
    }
    // a region of no rows or no columns is empty, wherever it is in the frame, and holds no pixel
    for (rows, columns) in [(3..3, 0..13), (0..7, 5..5), (7..7, 13..13), (0..0, 0..0)] {
        let empty = read(rows.clone(), columns.clone()).expect("an empty region");
        assert_eq!(empty.pixels.len(), 0, "{rows:?} {columns:?}");
        assert!(
            matches!(empty.pixels, ColumnData::U8(_)),
            "the type is the stored one even when there are no pixels"
        );
    }
}

#[test]
fn a_cancelled_frame_read_returns_cancelled() {
    // a frame of 400 by 400 bytes, which the ranged reader reads in blocks of whole rows (at most 10,000 pixels each)
    let (_disk, _file, source) = source_over("frames-cancel", |layers| {
        put_frame(
            &layer(layers, 1),
            layout::PRE_EXPOSURE,
            400,
            400,
            &pattern(400, 400),
        );
    });
    let request = ask(1, layout::PRE_EXPOSURE);

    // a token that has fired before the read gives no pixels
    let fired = CancelToken::new();
    fired.cancel();
    let error = source
        .read_image(&request, &ReadContext::new().with_cancel(fired))
        .unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error}");
    let said = error.to_string();
    assert!(
        said.contains("layer 1") && said.contains("read_image"),
        "{said}"
    );

    // a read that is stopped between blocks returns Cancelled, and the progress it reported stopped short of the whole
    let token = CancelToken::new();
    let seen: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let trigger = token.clone();
    let context = ReadContext::new()
        .with_cancel(token)
        .with_progress(Arc::new(move |done, total| {
            sink.lock().expect("a lock").push((done, total));
            if done >= 10_000 {
                trigger.cancel();
            }
        }));
    let stopped = source.read_image(&request, &context).unwrap_err();
    assert!(matches!(stopped.kind(), ErrorKind::Cancelled), "{stopped}");
    let seen = seen.lock().expect("a lock").clone();
    assert!(
        !seen.is_empty()
            && seen
                .iter()
                .all(|&(done, total)| total == 160_000 && done < total),
        "{seen:?}"
    );

    // and a read that is not stopped reports its progress in order, up to the whole frame
    let seen: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let whole = source
        .read_image(
            &request,
            &ReadContext::new().with_progress(Arc::new(move |done, total| {
                sink.lock().expect("a lock").push((done, total));
            })),
        )
        .expect("the frame");
    assert_eq!(whole.pixels.len(), 160_000);
    let seen = seen.lock().expect("a lock").clone();
    assert!(seen.len() > 1, "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0].0 < w[1].0), "{seen:?}");
    assert_eq!(*seen.last().unwrap(), (160_000, 160_000));
}

// ---------------------------------------------------------------- what V17 says about a request

#[test]
fn what_a_request_gets_wrong_is_a_typed_error_that_says_which_frame() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_2026_08_18_IMAGES),
    )
    .expect("the images file opens");
    let source = opened.source();
    let context = ReadContext::new();
    // a phase that is not there, a channel of the build that is not an image, and a layer that has no frame
    for (request, what) in [
        (ask(1, "Mid_Exposure"), "a phase that is not a channel"),
        (ask(1, "Laser_Power"), "a laser channel is not an image"),
        (ask(99, layout::PRE_EXPOSURE), "a layer that has no frame"),
    ] {
        let error = source.read_image(&request, &context).unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{what}: {error}"
        );
        assert!(error.to_string().contains("read_image"), "{what}: {error}");
    }
    let nobody = source
        .read_image(&ask(99, layout::PRE_EXPOSURE), &context)
        .unwrap_err();
    assert!(nobody.to_string().contains("layer 99"), "{nobody}");
}

#[test]
fn a_frame_the_metadata_has_and_the_file_lacks_is_corrupt() {
    // the metadata is read from a file that has the frame, and the source reads another, whose mount does not hold it
    let with = source_over("frames-with", |layers| {
        put_frame(
            &layer(layers, 1),
            layout::PRE_EXPOSURE,
            4,
            6,
            &pattern(4, 6),
        );
    });
    let without = source_over("frames-without", |layers| {
        layer(layers, 1);
    });
    let build = (*with.2.build().expect("the metadata")).clone();
    let source = V17Source::new(build, without.1.group("Mount").expect("the mount"));
    let error = source
        .read_image(&ask(1, layout::PRE_EXPOSURE), &ReadContext::new())
        .unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Corrupt { .. }), "{error}");
    assert!(error.to_string().contains("layer 1"), "{error}");
}

#[test]
fn image_regions_are_declared_in_the_capabilities() {
    let set = common::v17_set();
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::ImageRegions));
    // reading is declared and writing is not: the writer arrives with IP-08
    assert!(!set
        .capabilities()
        .supports(Direction::Write, Feature::ImageRegions));
    // the frames themselves are still described under `Images`
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Images));
}

// ---------------------------------------------------------------- a real frame

#[test]
fn a_real_frame_reads_by_region_when_the_large_fixture_is_there() {
    // the frames of a real build are 5068 rows by 5064 columns of bytes, 25.7 MB, one chunk each: not square, which a mix-up of rows and columns would show
    let path = large_fixture_or_skip!("Build Log Fixtures/V17_2026_08_18_with_images.h5");
    let opened = open(production(), &path).expect("the real file opens");
    let source = opened.source();
    let request = ask(1, layout::PRE_EXPOSURE);
    let context = ReadContext::new();

    let started = Instant::now();
    let whole = source.read_image(&request, &context).expect("the frame");
    let whole_took = started.elapsed();
    assert_eq!(whole.region, ImageRegion::whole(5068, 5064));
    assert_eq!(whole.pixels.len(), 5068 * 5064);

    // a region is the rectangle of that frame, rows first: the same pixels as the crop, and as the file holds them
    let wanted = region(4000..4100, 100..3000);
    let started = Instant::now();
    let part = source
        .read_image(&request.clone().with_region(wanted.clone()), &context)
        .expect("a region");
    let part_took = started.elapsed();
    assert_eq!(part, whole.crop(&wanted).expect("a crop"));
    assert_eq!((part.rows(), part.columns()), (100, 2900));
    // the rectangle with the same numbers turned the other way round is inside this frame too, and is other pixels
    let other = source
        .read_image(
            &request.clone().with_region(region(100..3000, 4000..4100)),
            &context,
        )
        .expect("the transposed rectangle is inside this frame");
    assert_ne!(other.pixels, part.pixels);
    // one region of a one-chunk frame decodes the chunk (decision G17): it is timed, not bounded
    eprintln!(
        "a real frame, 5068 by 5064 bytes: the whole frame {whole_took:?}, a region of 100 by 2900 {part_took:?}"
    );
}
