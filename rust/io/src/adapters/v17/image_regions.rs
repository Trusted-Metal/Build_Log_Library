//! Where the frames of V17 are stored, and how a region of one is read (IP-06 phase 6G, task T06.7; decision D06-13).
//!
//! The metadata described the frames in phase 13F (`super::images`): each phase of a layer's images is a channel of the whole build, recorded in the layers that have a frame. This file reads their
//! **pixels**.
//!
//! * **A frame is the dataset `raw` of the image group of a phase** in a layer's group (`Layer_M/Pre_Exposure/raw` or `Layer_M/Post_Exposure/raw`). The channel's key is the phase's name.
//! * **The metadata answers first** (D06-9): a channel that the build does not have, one that is not an image, or a layer that has no frame of it, is `NotPresent` and the file is not touched. A frame
//!   that the metadata says is recorded and the file does not hold is `Corrupt`.
//! * **The region is read by the column engine's frame reader** (`crate::columns::read_frame`): the rules for the region, the two-dimensional ranged read, cancel and progress, and the typing by what
//!   is stored are the engine's and the same for every version. A frame is not cached.
//!
//! Every name comes from [`super::layout`].

use build_log_model::{
    ChannelKey, ChannelKind, Error, ErrorKind, ImageData, ImageRequest, Presence,
};
use hdf5::dataset::Dataset;

use super::columns::V17Locator;
use super::layout;
use super::KEY;
use crate::columns::read_frame;
use build_log_model::ReadContext;

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

fn not_present(what: &str) -> Error {
    Error::new(ErrorKind::NotPresent {
        what: what.to_string(),
    })
}

impl V17Locator {
    /// The dataset that holds the frame a request names. The metadata says whether it is recorded; the file says where.
    fn locate_image(&self, request: &ImageRequest) -> Result<Dataset, Error> {
        let build = self.build()?;
        let channel = build
            .sensors
            .iter()
            .find(|channel| channel.key == request.channel && channel.kind == ChannelKind::Image)
            .ok_or_else(|| not_present("the image channel"))?;
        match channel.presence.get(request.layer) {
            Some(Presence::Recorded { .. }) | Some(Presence::RecordedEmpty) => {}
            _ => return Err(not_present("the image in this layer")),
        }
        // the key of a channel names a phase, and only the phases of V17 name a group
        let phase = [layout::PRE_EXPOSURE, layout::POST_EXPOSURE]
            .into_iter()
            .find(|phase| ChannelKey::new([*phase]) == request.channel)
            .ok_or_else(|| not_present("a phase of V17"))?;
        let missing = || {
            corrupt(format!(
                "the metadata says the image is recorded in layer {}, and the file has no frame for it",
                request.layer.0
            ))
        };
        self.mount
            .group(layout::MONITORING_DATA)
            .and_then(|layers| {
                layers.group(&format!("{}{}", layout::LAYER_PREFIX, request.layer.0))
            })
            .and_then(|layer| layer.group(phase))
            .and_then(|images| images.dataset(layout::RAW))
            .map_err(|_| missing())
    }
}

/// Reads the region a request names of the frame of its phase in its layer.
pub(super) fn read_image(
    locator: &V17Locator,
    request: &ImageRequest,
    context: &ReadContext,
) -> Result<ImageData, Error> {
    context.check_cancelled()?;
    let frame = locator.locate_image(request)?;
    read_frame(&frame, KEY, request.region.as_ref(), context)
}
