//! Where the samples of V17 are stored (IP-13 phase 13L, task T13.14; IP-06 decisions D06-1 (c) and D06-9): the **locator** that the column engine asks.
//!
//! The engine (`crate::columns`) knows the rules for every version; this file only says **which dataset holds a column** of V17.
//!
//! * **A column is a laser's channel in a layer**: the request names the layer, the laser (by its key, the name of its group) and the channel (by the name the adapter gave
//!   it when it described the laser, which is its dataset's own name). A request with no laser names a column of the build, and V17 has none that `read` serves: the camera
//!   images and the OPC UA events have accessors of their own (phases 6F and 6G).
//! * **The metadata answers first** (D06-9): a channel that the metadata does not have for this laser, or has no presence for in this layer, is `NotPresent`, and the file is not
//!   touched. A channel recorded in the layer is **read from the file as it is stored**: with samples, or recorded empty (a dataset of no value, which the engine reads as an
//!   empty column of the stored type).
//! * **A column that the metadata says is recorded but the file does not hold is `Corrupt`**, located by the engine, and the other channels of the laser still read (BL-FA-09).
//! * **Nothing is repaired and nothing is cached here.** The cache is phase 6C, and the engine opens the dataset for each read (D06-10).
//!
//! Every name comes from [`super::layout`] and [`super::lasers`], which is where the channels are listed.

use std::sync::Arc;

use build_log_model::{BuildMeta, ChannelKey, ColumnRequest, Error, ErrorKind, Presence};
use hdf5::Group;

use super::lasers::{channel_dataset, specs};
use super::layout;
use super::KEY;
use crate::columns::{Located, Locator};

/// Finds the datasets of the columns of a V17 build: its metadata says what is recorded, and its mount says where.
pub struct V17Locator {
    pub(super) build: Arc<BuildMeta>,
    pub(super) mount: Group,
}

impl V17Locator {
    pub fn new(build: Arc<BuildMeta>, mount: Group) -> Self {
        V17Locator { build, mount }
    }
}

fn not_present(what: impl Into<String>) -> Error {
    Error::new(ErrorKind::NotPresent { what: what.into() })
}

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

impl Locator for V17Locator {
    fn version(&self) -> &str {
        KEY
    }

    fn locate(&self, request: &ColumnRequest) -> Result<Located, Error> {
        let Some(laser_key) = &request.laser else {
            return Err(not_present(
                "a column of the build and not of a laser (V17 has none that `read` serves)",
            ));
        };
        let laser = self
            .build
            .laser(laser_key)
            .ok_or_else(|| not_present("the laser"))?;
        let channel = laser
            .channels
            .iter()
            .find(|channel| channel.key == request.channel)
            .ok_or_else(|| not_present("the channel of this laser"))?;
        match channel.presence.get(request.layer) {
            Some(Presence::Recorded { .. }) | Some(Presence::RecordedEmpty) => {}
            _ => return Err(not_present("the channel in this layer")),
        }
        let spec = specs()
            .into_iter()
            .find(|spec| ChannelKey::new([spec.dataset]) == request.channel)
            .ok_or_else(|| not_present("a channel of V17"))?;

        let layer_name = format!("{}{}", layout::LAYER_PREFIX, request.layer.0);
        let group = self
            .mount
            .group(layout::MONITORING_DATA)
            .and_then(|layers| layers.group(&layer_name))
            .and_then(|layer| layer.group(&laser_key.key))
            .map_err(|_| {
                corrupt(format!(
                    "the metadata says the laser is recorded in layer {}, and the file has no group for it",
                    request.layer.0
                ))
            })?;
        match channel_dataset(&group, &spec)? {
            Some(dataset) => Ok(Located::new(dataset)),
            None => Err(corrupt(format!(
                "the metadata says the channel is recorded in layer {}, and the file has no dataset for it",
                request.layer.0
            ))),
        }
    }
}
