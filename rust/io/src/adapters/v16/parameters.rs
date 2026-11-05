//! The parameters of a V16 laser (IP-12 phase 12E; decisions D12-6 and D12-7, Q12-2 and Q12-3, and IP-24 finding F2): the calibration of the laser, read into the Model's [`LaserParameters`].
//!
//! * **The calibration is the laser's own.** `Laser_Calibration_N` is laser N's, by its number: V16 has no optical train, so the snapshot has **no optical train number** and nothing is paired by an
//!   inference of the adapter. A laser the file has no calibration for has no snapshot, and says nothing of it.
//! * **Thirteen attributes have a name in the CDM** ([`ROWS`]); the twelve others are **not named** and stay in the snapshot's `extra` as the file has them (naming one is for the owner to decide).
//!   A number is a number whether the file stored it as an `int16`, an `int32` or a `float32`; V16 states no unit, so a quantity's unit is unknown.
//! * **The three offsets carry a note, and are never flipped** (decision D12-6): their sign is opposite to other versions' (an inference from one machine's numbers), so each origin says so and the
//!   value is as the file wrote it.
//! * **The mount's two bit resolutions are given to each laser's three galvos** (IP-24 finding F2): V16 writes one value, as text of digits, for every laser and axis. Each value is a whole number and
//!   each origin says that the value was global in the file. A bit resolution that is not a whole number is reported on the galvos' values and its attribute stays on the mount.
//! * **The correction grid is described, and read through the engine** (IP-24 phase 24B, decision D24-11): the snapshot lists it (its name, its stored type and its extents, and that its domain and its axis
//!   order are unrecorded), and its values come through `ColumnSource::read_grid`, as stored. It is neither interpreted nor evaluated, and its name does not say whether it is the grid or its inverse.
//! * **What is not typed is kept**, in the snapshot's `extra` by its path below the calibration (a child that is not the grid) and, in the build's `extra`, the two formula images beside the
//!   calibrations. Nothing is dropped.
//! * **A calibration that cannot be read is reported on its laser** (decision D13-21): the laser has no snapshot, an origin says why, and the whole calibration is kept in the build's `extra`. Only damage
//!   to the file itself stops the open.
//!
//! Every name comes from [`super::layout`]; the names that the CDM gives are the Model's.

use build_log_model::{
    BuildMeta, CorrectionGrid, Error, ErrorKind, LaserParameters, Origin, ParameterValue, Quantity,
    BIT_RESOLUTION_GLOBAL, SCAN_HEAD_OFFSET_SIGN_OPPOSITE,
};
use hdf5::{Group, Location};

use super::layout;
use super::KEY;
use crate::columns::describe_grid;
use crate::hdf5::{keep_group_remainder, keep_node, keep_other_attributes, keep_other_nodes};
use crate::toolkit::content::is_about_the_content;
use crate::toolkit::values::{optional_integer, optional_number, optional_text};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// What kind of value an attribute holds, which decides how it is read.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    /// A number, whatever width the file stored it in, with an unknown unit: V16 states none.
    Quantity,
    /// A number that has no unit.
    Plain,
}

/// One attribute of a laser's calibration, the name the CDM gives its value, how it is read, and the note that its origin carries, if it has one.
struct Row {
    attribute: &'static str,
    name: &'static str,
    kind: Kind,
    note: Option<&'static str>,
}

const fn row(attribute: &'static str, name: &'static str, kind: Kind) -> Row {
    Row {
        attribute,
        name,
        kind,
        note: None,
    }
}

/// An offset: its origin carries the note that its sign is opposite to other versions' (decision D12-6).
const fn offset(attribute: &'static str, name: &'static str) -> Row {
    Row {
        attribute,
        name,
        kind: Kind::Quantity,
        note: Some(SCAN_HEAD_OFFSET_SIGN_OPPOSITE),
    }
}

/// The thirteen attributes of a laser's calibration that have a name in the CDM (Appendix B of the plan).
const ROWS: [Row; 13] = [
    row(layout::LASER_MODEL, "light_source.model", Kind::Text),
    row(layout::SCAN_HEAD_MODEL, "scanner.model", Kind::Text),
    row(
        layout::SCANNER_FIELD_SIZE_X,
        "scanner.field_size_x",
        Kind::Quantity,
    ),
    row(
        layout::SCANNER_FIELD_SIZE_Y,
        "scanner.field_size_y",
        Kind::Quantity,
    ),
    row(
        layout::SCANNER_FIELD_SIZE_Z,
        "scanner.field_size_z",
        Kind::Quantity,
    ),
    offset(layout::SCANNER_OFFSET_X, "scanner.offset_x"),
    offset(layout::SCANNER_OFFSET_Y, "scanner.offset_y"),
    offset(layout::SCANNER_OFFSET_Z, "scanner.offset_z"),
    row(layout::SCALING_X, "scanner.scaling_x", Kind::Plain),
    row(layout::SCALING_Y, "scanner.scaling_y", Kind::Plain),
    row(
        layout::MAJOR_AXIS_ANGLE,
        "optics.major_axis_angle",
        Kind::Quantity,
    ),
    row(
        layout::RAYLEIGH_LENGTH_MAJOR_AXIS,
        "optics.rayleigh_length_major",
        Kind::Quantity,
    ),
    row(
        layout::RAYLEIGH_LENGTH_MINOR_AXIS,
        "optics.rayleigh_length_minor",
        Kind::Quantity,
    ),
];

/// The three galvos that each laser's snapshot gives a bit resolution to, by the prefix of the CDM's names.
const GALVOS: [&str; 3] = ["galvo_x.", "galvo_y.", "galvo_z."];

/// The mount's bit resolutions: the attribute, and the CDM's name of the value below a galvo's prefix.
const BIT_RESOLUTIONS: [(&str, &str); 2] = [
    (layout::ACTUAL_BIT_RESOLUTION, "actual_bit_resolution"),
    (layout::COMMANDED_BIT_RESOLUTION, "commanded_bit_resolution"),
];

/// What the mount says of one bit resolution: the number, nothing if the mount has none, or the reason it could not be read.
type BitResolution = (&'static str, &'static str, Result<Option<i64>, Error>);

/// What the calibration took from the file, so that the build level and the rest of the calibration group do not keep it as well.
#[derive(Default)]
pub struct Taken {
    /// The attributes of the mount that were given to a snapshot.
    pub mount_attributes: Vec<&'static str>,
    /// The calibrations of the lasers that were read into a snapshot.
    pub calibrations: Vec<String>,
}

/// The group of this name below `parent`, or `None` if there is none; a name that is something else is a fault.
fn child(parent: &Group, name: &str) -> Result<Option<Group>, Error> {
    if !parent.link_exists(name) {
        return Ok(None);
    }
    parent
        .group(name)
        .map(Some)
        .map_err(|e| corrupt(format!("`{name}` is not a group: {e}")))
}

/// The descriptor of the correction grid of a laser's calibration, or `None` if the calibration has none. **The name does not say which grid it is**, so it has no role. **What a grid is, is the engine's
/// to say** (`describe_grid`): a dataset that it cannot read is an error here, and the caller keeps it untyped.
fn describe_the_grid(group: &Group) -> Result<Option<CorrectionGrid>, Error> {
    if !group.link_exists(layout::CORRECTION_DATA) {
        return Ok(None);
    }
    let dataset = group.dataset(layout::CORRECTION_DATA).map_err(|e| {
        corrupt(format!(
            "`{}` is not a dataset: {e}",
            layout::CORRECTION_DATA
        ))
    })?;
    let (stored_as, extents) = describe_grid(&dataset, KEY)?;
    Ok(Some(CorrectionGrid::new(
        layout::CORRECTION_DATA,
        None,
        stored_as,
        extents,
    )))
}

/// The snapshot of one laser's calibration: the thirteen values by their CDM names, the note on the offsets, and what else the group holds.
fn read_calibration(group: &Group) -> Result<LaserParameters, Error> {
    let mut parameters = LaserParameters::new();
    let mut typed: Vec<&'static str> = Vec::new();
    for row in &ROWS {
        let value = match row.kind {
            Kind::Text => optional_text(group, row.attribute)?.map(ParameterValue::Text),
            Kind::Quantity => optional_number(group, row.attribute)?
                .map(|number| ParameterValue::Quantity(Quantity::from_file(number, None))),
            Kind::Plain => optional_number(group, row.attribute)?.map(ParameterValue::Plain),
        };
        let Some(value) = value else {
            continue;
        };
        parameters.set(row.name, value)?;
        if let Some(note) = row.note {
            parameters
                .origins
                .record(row.name, Origin::Other(note.to_string()));
        }
        typed.push(row.attribute);
    }
    // the correction grid (IP-24 phase 24B): a grid that the engine reads is described here and is not kept as an untyped node; one that it does not read stays where it is, and the snapshot says why
    let mut children: Vec<&str> = Vec::new();
    match describe_the_grid(group) {
        Ok(Some(grid)) => {
            parameters.grids.push(grid);
            children.push(layout::CORRECTION_DATA);
        }
        Ok(None) => {}
        Err(error) if is_about_the_content(&error) => {
            parameters.origins.record(
                format!("grid {}", layout::CORRECTION_DATA),
                Origin::not_read(&error),
            );
        }
        Err(error) => return Err(error),
    }
    // what is not named: the attributes as the file has them, and the children that are not the grid, by their paths below the calibration
    let mut extra = std::mem::take(&mut parameters.extra);
    keep_other_attributes(group, &typed, &mut extra)?;
    keep_other_nodes(group, &children, &[], &mut extra)?;
    parameters.extra = extra;
    Ok(parameters)
}

/// A whole number that V16 writes as **text of digits** (`20`) or as an integer of any width, as a scalar or an array of one element. Nothing else is a whole number here: a float (even `20.0`), a
/// sign, a space, a decimal point and a digit of another script are `Corrupt`, and text of more digits than a signed 64-bit integer holds is `OutOfWidth`. It is built from the toolkit's readers of
/// text and of an integer, and it stays here until a second version needs it (decision D12-12: the toolkit holds what two versions use).
pub fn optional_whole_number(location: &Location, name: &str) -> Result<Option<i64>, Error> {
    match optional_text(location, name) {
        Ok(None) => Ok(None),
        Ok(Some(text)) => {
            if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
                return Err(corrupt(format!(
                    "the attribute `{name}` holds `{text}`, which is not a whole number written in digits"
                )));
            }
            text.parse::<i64>().map(Some).map_err(|_| {
                Error::new(ErrorKind::OutOfWidth {
                    value: text.clone(),
                    stored_as: "a signed 64-bit integer".to_string(),
                })
            })
        }
        // not text: an integer is a whole number, and anything else (a float, several values) is refused by the reader of integers, which names the attribute
        Err(error) if matches!(error.kind(), ErrorKind::Corrupt { .. }) => {
            optional_integer(location, name)
        }
        Err(error) => Err(error),
    }
}

/// The two bit resolutions of the mount, each as a number, as nothing, or as the reason it could not be read.
fn bit_resolutions(mount: &Group) -> Result<Vec<BitResolution>, Error> {
    let mut found = Vec::new();
    for (attribute, field) in BIT_RESOLUTIONS {
        match optional_whole_number(mount, attribute) {
            Err(error) if !is_about_the_content(&error) => return Err(error),
            outcome => found.push((attribute, field, outcome)),
        }
    }
    Ok(found)
}

/// Gives the mount's bit resolutions to the three galvos of one snapshot, each with the origin that says the value was global in the file. A bit resolution that could not be read is reported on
/// each galvo's value instead. Returns, for each bit resolution, whether a value was given.
fn give_bit_resolutions(parameters: &mut LaserParameters, bits: &[BitResolution]) -> Vec<bool> {
    let mut given = Vec::new();
    for (_, field, outcome) in bits {
        let mut gave = false;
        for galvo in GALVOS {
            let name = format!("{galvo}{field}");
            match outcome {
                Ok(Some(number)) => {
                    // the names are the CDM's: a value that the snapshot does not accept is a fault of this table, never of the file
                    parameters
                        .set(&name, ParameterValue::Whole(*number))
                        .expect("the CDM names the bit resolution of a galvo");
                    parameters
                        .origins
                        .record(name, Origin::Other(BIT_RESOLUTION_GLOBAL.to_string()));
                    gave = true;
                }
                Ok(None) => {}
                Err(error) => {
                    parameters.origins.record(name, Origin::not_read(error));
                }
            }
        }
        given.push(gave);
    }
    given
}

/// Reads the snapshot of every laser that has a calibration, and says what was taken from the file. A calibration that cannot be read is reported on its laser and kept whole by
/// [`keep_the_rest`].
pub fn read_all(mount: &Group, build: &mut BuildMeta) -> Result<Taken, Error> {
    let mut taken = Taken::default();
    let calibration = match child(mount, layout::CALIBRATION_DATA) {
        Ok(Some(calibration)) => calibration,
        Ok(None) => return Ok(taken),
        Err(error) if is_about_the_content(&error) => {
            // the calibration group is something else: no laser has a snapshot, each says why, and the node is kept by `keep_the_rest`
            let keys: Vec<_> = build.lasers().iter().map(|l| l.key.clone()).collect();
            for key in keys {
                build
                    .laser_mut(&key)
                    .expect("a laser of the build")
                    .origins
                    .record("parameters", Origin::not_read(&error));
            }
            return Ok(taken);
        }
        Err(error) => return Err(error),
    };
    let bits = bit_resolutions(mount)?;
    let mut given = vec![false; bits.len()];
    let keys: Vec<_> = build.lasers().iter().map(|l| l.key.clone()).collect();
    for key in keys {
        let name = format!("{}{}", layout::CALIBRATION_PREFIX, key.number);
        let outcome = match child(&calibration, &name) {
            Ok(Some(group)) => read_calibration(&group),
            Ok(None) => continue,
            Err(error) => Err(error),
        };
        let laser = build.laser_mut(&key).expect("a laser of the build");
        match outcome {
            Ok(mut parameters) => {
                for (at, gave) in give_bit_resolutions(&mut parameters, &bits)
                    .into_iter()
                    .enumerate()
                {
                    given[at] |= gave;
                }
                laser.parameters = Some(parameters);
                taken.calibrations.push(name);
            }
            Err(error) if is_about_the_content(&error) => {
                laser.origins.record("parameters", Origin::not_read(&error));
            }
            Err(error) => return Err(error),
        }
    }
    // a bit resolution is the mount's own until a snapshot has it: it is taken from the mount only where it was given
    for ((attribute, _, _), gave) in bits.iter().zip(given) {
        if gave {
            taken.mount_attributes.push(attribute);
        }
    }
    Ok(taken)
}

/// What the calibration group holds that no snapshot took: the formula images beside the calibrations, a calibration of a laser that the build does not have, and a calibration that could not be
/// read. They are kept in the build's `extra` at their place. A calibration group that is not a group is kept as the node that it is.
pub fn keep_the_rest(mount: &Group, taken: &Taken, build: &mut BuildMeta) -> Result<(), Error> {
    if !mount.link_exists(layout::CALIBRATION_DATA) {
        return Ok(());
    }
    match mount.group(layout::CALIBRATION_DATA) {
        Ok(calibration) => {
            let read: Vec<&str> = taken.calibrations.iter().map(String::as_str).collect();
            keep_group_remainder(
                &calibration,
                &read,
                &[],
                &[layout::CALIBRATION_DATA.to_string()],
                &mut build.extra,
            )
        }
        Err(_) => keep_node(mount, layout::CALIBRATION_DATA, &[], &mut build.extra),
    }
}
