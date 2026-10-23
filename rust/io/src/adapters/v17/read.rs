//! Reading a V17 build into the Model (IP-13). Phase 13B reads the **build level**: the identity, the declared layer count and the build's layer thickness.
//! Phase 13C adds the **layers** ([`super::layers`]) and phase 13D the **lasers** ([`super::lasers`], read with each layer). Everything else on the mount, and
//! the groups beside it, is kept in `extra`, never dropped (decisions D13-3 and D13-19).
//!
//! Every path is relative to the mount (decision D04-9), and every name comes from [`super::layout`].

use build_log_model::{BuildIdentity, BuildMeta, Error, Quantity};
use hdf5::Group;

use super::layers::read_layers;
use super::layout;
use super::values::{optional_float, optional_integer, optional_text, required_text};
use crate::hdf5::{from_binding, keep_node, keep_other_attributes, keep_other_nodes};

/// The build level of the build whose data is under `mount`.
pub fn read_build(mount: &Group) -> Result<BuildMeta, Error> {
    let identity = BuildIdentity {
        build_id: required_text(mount, layout::BUILD_ID)?,
        date: optional_text(mount, layout::DATE)?,
        username: optional_text(mount, layout::USERNAME)?,
        alloy: optional_text(mount, layout::ALLOYS)?,
        part_count: optional_integer(mount, layout::NUMBER_OF_PARTS)?,
        // V17 does not carry these: absent is absent
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    };
    let mut build = BuildMeta::new(identity);
    build.layers_declared = optional_integer(mount, layout::LAYER_QUANTITY)?;
    // the unit of the thickness is not written by V17 (decision D02-18, J5): the value is kept with an unknown unit
    build.layer_thickness = optional_float(mount, layout::LAYER_THICKNESS)?
        .map(|value| Quantity::from_file(value, None));
    keep_other_attributes(mount, &layout::TYPED_ON_THE_MOUNT, &mut build.extra)?;
    read_layers(mount, &mut build)?;
    keep_the_rest(mount, &mut build)?;
    Ok(build)
}

/// What the mount holds besides its layers' group, and the groups of V17 that sit **beside** the mount (decision D13-19), kept in the build's `extra`: nothing a
/// file holds is dropped. A path that starts with `..` is beside the mount.
fn keep_the_rest(mount: &Group, build: &mut BuildMeta) -> Result<(), Error> {
    keep_other_nodes(mount, &[layout::MONITORING_DATA], &[], &mut build.extra)?;
    let beside = parent_of(mount)?;
    let up = [String::from("..")];
    for name in layout::BESIDE_THE_MOUNT {
        if beside.link_exists(name) {
            keep_node(&beside, name, &up, &mut build.extra)?;
        }
    }
    Ok(())
}

/// The group that holds the mount, found through the mount's own path: the root of the file for a standalone file, and the group it is nested in otherwise.
fn parent_of(mount: &Group) -> Result<Group, Error> {
    let name = mount.name();
    let parent = match name.rfind('/') {
        Some(at) if at > 0 => &name[..at],
        _ => "/",
    };
    let file = mount
        .file()
        .map_err(|e| from_binding("finding the file of the mount", e))?;
    file.group(parent)
        .map_err(|e| from_binding("opening the group that holds the mount", e))
}
