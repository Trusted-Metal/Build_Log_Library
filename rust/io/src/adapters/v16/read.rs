//! Reading a V16 build into the Model (IP-12). Phase 12C reads the **build level**: the identity, the declared layer count and the build's context, with the **layers**
//! ([`super::layers`]); phase 12D the **lasers** with each layer; phase 12E the **parameters** of each laser ([`super::parameters`]). Everything else on the mount, and the groups below it,
//! is kept in `extra`, never dropped.
//!
//! Every path is relative to the mount (decision D04-9), and every name comes from [`super::layout`].

use build_log_model::{BuildIdentity, BuildMeta, Error};
use hdf5::Group;

use super::layers::read_layers;
use super::layout;
use super::parameters;
use crate::hdf5::{keep_other_attributes, keep_other_nodes};
use crate::toolkit::context;
use crate::toolkit::values::{optional_integer, optional_text};

/// The build level of the build whose data is under `mount`.
pub fn read_build(mount: &Group) -> Result<BuildMeta, Error> {
    let identity = BuildIdentity {
        // a build with no id is read as it is: a missing attribute is not an error (decision D26-17), and an empty text is a value
        build_id: optional_text(mount, layout::BUILD_ID)?,
        // the date is the string the file gives, with the offset label it carries: nothing is corrected (CT-UC-12)
        date: optional_text(mount, layout::DATE)?,
        username: optional_text(mount, layout::USERNAME)?,
        alloy: optional_text(mount, layout::ALLOYS)?,
        part_count: optional_integer(mount, layout::NUMBER_OF_PARTS)?,
        // V16 does not carry these: absent is absent
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    };
    let mut build = BuildMeta::new(identity);
    build.layers_declared = optional_integer(mount, layout::LAYER_QUANTITY)?;
    // V16 has no layer thickness of the build, no build status and no OPC UA join: the build's thickness, its outcome and its provenance are as the Model makes them, which is not recorded
    // the machine, the material, the starting height and the build plate: what is typed leaves `extra`, and what could not be read stays in it
    let mut typed: Vec<&str> = layout::TYPED_ON_THE_MOUNT.to_vec();
    typed.extend(context::read(mount, &mut build, &layout::CONTEXT_TABLE));
    // the layers, and the lasers in them: the parameters are those of the lasers, so they come after
    read_layers(mount, &mut build)?;
    // the calibration of each laser, and the two bit resolutions of the mount that it gives to the galvos: what it took is typed, and the rest of the mount is kept
    let taken = parameters::read_all(mount, &mut build)?;
    typed.extend(&taken.mount_attributes);
    keep_other_attributes(mount, &typed, &mut build.extra)?;
    // the children of the mount besides the layers' group and the calibration: kept as they are; and what the calibration group holds that no laser took
    keep_other_nodes(
        mount,
        &[layout::MONITORING_DATA, layout::CALIBRATION_DATA],
        &[],
        &mut build.extra,
    )?;
    parameters::keep_the_rest(mount, &taken, &mut build)?;
    Ok(build)
}
