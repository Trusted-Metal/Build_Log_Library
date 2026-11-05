//! Where the samples of a version are stored (IP-13 phase 13L, task T13.14; IP-06 decisions D06-1 (c) and D06-9; moved to the toolkit by IP-12 phase 12A, decision D12-1): the part of the
//! **locator** that is the same for every version.
//!
//! The engine (`crate::columns`) knows the rules for every version; the adapter's locator says **which dataset holds a column**, and for a version whose lasers are laid out as
//! [`LaserLayout`] says, [`locate`] does it.
//!
//! * **A column is a laser's channel in a layer**: the request names the layer, the laser (by its key, the name of its group) and the channel (by the name the adapter gave
//!   it when it described the laser, which is its dataset's own name). A request with no laser names a column of the build, and a version may have none that `read` serves: the camera
//!   images and the OPC UA events have accessors of their own (phases 6F and 6G).
//! * **The metadata answers first** (D06-9): a channel that the metadata does not have for this laser, or has no presence for in this layer, is `NotPresent`, and the file is not
//!   touched. A channel recorded in the layer is **read from the file as it is stored**: with samples, or recorded empty (a dataset of no value, which the engine reads as an
//!   empty column of the stored type).
//! * **A column that the metadata says is recorded but the file does not hold is `Corrupt`**, located by the engine, and the other channels of the laser still read (BL-FA-09).
//! * **Nothing is repaired and nothing is cached here.** The cache is phase 6C, and the engine opens the dataset for each read (D06-10).

use build_log_model::{BuildMeta, ChannelKey, ColumnRequest, Error, ErrorKind, Presence};
use hdf5::Group;

use super::lasers::{channel_dataset, LaserLayout};
use crate::columns::Located;

fn not_present(what: impl Into<String>) -> Error {
    Error::new(ErrorKind::NotPresent { what: what.into() })
}

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The dataset that holds the column `request` names, in the build whose metadata is `build` and whose data is under `mount`, for a version laid out as `layout` says.
pub fn locate(
    build: &BuildMeta,
    mount: &Group,
    layout: &LaserLayout,
    request: &ColumnRequest,
) -> Result<Located, Error> {
    let Some(laser_key) = &request.laser else {
        return Err(not_present(format!(
            "a column of the build and not of a laser ({} has none that `read` serves)",
            layout.label
        )));
    };
    let laser = build
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
    let spec = layout
        .channels
        .iter()
        .find(|spec| ChannelKey::new([spec.dataset]) == request.channel)
        .ok_or_else(|| not_present(format!("a channel of {}", layout.label)))?;

    let layer_name = format!("{}{}", layout.layer_prefix, request.layer.0);
    let group = mount
        .group(layout.layers_group)
        .and_then(|layers| layers.group(&layer_name))
        .and_then(|layer| layer.group(&laser_key.key))
        .map_err(|_| {
            corrupt(format!(
                "the metadata says the laser is recorded in layer {}, and the file has no group for it",
                request.layer.0
            ))
        })?;
    match channel_dataset(&group, spec)? {
        Some(dataset) => Ok(Located::new(dataset)),
        None => Err(corrupt(format!(
            "the metadata says the channel is recorded in layer {}, and the file has no dataset for it",
            request.layer.0
        ))),
    }
}
