//! The parameters of a V17 laser (IP-13 phase 13H; decisions D13-16, D13-18 and D13-25): the calibration of its optical train, read into the Model's
//! [`LaserParameters`].
//!
//! * **Laser N uses optical train N.** The files do not say which train a laser uses (the train's own `ID` and `Scanner_Number` are empty text in the gate file), so
//!   the adapter pairs them by number and the snapshot's origin says so; the owner confirms it is always right (decision D13-18).
//! * **Each value is read as the file gave it.** A number is a number whether the file stored it as a float or as an integer (the 2026-10-15 file has `int32` beam
//!   values); its unit is the text of its companion attribute where there is one, and unknown where there is not. Text is text, a whole number stays whole, and a flag
//!   is 0 or 1.
//! * **A name is held by one attribute.** The tuning type of a galvo is `Tuning_Type` in the current output and `Galvo_Tuning_Type` in the March files; a group
//!   that holds both is a fault, and the first is not taken.
//! * **The equation is named constants** (decision D13-25): `LINEAR` and `POLYNOMIAL`, matched exactly, give `b` and `a` (the intercept first) and `c0`, `c1` and so on
//!   from the positional text, with the origin `DeclaredByAdapter` because the names are the adapter's; any other algorithm is kept as written with no constants, and
//!   its text of constants stays in `extra`.
//! * **A power limit that is text is not a number.** The March files hold `Power_Min` and `Power_Max` as text that the old writer filled with the bytes of a number; the
//!   adapter does not read them as numbers, says so in the origin `light_source.power_limits`, and keeps the two attributes as they are (decision D13-16).
//! * **What is not typed is kept**, in the snapshot's `extra`, by its path below the optical train: the correction grids, the attributes that are not in the tables
//!   and any group or dataset that is not known. Nothing is dropped.
//! * **A train that cannot be read is reported on its laser** (decision D13-21): the laser has no snapshot, an origin says why, and the whole train is kept in the
//!   build's `extra`. Only damage to the file itself stops the open.
//!
//! Every name comes from [`super::layout`].

use std::collections::BTreeSet;

use build_log_model::{
    parse_embedded, BuildMeta, CorrectionGrid, Equation, EquationKind, Error, ErrorKind,
    GalvoProtocol, GridRole, LaserParameters, Origin, ParameterValue, Quantity,
};
use hdf5::Group;

use super::layout;
use super::KEY;
use crate::columns::describe_grid;
use crate::hdf5::{keep_group_remainder, keep_other_attributes, keep_other_nodes};
use crate::toolkit::content::is_about_the_content;
use crate::toolkit::values::{
    optional_integer, optional_number, optional_text, optional_written, Written,
};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// What kind of value an attribute holds, which decides how it is read.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    /// A number with a unit that the companion attribute states, where the row names one.
    Quantity,
    /// A number that has no unit.
    Plain,
    Whole,
    Flag,
    Protocol,
    /// A period: a number with no unit, or text with a unit in it (`10us`); both are one quantity.
    Period,
}

/// One attribute, the name the CDM gives its value, and how it is read.
struct Row {
    attribute: &'static str,
    name: &'static str,
    kind: Kind,
    /// The attribute that holds the unit of the value, where the file has one.
    unit: Option<&'static str>,
}

const fn row(attribute: &'static str, name: &'static str, kind: Kind) -> Row {
    Row {
        attribute,
        name,
        kind,
        unit: None,
    }
}

const fn with_unit(attribute: &'static str, name: &'static str, unit: &'static str) -> Row {
    Row {
        attribute,
        name,
        kind: Kind::Quantity,
        unit: Some(unit),
    }
}

/// The attributes of the optical train's own group.
const TRAIN_ROWS: &[Row] = &[
    row(layout::OPTICAL_TRAIN_ID, "optics.id", Kind::Text),
    row(layout::SCANNER_NUMBER, "optics.scanner_number", Kind::Text),
    row(layout::METADATA, "optics.metadata", Kind::Text),
    row(layout::BEAM_PROFILE_TYPE, "optics.profile_type", Kind::Text),
    row(
        layout::BEAM_WAIST_DEFINITION,
        "optics.waist_definition",
        Kind::Text,
    ),
    with_unit(
        layout::BEAM_WAIST_MAJOR,
        "optics.waist_major",
        layout::BEAM_WAIST_MAJOR_UNIT,
    ),
    with_unit(
        layout::BEAM_WAIST_MINOR,
        "optics.waist_minor",
        layout::BEAM_WAIST_MINOR_UNIT,
    ),
    with_unit(
        layout::BEAM_WAIST_OFFSET_Z,
        "optics.waist_offset_z",
        layout::BEAM_WAIST_OFFSET_Z_UNIT,
    ),
    with_unit(
        layout::BUILD_PLANE_OFFSET_MAJOR,
        "optics.build_plane_offset_major",
        layout::BUILD_PLANE_OFFSET_MAJOR_UNIT,
    ),
    with_unit(
        layout::BUILD_PLANE_OFFSET_MINOR,
        "optics.build_plane_offset_minor",
        layout::BUILD_PLANE_OFFSET_MINOR_UNIT,
    ),
    with_unit(
        layout::COLLIMATOR_FOCAL_LENGTH,
        "optics.collimator_focal_length",
        layout::COLLIMATOR_FOCAL_LENGTH_UNIT,
    ),
    row(layout::M2_MAJOR, "optics.m2_major", Kind::Plain),
    row(layout::M2_MINOR, "optics.m2_minor", Kind::Plain),
    with_unit(
        layout::MAJOR_AXIS_ANGLE,
        "optics.major_axis_angle",
        layout::MAJOR_AXIS_ANGLE_UNIT,
    ),
    row(
        layout::MAJOR_WAIST_D4SIGMA,
        "optics.major_waist_d4sigma",
        Kind::Quantity,
    ),
    row(
        layout::MINOR_WAIST_D4SIGMA,
        "optics.minor_waist_d4sigma",
        Kind::Quantity,
    ),
    row(
        layout::NOMINAL_WORKING_DISTANCE,
        "optics.nominal_working_distance",
        Kind::Quantity,
    ),
    row(
        layout::OPTICAL_TRAIN_DEFAULT_POSITION,
        "optics.default_position",
        Kind::Quantity,
    ),
    row(
        layout::OPTICAL_TRAIN_MOVEABLE,
        "optics.moveable",
        Kind::Flag,
    ),
    row(
        layout::OPTICAL_TRAIN_POSITION,
        "optics.position",
        Kind::Quantity,
    ),
    with_unit(
        layout::RAYLEIGH_LENGTH_MAJOR,
        "optics.rayleigh_length_major",
        layout::RAYLEIGH_LENGTH_MAJOR_UNIT,
    ),
    with_unit(
        layout::RAYLEIGH_LENGTH_MINOR,
        "optics.rayleigh_length_minor",
        layout::RAYLEIGH_LENGTH_MINOR_UNIT,
    ),
    with_unit(
        layout::THERMAL_LENSING_FOCAL_PLANE_SHIFT,
        "optics.thermal_lensing_focal_plane_shift",
        layout::THERMAL_LENSING_FOCAL_PLANE_SHIFT_UNIT,
    ),
    row(
        layout::THERMAL_LENSING_TEST_PASSED,
        "optics.thermal_lensing_test_passed",
        Kind::Flag,
    ),
    with_unit(
        layout::THERMAL_LENSING_THRESHOLD,
        "optics.thermal_lensing_threshold",
        layout::THERMAL_LENSING_THRESHOLD_UNIT,
    ),
];

/// The attributes of the scanner's group.
const SCANNER_ROWS: &[Row] = &[
    row(layout::SCAN_HEAD_MODEL, "scanner.model", Kind::Text),
    row(layout::SCANNER_SERIAL_NUMBER, "scanner.serial", Kind::Text),
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
    row(layout::SCANNER_OFFSET_X, "scanner.offset_x", Kind::Quantity),
    row(layout::SCANNER_OFFSET_Y, "scanner.offset_y", Kind::Quantity),
    row(layout::SCANNER_OFFSET_Z, "scanner.offset_z", Kind::Quantity),
    row(layout::SCANNER_ROTATION, "scanner.rotation", Kind::Quantity),
    row(layout::SCALING_X, "scanner.scaling_x", Kind::Plain),
    row(layout::SCALING_Y, "scanner.scaling_y", Kind::Plain),
    row(
        layout::X_GALVO_TO_EXIT_APERTURE,
        "scanner.x_galvo_to_exit_aperture",
        Kind::Quantity,
    ),
    row(
        layout::Y_GALVO_TO_EXIT_APERTURE,
        "scanner.y_galvo_to_exit_aperture",
        Kind::Quantity,
    ),
];

/// The attributes of a galvo's group, by the name of the value below the galvo's own name (`galvo_x.`, `galvo_y.` or `galvo_z.`).
const GALVO_ROWS: &[Row] = &[
    row(
        layout::ACTUAL_BIT_RESOLUTION,
        "actual_bit_resolution",
        Kind::Whole,
    ),
    row(
        layout::COMMANDED_BIT_RESOLUTION,
        "commanded_bit_resolution",
        Kind::Whole,
    ),
    row(layout::COMMUNICATION_PROTOCOL, "protocol", Kind::Protocol),
    row(
        layout::GALVO_RANGE_OF_MOTION,
        "range_of_motion",
        Kind::Quantity,
    ),
    row(layout::SMOOTHING_KERNEL, "smoothing_kernel", Kind::Text),
    row(
        layout::SMOOTHING_PARAMETERS,
        "smoothing_parameters",
        Kind::Plain,
    ),
    row(layout::TUNING_PARAMETERS, "tuning_parameters", Kind::Text),
    row(layout::TUNING_TYPE, "tuning_type", Kind::Text),
    row(layout::GALVO_TUNING_TYPE, "tuning_type", Kind::Text),
];

/// The attributes of the scanner card's group.
const CARD_ROWS: &[Row] = &[
    row(
        layout::ACTUAL_POSITION_TIMING_OFFSET,
        "scanner_card.actual_position_timing_offset",
        Kind::Whole,
    ),
    row(
        layout::COMMANDED_POSITION_TIMING_OFFSET,
        "scanner_card.commanded_position_timing_offset",
        Kind::Whole,
    ),
    row(
        layout::SAMPLE_PERIOD,
        "scanner_card.sample_period",
        Kind::Period,
    ),
];

/// The attributes of the light source's group that are values; the equation is read apart.
const LIGHT_ROWS: &[Row] = &[
    row(layout::LASER_MODEL, "light_source.model", Kind::Text),
    row(
        layout::LASER_SERIAL_NUMBER,
        "light_source.serial",
        Kind::Text,
    ),
    with_unit(
        layout::LIGHT_WAVELENGTH,
        "light_source.wavelength",
        layout::LIGHT_WAVELENGTH_UNIT,
    ),
    row(
        layout::POWER_BIT_RESOLUTION,
        "light_source.power_bit_resolution",
        Kind::Whole,
    ),
    row(
        layout::POWER_MAX_ACTUAL,
        "light_source.power_max_actual",
        Kind::Quantity,
    ),
    row(
        layout::POWER_MAX_NOMINAL,
        "light_source.power_max_nominal",
        Kind::Quantity,
    ),
    row(
        layout::POWER_MIN_ACTUAL,
        "light_source.power_min_actual",
        Kind::Quantity,
    ),
    row(
        layout::POWER_MIN_NOMINAL,
        "light_source.power_min_nominal",
        Kind::Quantity,
    ),
];

/// Every attribute name that the parameters type in the group at `path` below an optical train (the units and the equation's two attributes included), so that a
/// test can tell what is typed from what is kept. An empty path is the train's own group.
pub fn typed_names(path: &[&str]) -> Vec<&'static str> {
    let rows: &[Row] = match path {
        [] => TRAIN_ROWS,
        [group] if *group == layout::SCANNER => SCANNER_ROWS,
        [group, galvo] if *group == layout::SCANNER && GALVOS.iter().any(|(g, _)| g == galvo) => {
            GALVO_ROWS
        }
        [group] if *group == layout::SCANNER_CARD => CARD_ROWS,
        [group] if *group == layout::LIGHT_SOURCE => LIGHT_ROWS,
        _ => &[],
    };
    let mut names: Vec<&'static str> = rows
        .iter()
        .flat_map(|row| std::iter::once(row.attribute).chain(row.unit))
        .collect();
    if path == [layout::LIGHT_SOURCE] {
        names.extend([
            layout::VOLTS_TO_WATTS_ALGORITHM,
            layout::VOLTS_TO_WATTS_PARAMS,
        ]);
    }
    names
}

/// The sample period of a scanner card as one quantity, whether the file wrote it as a number with no unit (`10.0`) or as text that has the unit in it (`10us`):
/// the value as written, and a unit only where the file states one (decision D13-25).
pub fn sample_period(card: &Group) -> Result<Option<Quantity>, Error> {
    match optional_written(card, layout::SAMPLE_PERIOD)? {
        None => Ok(None),
        Some(Written::Number(value)) => Ok(Some(Quantity::from_file(value, None))),
        Some(Written::Text(text)) => parse_embedded(&text).map(Some).ok_or_else(|| {
            corrupt(format!(
                "the sample period `{text}` is neither a number nor a number with a unit"
            ))
        }),
    }
}

/// The value of one row of a group, and the unit attribute that it consumed, or `None` when the group does not have the attribute.
fn read_value(
    group: &Group,
    row: &Row,
) -> Result<Option<(ParameterValue, Option<&'static str>)>, Error> {
    let name = row.attribute;
    let value = match row.kind {
        Kind::Text => optional_text(group, name)?.map(ParameterValue::Text),
        Kind::Plain => optional_number(group, name)?.map(ParameterValue::Plain),
        Kind::Whole => optional_integer(group, name)?.map(ParameterValue::Whole),
        Kind::Flag => match optional_integer(group, name)? {
            None => None,
            Some(0) => Some(ParameterValue::Flag(false)),
            Some(1) => Some(ParameterValue::Flag(true)),
            Some(other) => {
                return Err(corrupt(format!(
                    "the flag `{name}` holds {other}, and only 0 and 1 mean anything"
                )))
            }
        },
        Kind::Protocol => optional_text(group, name)?.map(|text| {
            ParameterValue::Protocol(match text.as_str() {
                layout::PROTOCOL_SL2_100 => GalvoProtocol::Sl2100,
                layout::PROTOCOL_XY2_100 => GalvoProtocol::Xy2100,
                _ => GalvoProtocol::Other(text),
            })
        }),
        Kind::Quantity => {
            let Some(number) = optional_number(group, name)? else {
                return Ok(None);
            };
            let unit = match row.unit {
                Some(unit) => optional_text(group, unit)?,
                None => None,
            };
            let consumed = unit.as_ref().and(row.unit);
            return Ok(Some((
                ParameterValue::Quantity(Quantity::from_file(number, unit.as_deref())),
                consumed,
            )));
        }
        Kind::Period => sample_period(group)?.map(ParameterValue::Quantity),
    };
    Ok(value.map(|value| (value, None)))
}

/// Reads the rows of one group into the snapshot. `prefix` goes before each row's name (`galvo_x.` for a galvo), and the attributes that were read, units
/// included, are added to `typed`.
fn read_rows(
    group: &Group,
    rows: &[Row],
    prefix: &str,
    parameters: &mut LaserParameters,
    typed: &mut Vec<&'static str>,
) -> Result<(), Error> {
    let mut given: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        let Some((value, unit)) = read_value(group, row)? else {
            continue;
        };
        let name = format!("{prefix}{}", row.name);
        if !given.insert(name.clone()) {
            // two names for one value: neither is taken (decision D13-15)
            return Err(corrupt(format!(
                "two attributes of one group hold `{name}`; the first is not taken"
            )));
        }
        parameters.set(&name, value)?;
        typed.push(row.attribute);
        typed.extend(unit);
    }
    Ok(())
}

/// The groups the snapshot reads below the optical train, and what the group of each galvo is called.
const GALVOS: [(&str, &str); 3] = [
    (layout::GALVO_X, "galvo_x."),
    (layout::GALVO_Y, "galvo_y."),
    (layout::GALVO_Z, "galvo_z."),
];

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

/// The two grids of an optical train, by the name the file spells and by what that name says each is: the grid, and the one that is named as its inverse.
const GRIDS: [(&str, GridRole); 2] = [
    (layout::CORRECTION_DATA, GridRole::Forward),
    (layout::INVERSE_CORRECTION_DATA, GridRole::Inverse),
];

/// The descriptor of the grid `name` of a train, or `None` if the train has none by that name. **What a grid is, is the engine's to say** (`describe_grid`): a dataset that it cannot read is an
/// error here, and the caller keeps it untyped.
fn describe_one_grid(
    train: &Group,
    name: &str,
    role: GridRole,
) -> Result<Option<CorrectionGrid>, Error> {
    if !train.link_exists(name) {
        return Ok(None);
    }
    let dataset = train
        .dataset(name)
        .map_err(|e| corrupt(format!("`{name}` is not a dataset: {e}")))?;
    let (stored_as, extents) = describe_grid(&dataset, KEY)?;
    Ok(Some(CorrectionGrid::new(
        name,
        Some(role),
        stored_as,
        extents,
    )))
}

/// The snapshot of the optical train `train`, which is the one of the laser numbered `number`.
pub fn read_train(train: &Group, number: u32) -> Result<LaserParameters, Error> {
    let mut parameters = LaserParameters::new();
    parameters.optical_train_number = Some(number);
    // the files do not say which train a laser uses: laser N is paired with train N, by the owner's word (decision D13-18)
    parameters
        .origins
        .record("optical_train_number", Origin::DeclaredByAdapter);

    let mut typed: Vec<&'static str> = Vec::new();
    read_rows(train, TRAIN_ROWS, "", &mut parameters, &mut typed)?;
    // the correction grids (IP-24 phase 24B): a grid that the engine reads is described here and is not kept as an untyped node; one that it does not read stays where it is, and the snapshot says why
    let mut children: Vec<&str> = vec![layout::SCANNER, layout::SCANNER_CARD, layout::LIGHT_SOURCE];
    for (name, role) in GRIDS {
        match describe_one_grid(train, name, role) {
            Ok(Some(grid)) => {
                parameters.grids.push(grid);
                children.push(name);
            }
            Ok(None) => {}
            Err(error) if is_about_the_content(&error) => {
                parameters
                    .origins
                    .record(format!("grid {name}"), Origin::not_read(&error));
            }
            Err(error) => return Err(error),
        }
    }
    let mut extra = std::mem::take(&mut parameters.extra);
    keep_other_attributes(train, &typed, &mut extra)?;
    keep_other_nodes(train, &children, &[], &mut extra)?;

    if let Some(scanner) = child(train, layout::SCANNER)? {
        let mut typed: Vec<&'static str> = Vec::new();
        read_rows(&scanner, SCANNER_ROWS, "", &mut parameters, &mut typed)?;
        let mut galvos: Vec<&str> = Vec::new();
        for (name, prefix) in GALVOS {
            let Some(galvo) = child(&scanner, name)? else {
                continue;
            };
            let mut typed: Vec<&'static str> = Vec::new();
            read_rows(&galvo, GALVO_ROWS, prefix, &mut parameters, &mut typed)?;
            keep_group_remainder(
                &galvo,
                &[],
                &typed,
                &[layout::SCANNER.to_string(), name.to_string()],
                &mut extra,
            )?;
            galvos.push(name);
        }
        keep_group_remainder(
            &scanner,
            &galvos,
            &typed,
            &[layout::SCANNER.to_string()],
            &mut extra,
        )?;
    }

    if let Some(card) = child(train, layout::SCANNER_CARD)? {
        let mut typed: Vec<&'static str> = Vec::new();
        read_rows(&card, CARD_ROWS, "", &mut parameters, &mut typed)?;
        keep_group_remainder(
            &card,
            &[],
            &typed,
            &[layout::SCANNER_CARD.to_string()],
            &mut extra,
        )?;
    }

    if let Some(light) = child(train, layout::LIGHT_SOURCE)? {
        let mut typed: Vec<&'static str> = Vec::new();
        read_rows(&light, LIGHT_ROWS, "", &mut parameters, &mut typed)?;
        read_equation(&light, &mut parameters, &mut typed)?;
        // the March files hold the limits as text, which is not a number: the two attributes stay as they are, and the snapshot says it has no limits to give
        let limits_are_text =
            light.attr(layout::POWER_MIN).is_ok() || light.attr(layout::POWER_MAX).is_ok();
        let limits = &parameters.light_source;
        if limits_are_text
            && limits.power_max_actual.is_none()
            && limits.power_max_nominal.is_none()
            && limits.power_min_actual.is_none()
            && limits.power_min_nominal.is_none()
        {
            parameters.origins.record(
                "light_source.power_limits",
                Origin::Other("not recorded: the file stores text where a number belongs".into()),
            );
        }
        keep_group_remainder(
            &light,
            &[],
            &typed,
            &[layout::LIGHT_SOURCE.to_string()],
            &mut extra,
        )?;
    }

    parameters.extra = extra;
    Ok(parameters)
}

/// The volts-to-watts equation: the algorithm, and the constants of the two algorithms that are named, from the positional text, by name.
fn read_equation(
    light: &Group,
    parameters: &mut LaserParameters,
    typed: &mut Vec<&'static str>,
) -> Result<(), Error> {
    let Some(algorithm) = optional_text(light, layout::VOLTS_TO_WATTS_ALGORITHM)? else {
        return Ok(());
    };
    typed.push(layout::VOLTS_TO_WATTS_ALGORITHM);
    let kind = match algorithm.as_str() {
        layout::ALGORITHM_LINEAR => EquationKind::Linear,
        layout::ALGORITHM_POLYNOMIAL => EquationKind::Polynomial,
        _ => EquationKind::Other(algorithm),
    };
    let mut equation = Equation::new(kind.clone());
    if !matches!(kind, EquationKind::Other(_)) {
        let text = optional_text(light, layout::VOLTS_TO_WATTS_PARAMS)?.ok_or_else(|| {
            corrupt("the volts-to-watts algorithm is named and its constants are not given".into())
        })?;
        let mut values = Vec::new();
        for part in text.split(',') {
            let value: f64 = part.trim().parse().map_err(|_| {
                corrupt(format!(
                    "the volts-to-watts constants `{text}` are not a list of numbers"
                ))
            })?;
            values.push(value);
        }
        let names: Vec<String> = match kind {
            EquationKind::Linear => vec!["b".to_string(), "a".to_string()],
            _ => (0..values.len()).map(|i| format!("c{i}")).collect(),
        };
        if names.len() != values.len() || values.is_empty() {
            return Err(corrupt(format!(
                "the volts-to-watts algorithm takes {} constants and the file gives {}",
                names.len().max(1),
                values.len()
            )));
        }
        for (name, value) in names.into_iter().zip(values) {
            equation = equation.with_constant(name, value);
        }
        typed.push(layout::VOLTS_TO_WATTS_PARAMS);
        // the constants are named by the adapter, from the order of the text
        parameters
            .origins
            .record("light_source.volts_to_watts", Origin::DeclaredByAdapter);
    }
    parameters.light_source.volts_to_watts = Some(equation);
    Ok(())
}

/// Reads the snapshot of every laser that has an optical train, and returns the names of the trains that were read into a snapshot: the rest of the calibration group
/// is kept in the build's `extra`, and a train that could not be read is kept whole there (decision D13-21).
pub fn read_all(mount: &Group, build: &mut BuildMeta) -> Result<Vec<String>, Error> {
    let mut read = Vec::new();
    let Some(calibration) = child(mount, layout::CALIBRATION_DATA)? else {
        return Ok(read);
    };
    let keys: Vec<_> = build.lasers().iter().map(|l| l.key.clone()).collect();
    for key in keys {
        let name = format!("{}{}", layout::OPTICAL_TRAIN_PREFIX, key.number);
        let outcome = match child(&calibration, &name) {
            Ok(Some(train)) => read_train(&train, key.number),
            Ok(None) => continue,
            Err(error) => Err(error),
        };
        let laser = build.laser_mut(&key).expect("a laser of the build");
        match outcome {
            Ok(parameters) => {
                laser.parameters = Some(parameters);
                read.push(name);
            }
            Err(error) if is_about_the_content(&error) => {
                laser.origins.record("parameters", Origin::not_read(&error));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(read)
}
