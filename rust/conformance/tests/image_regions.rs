//! The fingerprints of frames and the check of a column source against them (IP-06 phase 6G, task T06.7, decision D06-13).
//!
//! The expectation file of the one miniature that holds camera images carries a fingerprint for every frame. The check is proved here on a source made of synthetic frames (not square, and not
//! symmetric, so that a rows-and-columns mix-up changes them): a source that serves them as stored passes, and each way of serving one wrongly is reported, on that frame and on no other.

use build_log_conformance::{
    check_image_regions, frame_expectations, load_all, Expectation, Fingerprint,
};
use build_log_model::{
    ChannelKey, ColumnData, ColumnRequest, ColumnSource, Error, ImageData, ImageRegion,
    ImageRequest, InMemorySource, LayerId, ReadContext, Shared,
};
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------- known answers

// Worked out with the standard library alone in Python (`hashlib`, `struct`); the test of the oracle holds the same literals, so that the two languages agree on what the bytes are and
// in which order: the pixels, row after row.
const FRAME_2_BY_3: &str = "7192385c3c0605de55bb9476ce1d90748190ecb32a8eed7f5207b30cf6a1fe89";
const FRAME_3_BY_2_TRANSPOSED: &str =
    "10368eb6ee9375eb0425110bdc663548bff2a9d34219eb78c815af87bb8b1ae2";
const FRAME_U16_1_256: &str = "afa7518106309c22d325df6d2663249d158d2f36f1976269d6d4104d9198a108";

fn u8_frame(rows: u64, columns: u64, pixels: Vec<u8>) -> ImageData {
    ImageData::new(
        ImageRegion::whole(rows, columns),
        ColumnData::U8(Shared::new(pixels)),
    )
    .expect("a frame")
}

#[test]
fn the_fingerprint_of_known_frames_is_what_hashlib_gives() {
    let frame = u8_frame(2, 3, vec![1, 2, 3, 4, 5, 6]);
    let print = Fingerprint::of(&frame.pixels).expect("numbers");
    assert_eq!(print.sha256, FRAME_2_BY_3);
    assert_eq!(
        (print.count, print.stored_as.as_str(), print.min, print.max),
        (6, "U8", Some(1.0), Some(6.0))
    );
    assert_eq!(
        (print.first, print.last),
        (vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0])
    );
    // the same pixels the other way round (three rows of two) are another frame: the bytes are 1 4 2 5 3 6
    let turned = u8_frame(3, 2, vec![1, 4, 2, 5, 3, 6]);
    assert_eq!(
        Fingerprint::of(&turned.pixels).unwrap().sha256,
        FRAME_3_BY_2_TRANSPOSED
    );
    // a wider pixel is its little-endian bytes
    let wide = ImageData::new(
        ImageRegion::whole(1, 2),
        ColumnData::U16(Shared::new(vec![1, 256])),
    )
    .expect("a frame");
    assert_eq!(
        Fingerprint::of(&wide.pixels).unwrap().sha256,
        FRAME_U16_1_256
    );
}

// ---------------------------------------------------------------- a synthetic build: two phases, two layers, frames of 9 rows by 14 columns

const LAYERS: [u32; 2] = [1, 2];
const PHASES: [&str; 2] = ["Pre_Exposure", "Post_Exposure"];
const ROWS: u64 = 9;
const COLUMNS: u64 = 14;

/// The frame of a layer and a phase: a pixel is a different number in every place, and the frame is not symmetric.
fn frame_of(layer: u32, phase: &str) -> ImageData {
    let seed = layer as u64 * 17 + phase.len() as u64 * 5;
    let pixels = (0..ROWS)
        .flat_map(|r| (0..COLUMNS).map(move |c| ((r * 31 + c * 7 + seed * (r + 1)) % 251) as u8))
        .collect();
    u8_frame(ROWS, COLUMNS, pixels)
}

fn channel(phase: &str) -> ChannelKey {
    ChannelKey::new([phase])
}

/// The source that serves every frame as stored; `change` may serve one differently, or not at all (`None`).
fn source_with(change: impl Fn(u32, &str, ImageData) -> Option<ImageData>) -> InMemorySource {
    let mut source = InMemorySource::new();
    for layer in LAYERS {
        for phase in PHASES {
            if let Some(data) = change(layer, phase, frame_of(layer, phase)) {
                source.insert_image(LayerId(layer), channel(phase), data);
            }
        }
    }
    source
}

/// An expectation that describes the frames as stored; `described` says whether each carries its fingerprint.
fn expectation(described: bool) -> Expectation {
    let sensors: Vec<Value> = PHASES
        .iter()
        .map(|&phase| {
            let frames: Vec<Value> = LAYERS
                .iter()
                .map(|&layer| {
                    let data = frame_of(layer, phase);
                    let mut frame = Map::new();
                    frame.insert("layer".into(), json!(layer));
                    frame.insert("rows".into(), json!(ROWS));
                    frame.insert("columns".into(), json!(COLUMNS));
                    for (key, value) in Fingerprint::of(&data.pixels)
                        .unwrap()
                        .to_json()
                        .as_object()
                        .unwrap()
                    {
                        frame.insert(key.clone(), value.clone());
                    }
                    Value::Object(frame)
                })
                .collect();
            let mut sensor = json!({
                "name": phase,
                "layers": LAYERS,
                "clock_class": "Image",
                "image": {"rows": ROWS, "columns": COLUMNS, "pixel": "U8"},
            });
            if described {
                sensor["frames"] = json!(frames);
            }
            sensor
        })
        .collect();
    Expectation::parse(
        &json!({"expectations_version": 1, "file_version": 17, "sensors": sensors}).to_string(),
    )
    .expect("an expectation")
}

fn checked(source: &dyn ColumnSource) -> Vec<String> {
    check_image_regions(source, &expectation(true))
}

/// What the check says when the frame of `layer` and `phase` is served as `change` says and every other frame as stored; every line must be about that frame.
fn broken(layer: u32, phase: &str, change: impl Fn(ImageData) -> Option<ImageData>) -> Vec<String> {
    let source = source_with(|l, p, data| {
        if (l, p) == (layer, phase) {
            change(data)
        } else {
            Some(data)
        }
    });
    let found = checked(&source);
    assert!(
        !found.is_empty(),
        "nothing was reported for layer {layer}, {phase}"
    );
    let who = format!("layer {layer}, {phase}: ");
    for line in &found {
        assert!(line.starts_with(&who), "a line about another frame: {line}");
    }
    found
}

fn has(found: &[String], what: &str) -> bool {
    found.iter().any(|line| line.contains(what))
}

/// The pixels of a frame, as a plain list.
fn pixels_of(data: &ImageData) -> Vec<u8> {
    match &data.pixels {
        ColumnData::U8(v) => v.as_slice().to_vec(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_synthetic_source_with_frames_passes_and_a_changed_one_fails() {
    let expected = expectation(true);
    assert_eq!(
        checked(&source_with(|_, _, d| Some(d))),
        Vec::<String>::new()
    );
    assert_eq!(frame_expectations(&expected).expect("four frames").len(), 4);

    // one pixel that differs, in the middle, is found by the hash and by nothing else
    let found = broken(2, "Pre_Exposure", |mut d| {
        let mut pixels = pixels_of(&d);
        pixels[60] ^= 1;
        d.pixels = ColumnData::U8(Shared::new(pixels));
        Some(d)
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(has(&found, "the SHA-256 of the values"));

    // a frame served the other way round: nine by fourteen read as fourteen by nine, in the same order and in column order
    let found = broken(1, "Post_Exposure", |d| {
        let mut turned = Vec::new();
        let pixels = pixels_of(&d);
        for c in 0..COLUMNS as usize {
            for r in 0..ROWS as usize {
                turned.push(pixels[r * COLUMNS as usize + c]);
            }
        }
        Some(u8_frame(COLUMNS, ROWS, turned))
    });
    assert!(has(&found, "the frame covers 14 rows by 9 columns"));
    assert!(has(&found, "the SHA-256 of the values"));
    // the right pixels with the dimensions the wrong way round
    let found = broken(1, "Post_Exposure", |d| {
        Some(u8_frame(COLUMNS, ROWS, pixels_of(&d)))
    });
    assert!(has(
        &found,
        "the frame covers 14 rows by 9 columns, expected 9 by 14"
    ));
    assert!(!has(&found, "the SHA-256"));

    // another stored type, a frame that is not served, and the frame of another layer
    let found = broken(2, "Post_Exposure", |d| {
        let wide: Vec<u16> = pixels_of(&d).into_iter().map(u16::from).collect();
        Some(ImageData::new(d.region.clone(), ColumnData::U16(Shared::new(wide))).unwrap())
    });
    assert!(has(&found, "the stored type is U16"));
    let found = broken(1, "Pre_Exposure", |_| None);
    assert_eq!(found.len(), 1);
    assert!(has(&found, "the frame was not read"));
    let found = broken(1, "Pre_Exposure", |_| Some(frame_of(2, "Pre_Exposure")));
    assert!(has(&found, "the SHA-256 of the values"));
}

/// How a source gets a rectangle wrong: the region it serves for the one asked for, in a frame of this many rows and columns.
type WrongRegion = fn(&ImageRegion, u64, u64) -> ImageRegion;

/// A source that serves what its inner source does, and a rectangle as `wrong` says.
struct Wrong {
    inner: InMemorySource,
    wrong: WrongRegion,
}

impl ColumnSource for Wrong {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        self.inner.read(request, context)
    }

    fn read_image(
        &self,
        request: &ImageRequest,
        context: &ReadContext,
    ) -> Result<ImageData, Error> {
        let mut request = request.clone();
        if let Some(region) = &request.region {
            request.region = Some((self.wrong)(region, ROWS, COLUMNS));
        }
        self.inner.read_image(&request, context)
    }
}

#[test]
fn a_source_that_serves_the_whole_frame_right_and_a_region_wrong_is_found() {
    let wrongs: [(&str, WrongRegion); 4] = [
        // rows and columns mixed up: the region's rows are taken as its columns
        ("rows and columns mixed up", |r, rows, columns| {
            ImageRegion::new(
                r.columns.start.min(rows)..r.columns.end.min(rows),
                r.rows.start.min(columns)..r.rows.end.min(columns),
            )
        }),
        // the region shifted by one row
        ("a region shifted by a row", |r, rows, _| {
            ImageRegion::new(
                (r.rows.start + 1).min(rows)..(r.rows.end + 1).min(rows),
                r.columns.clone(),
            )
        }),
        // a region that is its frame's whole width whatever the columns asked for
        ("the columns ignored", |r, _, columns| {
            ImageRegion::new(r.rows.clone(), 0..columns)
        }),
        // a region of no pixels served as the whole frame
        (
            "an empty region served as the whole frame",
            |r, rows, columns| {
                if r.pixels() == 0 {
                    ImageRegion::whole(rows, columns)
                } else {
                    r.clone()
                }
            },
        ),
    ];
    for (what, wrong) in wrongs {
        let source = Wrong {
            inner: source_with(|_, _, d| Some(d)),
            wrong,
        };
        let found = checked(&source);
        assert!(!found.is_empty(), "{what} was not found");
        // the whole frames are right: nothing is said about their hash or their dimensions
        assert!(
            found
                .iter()
                .all(|l| !l.contains("SHA-256") && !l.contains("the frame covers")),
            "{what}: {found:?}"
        );
        // and every frame is reported, each on its own lines
        for layer in LAYERS {
            for phase in PHASES {
                assert!(
                    found
                        .iter()
                        .any(|l| l.starts_with(&format!("layer {layer}, {phase}: "))),
                    "{what}: nothing about layer {layer}, {phase}"
                );
            }
        }
    }
}

#[test]
fn an_expectation_that_does_not_describe_a_frame_is_an_error_and_not_a_pass() {
    // a channel with no fingerprints is an error, and the check says so as one line
    let bare = expectation(false);
    assert!(frame_expectations(&bare)
        .unwrap_err()
        .contains("no fingerprints"));
    assert_eq!(
        check_image_regions(&source_with(|_, _, d| Some(d)), &bare).len(),
        1
    );
    // a channel that lists a layer for which it has no fingerprint is an error that names the layer
    let mut root: Value = json!({"expectations_version": 1, "file_version": 17, "sensors": [{
        "name": "Pre_Exposure", "layers": [1, 2], "image": {"rows": 1, "columns": 1, "pixel": "U8"},
        "frames": []
    }]});
    let error = frame_expectations(&Expectation::parse(&root.to_string()).unwrap()).unwrap_err();
    assert!(error.contains("Pre_Exposure, layer 1"), "{error}");
    // a fingerprint that is not what the format says names the frame
    root["sensors"][0]["frames"] = json!([
        {"layer": 1, "rows": 1, "columns": 1, "count": "one"},
        {"layer": 2, "rows": 1, "columns": 1, "count": 1}
    ]);
    let error = frame_expectations(&Expectation::parse(&root.to_string()).unwrap()).unwrap_err();
    assert!(
        error.contains("layer 1") && error.contains("count"),
        "{error}"
    );
    // an expectation with no image channel describes none
    let none = Expectation::parse(
        &json!({"expectations_version": 1, "file_version": 16, "sensors": []}).to_string(),
    )
    .unwrap();
    assert!(frame_expectations(&none).unwrap().is_empty());
    assert!(check_image_regions(&InMemorySource::new(), &none).is_empty());
}

#[test]
fn the_miniatures_describe_every_frame_with_its_fingerprint() {
    let all = load_all().expect("the expectation files");
    let mut with_frames = Vec::new();
    for expectation in &all {
        let frames = frame_expectations(expectation).expect("every frame carries its fingerprint");
        if !frames.is_empty() {
            with_frames.push((expectation.miniature().to_string(), frames));
        }
    }
    assert_eq!(with_frames.len(), 1);
    let (name, frames) = &with_frames[0];
    assert_eq!(name, "v17_2026_08_18_images.h5");
    // two phases in three layers, each a frame of 128 by 128 bytes
    assert_eq!(frames.len(), 6);
    for frame in frames {
        assert_eq!((frame.rows, frame.columns), (128, 128));
        assert_eq!(frame.fingerprint.count, 128 * 128);
        assert_eq!(frame.fingerprint.stored_as, "U8");
    }
}
