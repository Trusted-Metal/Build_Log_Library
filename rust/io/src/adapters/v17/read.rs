//! Reading a V17 build into the Model (IP-13). Phase 13B reads the **build level**: the identity, the declared layer count and the build's layer thickness.
//! Phase 13C adds the **layers** ([`super::layers`]) and phase 13D the **lasers** ([`super::lasers`], read with each layer). Everything else on the mount, and
//! the groups beside it, is kept in `extra`, never dropped (decisions D13-3 and D13-19).
//!
//! Every path is relative to the mount (decision D04-9), and every name comes from [`super::layout`].

use build_log_model::{BuildIdentity, BuildMeta, Error, JoinStatus, Outcome, Quantity};
use hdf5::Group;

use super::events::Events;
use super::layers::read_layers_with;
use super::layout;
use super::parameters;
use crate::hdf5::{
    from_binding, keep_group_remainder, keep_node, keep_other_attributes, keep_other_nodes,
};
use crate::toolkit::context;
use crate::toolkit::values::{optional_float, optional_integer, optional_text, required_text};

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
    // how the build ended and what was done to it after capture, as the file says it (decision J14): the status is the build's outcome, and the join's status
    // and version are provenance. A file with none of them has none.
    build.quality.outcome = outcome_of(optional_text(mount, layout::BUILD_STATUS)?);
    build.provenance.opcua_join_status =
        optional_text(mount, layout::OPCUA_JOIN_STATUS)?.map(join_status_of);
    build.provenance.opcua_join_version = optional_text(mount, layout::OPCUA_JOIN_VERSION)?;
    // the machine, the material, the starting height and the build plate (IP-02 phase 2V): what is typed leaves `extra`, and what could not be read stays in it
    let mut typed: Vec<&str> = layout::TYPED_ON_THE_MOUNT.to_vec();
    typed.extend(context::read(mount, &mut build, &layout::CONTEXT_TABLE));
    keep_other_attributes(mount, &typed, &mut build.extra)?;
    let mut events = Events::new();
    read_layers_with(mount, &mut build, &mut events)?;
    let trains = parameters::read_all(mount, &mut build)?;
    keep_the_rest(mount, &mut build, &trains, &mut events)?;
    events.finish(&mut build);
    Ok(build)
}

/// The outcome that a build status stands for. Only the two words that tauri writes are named, and they are matched exactly: any other text is kept as the
/// file spells it, and no status is not a status (decision D13-24).
fn outcome_of(status: Option<String>) -> Outcome {
    match status.as_deref() {
        None => Outcome::NotRecorded,
        Some(layout::STATUS_COMPLETE) => Outcome::Complete,
        Some(layout::STATUS_IN_PROGRESS) => Outcome::InProgress,
        Some(other) => Outcome::Other(other.to_string()),
    }
}

/// The join status that a text stands for, matched exactly in the same way.
fn join_status_of(status: String) -> JoinStatus {
    match status.as_str() {
        layout::JOIN_STATUS_COMPLETE => JoinStatus::Complete,
        _ => JoinStatus::Other(status),
    }
}

/// What the mount holds besides its layers' group, and the groups of V17 that sit **beside** the mount (decision D13-19), kept in the build's `extra`: nothing a
/// file holds is dropped. A path that starts with `..` is beside the mount.
fn keep_the_rest(
    mount: &Group,
    build: &mut BuildMeta,
    trains: &[String],
    events: &mut Events,
) -> Result<(), Error> {
    keep_other_nodes(
        mount,
        &[layout::MONITORING_DATA, layout::CALIBRATION_DATA],
        &[],
        &mut build.extra,
    )?;
    // the calibration group: the optical trains that became a laser's parameters are typed, and everything else in it (the formula images, a train no laser uses, a
    // train that could not be read) is kept
    if mount.link_exists(layout::CALIBRATION_DATA) {
        let calibration = mount
            .group(layout::CALIBRATION_DATA)
            .map_err(|e| from_binding("opening the calibration group", e))?;
        let typed: Vec<&str> = trains.iter().map(String::as_str).collect();
        keep_group_remainder(
            &calibration,
            &typed,
            &[],
            &[layout::CALIBRATION_DATA.to_string()],
            &mut build.extra,
        )?;
    }
    let beside = parent_of(mount)?;
    let up = [String::from("..")];
    for name in layout::BESIDE_THE_MOUNT {
        if !beside.link_exists(name) {
            continue;
        }
        if name == layout::GLOBAL_SENSORS {
            // the bucket of events that no layer owns is read; what else the group holds is kept
            events.read_top(&beside, &mut build.extra, &mut build.origins)?;
        } else {
            keep_node(&beside, name, &up, &mut build.extra)?;
        }
    }
    Ok(())
}

/// The group that holds the mount, found through the mount's own path: the root of the file for a standalone file, and the group it is nested in otherwise.
pub(super) fn parent_of(mount: &Group) -> Result<Group, Error> {
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
