//! The layers of a V17 build (IP-13 phase 13C).
//!
//! * **A layer's id is the number in its group's name**, and the ids may have gaps: the special-layer miniature holds layers 1964 to 1967 and 2006 to 2008 of a
//!   build that declares 2,110. Declared and present are counted separately; neither is derived from the other.
//! * **Times are the integers as written**, in no unit: the files do not state one, and tauri's own releases differ (decision D02-19).
//! * **A layer has two thicknesses, the actual and the commanded, and each is reported on its own.** A value that V17 writes when it does not know (`0.0`) is
//!   not a thickness: the field stays empty and the origin of that thickness keeps the raw value (decision J4). An attribute the file does not have says
//!   nothing at all.
//! * **Every other attribute of a layer's group is kept in the layer's `extra`**, and so is every child of it that is not a laser's group (phase 13D); the
//!   lasers are read by [`super::lasers`]. A child of the layers' group that is not named for a layer is kept in the build's `extra`.

use build_log_model::{BuildMeta, Error, ErrorKind, LayerId, LayerMeta, Location, TimeValue};
use hdf5::Group;

use super::lasers::Lasers;
use super::layout;
use super::values::{optional_float, optional_integer};
use crate::hdf5::{from_binding, keep_group_remainder, keep_other_attributes};

/// What V17 writes for a thickness it does not know (decision J4). A thickness of `1.0` is a real one in V17.
const THICKNESS_PLACEHOLDERS: [f64; 1] = [0.0];

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The id of the layer that a group name stands for: the layer prefix and nothing but digits after it. `None` for any other name, and an error for digits that
/// do not fit a layer id.
fn layer_id_of(name: &str) -> Result<Option<LayerId>, Error> {
    let Some(digits) = name.strip_prefix(layout::LAYER_PREFIX) else {
        return Ok(None);
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }
    digits
        .parse::<u32>()
        .map(|n| Some(LayerId(n)))
        .map_err(|_| {
            Error::new(ErrorKind::OutOfWidth {
                value: digits.to_string(),
                stored_as: "a layer id (a 32-bit unsigned number)".to_string(),
            })
        })
}

/// Reads the layers that the mount holds into `build`. A mount without its layers' group is `Corrupt`.
pub fn read_layers(mount: &Group, build: &mut BuildMeta) -> Result<(), Error> {
    if !mount.link_exists(layout::MONITORING_DATA) {
        return Err(corrupt(format!(
            "the build has no `{}` group, which holds its layers",
            layout::MONITORING_DATA
        )));
    }
    let monitoring = mount
        .group(layout::MONITORING_DATA)
        .map_err(|e| from_binding("opening the group of the layers", e))?;
    let names = monitoring
        .member_names()
        .map_err(|e| from_binding("listing the layers", e))?;
    let mut lasers = Lasers::new(mount);
    let mut layer_names: Vec<&str> = Vec::new();
    for name in &names {
        let Some(id) = layer_id_of(name)? else {
            continue;
        };
        layer_names.push(name);
        let located = |error: Error| {
            error.at(Location::new()
                .with_layer(id)
                .with_operation("reading a layer"))
        };
        let group = monitoring
            .group(name)
            .map_err(|e| located(from_binding("opening a layer", e)))?;
        let mut layer = read_layer(&group, id).map_err(located)?;
        lasers.read_layer(&group, &mut layer).map_err(located)?;
        if build.layers.insert(id, layer).is_some() {
            return Err(located(corrupt(format!(
                "two groups are named for the layer {id}"
            ))));
        }
    }
    lasers.finish(build);
    // what the layers' group holds besides the layers: its own attributes and the children that are not named for a layer
    keep_group_remainder(
        &monitoring,
        &layer_names,
        &[],
        &[layout::MONITORING_DATA.to_string()],
        &mut build.extra,
    )
}

/// One layer: its times, its two thicknesses and, in `extra`, what else its group holds.
fn read_layer(group: &Group, id: LayerId) -> Result<LayerMeta, Error> {
    let mut layer = LayerMeta::new(id);
    layer.start = optional_integer(group, layout::START_LAYER_TIME)?
        .map(|value| TimeValue::from_file(value, None));
    layer.end = optional_integer(group, layout::END_LAYER_TIME)?
        .map(|value| TimeValue::from_file(value, None));
    if let Some(value) = optional_float(group, layout::ACTUAL_LAYER_THICKNESS)? {
        layer.set_actual_thickness_from_file(value, None, &THICKNESS_PLACEHOLDERS);
    }
    if let Some(value) = optional_float(group, layout::COMMANDED_LAYER_THICKNESS)? {
        layer.set_commanded_thickness_from_file(value, None, &THICKNESS_PLACEHOLDERS);
    }
    keep_other_attributes(group, &layout::TYPED_ON_A_LAYER, &mut layer.extra)?;
    Ok(layer)
}
