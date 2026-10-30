//! The camera images of a V17 build (IP-13 phase 13F; decision D13-26): which layers have a frame, and what a frame is.
//!
//! * **A layer's group may hold two image groups**, `Pre_Exposure` and `Post_Exposure`, each with one dataset that is the frame. Each phase becomes a **channel of the
//!   whole build** (the key is the phase's name), of the kind image, clocked as an image, recorded in the layers that have a frame, with one frame in each layer.
//! * **A frame is described and never read.** Its format is its rows, its columns and the type of a pixel, from the dataset's descriptor; no pixel is read (reading
//!   frames is phase 13L). Only a frame of two dimensions is described, because that is what the files hold: any other shape is a fault in that image.
//! * **The format is the same in every layer.** A frame that is stored differently from the one of an earlier layer is not described for its layer, and the layer's
//!   origin says why (a channel has one format).
//! * **Whatever else an image holds is kept** in the layer's `extra`, by its path below the layer: the attributes of the frame's dataset (the HDF5 image conventions),
//!   an attribute of the image group, and any child of it that is not the frame. A group of a phase that has no frame is kept whole.
//! * **An image that cannot be read is reported on its layer** (decision D13-21): its origin says why and its group is kept whole. Only damage to the file itself stops
//!   the open.
//!
//! V18 keeps its images under a sensor and is a draft that conforms to this when it settles; nothing here names it.
//!
//! Every name comes from [`super::layout`].

use std::collections::BTreeMap;

use build_log_model::{
    BuildMeta, ByLayer, ChannelDescriptor, ChannelKey, Error, ErrorKind, ImageFormat, LayerMeta,
    Origin, Presence,
};
use hdf5::{Dataset, Group};

use super::lasers::{is_about_the_content, numeric_type};
use super::layout;
use crate::hdf5::{describe_dataset, keep_dataset_remainder, keep_group_remainder, keep_node};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The two phases of a layer's images, in the order of the channels.
const PHASES: [&str; 2] = [layout::PRE_EXPOSURE, layout::POST_EXPOSURE];

/// What is known of one phase's channel so far.
struct Seen {
    format: ImageFormat,
    presence: ByLayer<Presence>,
}

/// The image channels of a build, collected layer by layer.
pub struct Images {
    seen: BTreeMap<usize, Seen>,
}

impl Images {
    pub fn new() -> Self {
        Images {
            seen: BTreeMap::new(),
        }
    }

    /// Reads the image groups of one layer's group. Returns the names of the children of the layer that were dealt with (typed, or kept whole because they could not
    /// be read), so that the caller does not keep them again.
    pub fn read_layer(
        &mut self,
        group: &Group,
        layer: &mut LayerMeta,
    ) -> Result<Vec<String>, Error> {
        let mut dealt_with = Vec::new();
        for (at, phase) in PHASES.iter().enumerate() {
            if !group.link_exists(phase) {
                continue;
            }
            match self.read_phase(group, at, phase, layer) {
                Ok(true) => dealt_with.push(phase.to_string()),
                // a group with no frame is nothing to describe: the caller keeps it whole
                Ok(false) => {}
                Err(error) if is_about_the_content(&error) => {
                    layer
                        .origins
                        .record(format!("image {phase}"), Origin::not_read(&error));
                    keep_node(group, phase, &[], &mut layer.extra)?;
                    dealt_with.push(phase.to_string());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(dealt_with)
    }

    /// One phase of one layer. `Ok(false)` if the group holds no frame.
    fn read_phase(
        &mut self,
        group: &Group,
        at: usize,
        phase: &str,
        layer: &mut LayerMeta,
    ) -> Result<bool, Error> {
        let holder = group
            .group(phase)
            .map_err(|e| corrupt(format!("`{phase}` is not a group: {e}")))?;
        if !holder.link_exists(layout::RAW) {
            return Ok(false);
        }
        let frame = holder
            .dataset(layout::RAW)
            .map_err(|e| corrupt(format!("the frame of `{phase}` is not a dataset: {e}")))?;
        let format = format_of(&frame)?;
        match self.seen.get_mut(&at) {
            Some(seen) if seen.format != format => {
                return Err(corrupt(format!(
                    "the image of `{phase}` is stored as {format:?} here and as {:?} in other layers",
                    seen.format
                )));
            }
            Some(seen) => {
                seen.presence.insert(layer.id, Presence::from_count(1));
            }
            None => {
                let mut presence = ByLayer::new();
                presence.insert(layer.id, Presence::from_count(1));
                self.seen.insert(at, Seen { format, presence });
            }
        }
        // what else the image holds: the attributes of the frame, and what the group holds besides the frame
        keep_dataset_remainder(
            &frame,
            &[],
            &[phase.to_string(), layout::RAW.to_string()],
            &mut layer.extra,
        )?;
        keep_group_remainder(
            &holder,
            &[layout::RAW],
            &[],
            &[phase.to_string()],
            &mut layer.extra,
        )?;
        Ok(true)
    }

    /// Adds the channels to the build's channels, in the order of the phases.
    pub fn finish(self, build: &mut BuildMeta) {
        for (at, seen) in self.seen {
            let key = ChannelKey::new([PHASES[at]]);
            build
                .sensors
                .push(ChannelDescriptor::image(key, seen.format).with_presence(seen.presence));
        }
    }
}

impl Default for Images {
    fn default() -> Self {
        Images::new()
    }
}

/// The format of a frame, from its dataset's descriptor: two dimensions, rows and then columns, and a number for each pixel.
fn format_of(frame: &Dataset) -> Result<ImageFormat, Error> {
    let descriptor = describe_dataset(frame)?;
    match descriptor.shape.as_slice() {
        [rows, columns] => Ok(ImageFormat::new(
            *rows as u64,
            *columns as u64,
            numeric_type(&descriptor.class)?,
        )),
        shape => Err(corrupt(format!(
            "a frame has two dimensions, and this one has {}",
            shape.len()
        ))),
    }
}
