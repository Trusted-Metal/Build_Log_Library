//! The fingerprint of a frame, and the check of a column source against the fingerprints of an expectation (IP-06 phase 6G, task T06.7, decision D06-13).
//!
//! A V17 expectation file carries, for each frame of each image channel of the build (a phase, in the layers that have it), a **fingerprint of its pixels**, written by the independent script
//! from the miniature alone: its rows and columns, the stored type, the minimum and the maximum, the first and the last three pixels **in row-major order** and the SHA-256 of the pixels'
//! bytes in row-major order. [`check_image_regions`] reads each of those frames from a [`ColumnSource`] and says every way in which what it got is not what the fingerprint says, and then
//! reads **rectangles of the frame** and says every way in which one is not the same rectangle of the frame it just read.
//!
//! * The fingerprint of a frame is a [`Fingerprint`] of its pixels, taken **row by row**: the same hash as the script's. A frame read the other way round (columns first) has another hash, so a
//!   rows-and-columns mix-up is found by it, on a frame that is not square or that is not symmetric; the miniature's frames are square but not symmetric.
//! * The rectangles are fixed ones of each frame: its first row, its last column, its middle, its last pixel and an empty rectangle. Each must equal the crop that the Model makes of the whole
//!   frame, so that a source cannot pass by serving the whole frame right and the regions wrong.
//! * The expectation holds no pixel but the first and the last three, so that the images stay bulk data.

use build_log_model::{ChannelKey, ColumnSource, ImageRegion, ImageRequest, LayerId, ReadContext};
use serde_json::Value;

use crate::columns::{differences, Fingerprint};
use crate::expectation::{array, int, text};
use crate::Expectation;

/// One frame that an expectation describes: where it is, its dimensions and the fingerprint of its pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameExpectation {
    pub layer: LayerId,
    /// The key of the build's image channel, which is the phase's name.
    pub channel: ChannelKey,
    pub rows: u64,
    pub columns: u64,
    pub fingerprint: Fingerprint,
}

impl FrameExpectation {
    /// Where the frame is, for a message: `layer 2, Pre_Exposure`.
    fn whose(&self) -> String {
        format!(
            "layer {}, {}",
            self.layer.0,
            self.channel.name().unwrap_or("")
        )
    }
}

/// Every frame that an expectation describes. **Every layer of an image channel must have its frame**: a channel that lists a layer with no fingerprint for it is an error, so that a check
/// cannot pass by having nothing to check. An expectation with no image channel describes none.
pub fn frame_expectations(expectation: &Expectation) -> Result<Vec<FrameExpectation>, String> {
    let mut found = Vec::new();
    for sensor in array(expectation.root(), "sensors")? {
        if sensor.get("image").is_none_or(Value::is_null) {
            continue;
        }
        let name = text(sensor, "name")?;
        let frames = array(sensor, "frames").map_err(|_| {
            format!(
                "{name}: the expectation has no fingerprints for the frames of an image channel"
            )
        })?;
        for layer in array(sensor, "layers")? {
            let id = layer
                .as_u64()
                .ok_or_else(|| format!("{name}: a layer is not a whole number"))?;
            let frame = frames
                .iter()
                .find(|f| f.get("layer").and_then(Value::as_u64) == Some(id))
                .ok_or_else(|| {
                    format!("{name}, layer {id}: the expectation has no fingerprint for this frame")
                })?;
            found.push(FrameExpectation {
                layer: LayerId(id as u32),
                channel: ChannelKey::new([name]),
                rows: int(frame, "rows")? as u64,
                columns: int(frame, "columns")? as u64,
                fingerprint: Fingerprint::from_json(frame)
                    .map_err(|e| format!("{name}, layer {id}: {e}"))?,
            });
        }
    }
    Ok(found)
}

/// The rectangles of a frame of `rows` by `columns` that the check reads: a row, a column, the middle, a pixel and nothing.
fn rectangles(rows: u64, columns: u64) -> Vec<(&'static str, ImageRegion)> {
    if rows == 0 || columns == 0 {
        return Vec::new();
    }
    vec![
        ("the first row", ImageRegion::new(0..1, 0..columns)),
        (
            "the last column",
            ImageRegion::new(0..rows, columns - 1..columns),
        ),
        (
            "the middle",
            ImageRegion::new(
                rows / 4..rows - rows / 4,
                columns / 4..columns - columns / 4,
            ),
        ),
        (
            "the last pixel",
            ImageRegion::new(rows - 1..rows, columns - 1..columns),
        ),
        (
            "an empty rectangle",
            ImageRegion::new(rows / 2..rows / 2, 0..columns),
        ),
    ]
}

/// Every way in which what `source` serves is not what the fingerprints of `expectation` say, as lines that name the layer and the phase.
///
/// Each described frame is read whole with a context that never cancels; one that cannot be read is reported with the error. Then each rectangle of [`rectangles`] is read and must be the same
/// rectangle of the whole frame. An expectation that cannot be read as one that describes frames is a single line.
pub fn check_image_regions(source: &dyn ColumnSource, expectation: &Expectation) -> Vec<String> {
    let frames = match frame_expectations(expectation) {
        Ok(frames) => frames,
        Err(problem) => return vec![problem],
    };
    let context = ReadContext::new();
    let mut found = Vec::new();
    for frame in &frames {
        let who = frame.whose();
        let request = ImageRequest::new(frame.layer, frame.channel.clone());
        let whole = match source.read_image(&request, &context) {
            Err(error) => {
                found.push(format!("{who}: the frame was not read: {error}"));
                continue;
            }
            Ok(whole) => whole,
        };
        let described = ImageRegion::whole(frame.rows, frame.columns);
        if whole.region != described {
            found.push(format!(
                "{who}: the frame covers {} rows by {} columns, expected {} by {}",
                whole.rows(),
                whole.columns(),
                frame.rows,
                frame.columns
            ));
        }
        match Fingerprint::of(&whole.pixels) {
            None => found.push(format!("{who}: the pixels are not numbers")),
            Some(actual) => differences(&who, &actual, &frame.fingerprint, &mut found),
        }
        for (what, rectangle) in rectangles(frame.rows, frame.columns) {
            let Some(expected) = whole.crop(&rectangle) else {
                continue;
            };
            match source.read_image(&request.clone().with_region(rectangle.clone()), &context) {
                Err(error) => found.push(format!("{who}: {what} was not read: {error}")),
                Ok(got) if got != expected => found.push(format!(
                    "{who}: {what} ({}..{} rows, {}..{} columns) is not the same rectangle of the frame",
                    rectangle.rows.start, rectangle.rows.end, rectangle.columns.start, rectangle.columns.end
                )),
                Ok(_) => {}
            }
        }
    }
    found
}
