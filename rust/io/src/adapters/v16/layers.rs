//! The layers of a V16 build (IP-12 phase 12C).
//!
//! * **A layer's id is the number in its group's name**, and the ids may have gaps. Declared and present are counted separately; neither is derived from the other.
//! * **Times are the integers as written**, in no unit: V16 states none (decisions D02-19 and D12-3), so the layer's times have an unknown unit and no instant is derived.
//! * **A layer has two thicknesses, the actual and the commanded, and each is reported on its own.** `1.0` is what V16 writes when it does not know a thickness
//!   ([`layout::LAYER_THICKNESS_PLACEHOLDERS`], decision J4): the field stays empty and the origin of that thickness keeps the raw value. An attribute the file does not have says nothing.
//! * **Every other attribute of a layer's group is kept in the layer's `extra`, and so is every child of it that is not a laser's group.** The lasers are read by [`super::lasers`], with each
//!   layer (phase 12D); V16 has no images and no events. A child of the layers' group that is not named for a layer is kept in the build's `extra`, and so are the group's own attributes
//!   **unless one is typed: the build's sample period is, once a laser has been given it** (decision D12-5).
//!
//! This reads what V16 holds and shares nothing with the layers of V17 yet: the toolkit's seam is the laser reader (decision D12-1), and a second copy of the layers' loop is the cost of
//! adding a version without touching another's folder (BL-VA-05); the third version shows whether it belongs in the toolkit.

use build_log_model::{BuildMeta, Error, ErrorKind, LayerId, LayerMeta, Location, TimeValue};
use hdf5::Group;

use super::lasers;
use super::layout;
use crate::hdf5::{from_binding, keep_group_remainder, keep_other_attributes, keep_other_nodes};
use crate::toolkit::values::{optional_float, optional_integer};

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
    let mut lasers = lasers::collector(mount);
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
        // the children of the layer that are typed: the lasers. Whatever else the layer holds is kept.
        let typed = lasers.read_layer(&group, &mut layer).map_err(located)?;
        let typed: Vec<&str> = typed.iter().map(String::as_str).collect();
        keep_other_nodes(&group, &typed, &[], &mut layer.extra).map_err(located)?;
        if build.layers.insert(id, layer).is_some() {
            return Err(located(corrupt(format!(
                "two groups are named for the layer {id}"
            ))));
        }
    }
    lasers.finish(build);
    // what the layers' group holds besides the layers: its own attributes and the children that are not named for a layer. **The sample period is typed only where a laser has it**: a build with
    // no laser, or a period that could not be read (the laser says so in its origins), keeps the attribute, so that nothing the file holds is dropped
    let typed_attributes: &[&str] = if the_period_is_given(build) {
        &[layout::SAMPLE_PERIOD]
    } else {
        &[]
    };
    keep_group_remainder(
        &monitoring,
        &layer_names,
        typed_attributes,
        &[layout::MONITORING_DATA.to_string()],
        &mut build.extra,
    )
}

/// Whether a laser has been given the build's sample period in some layer.
fn the_period_is_given(build: &BuildMeta) -> bool {
    build.lasers().iter().any(|laser| {
        laser
            .layers
            .iter()
            .any(|(_, entry)| entry.time_axis.period.is_some())
    })
}

/// One layer: its times, its two thicknesses and, in `extra`, what else its group holds.
fn read_layer(group: &Group, id: LayerId) -> Result<LayerMeta, Error> {
    let mut layer = LayerMeta::new(id);
    layer.start = optional_integer(group, layout::START_LAYER_TIME)?
        .map(|value| TimeValue::from_file(value, None));
    layer.end = optional_integer(group, layout::END_LAYER_TIME)?
        .map(|value| TimeValue::from_file(value, None));
    if let Some(value) = optional_float(group, layout::ACTUAL_LAYER_THICKNESS)? {
        layer.set_actual_thickness_from_file(value, None, &layout::LAYER_THICKNESS_PLACEHOLDERS);
    }
    if let Some(value) = optional_float(group, layout::COMMANDED_LAYER_THICKNESS)? {
        layer.set_commanded_thickness_from_file(value, None, &layout::LAYER_THICKNESS_PLACEHOLDERS);
    }
    keep_other_attributes(group, &layout::TYPED_ON_A_LAYER, &mut layer.extra)?;
    Ok(layer)
}
