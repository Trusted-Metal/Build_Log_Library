//! The layout of V16 (IP-12 phase 12C; requirement BL-ST-13): **every group, dataset and attribute name of the version, and nowhere else.**
//!
//! A name is the knowledge of the version and not of the code that reads it, so the other files of this folder say `layout::BUILD_ID` and never spell the name. A guard
//! (`layout_names_appear_only_in_the_layout_module`, `tests/adapter_isolation.rs`) reads the constants of this file and fails if another file of the folder holds one as a string.
//! Every path is **relative to the mount** (decision D04-9).
//!
//! The names here are the ones the V16 miniature shows, and the trimmed file and the original agree with it (decision D12-8: V16 has one shape). V16 shares the mount's name and the
//! marker with V17, and most of the build's attributes; it has **no `Layer_Thickness` on the mount**, no build status and no join, and a layer's thicknesses are the placeholder `1.0`. The
//! calibration (phase 12E) joins this file with the phase that reads it.

use crate::toolkit::context::{ContextKind, ContextRow, ContextTable};

/// The mount: the group that holds the build, and carries the marker.
pub const MOUNT: &str = "Built_Part";

/// The marker, an attribute of the mount: the version of the file, an `int32` scalar. It is the version itself, so it is read by the dispatcher and is not kept.
pub const MARKER: &str = "File_Version";

// ---------------------------------------------------------------- the attributes of the mount that the build level types

/// The build's id (text).
pub const BUILD_ID: &str = "Build_ID";
/// The build's date, as the string the file gives (`2026-01-26 19:37:22 UTC-05:00`: its offset label looks wrong, and nothing is corrected).
pub const DATE: &str = "Date";
/// The operator's name (text).
pub const USERNAME: &str = "Username";
/// The alloy (text).
pub const ALLOYS: &str = "Alloys";
/// The number of parts.
pub const NUMBER_OF_PARTS: &str = "Number_Of_Parts";
/// The number of layers the file declares (an array of one element in the file).
pub const LAYER_QUANTITY: &str = "Layer_Quantity";

/// The attributes of the mount that the build level reads or consumes, the marker included. Every other attribute of the mount is kept in `extra`.
pub const TYPED_ON_THE_MOUNT: [&str; 7] = [
    MARKER,
    BUILD_ID,
    DATE,
    USERNAME,
    ALLOYS,
    NUMBER_OF_PARTS,
    LAYER_QUANTITY,
];

// ---------------------------------------------------------------- the layers

/// The group of the mount that holds the layers, one group for each. It carries the build's sample period.
pub const MONITORING_DATA: &str = "Monitoring_Data";
/// The build's sample period, an attribute of the layers' group: **one value for the whole build**, written as text that has its unit in it (`10us`). V16 has no sample period for each laser.
pub const SAMPLE_PERIOD: &str = "Sample_Period";
/// What the name of a layer's group starts with; the layer's id, in digits, follows it.
pub const LAYER_PREFIX: &str = "Layer_";
/// The start of a layer, as an integer. The file states no unit.
pub const START_LAYER_TIME: &str = "Start_Layer_Time";
/// The end of a layer, as an integer.
pub const END_LAYER_TIME: &str = "End_Layer_Time";
/// The thickness that the file says was laid: the placeholder `1.0` in every file we hold.
pub const ACTUAL_LAYER_THICKNESS: &str = "Actual_Layer_Thickness";
/// The thickness that the file says was commanded: the placeholder `1.0` in every file we hold.
pub const COMMANDED_LAYER_THICKNESS: &str = "Commanded_Layer_Thickness";

/// The attributes of a layer's group that the layer level reads. Every other attribute of the group is kept in the layer's `extra`.
pub const TYPED_ON_A_LAYER: [&str; 4] = [
    START_LAYER_TIME,
    END_LAYER_TIME,
    ACTUAL_LAYER_THICKNESS,
    COMMANDED_LAYER_THICKNESS,
];

/// What V16 writes for a layer's thickness when it does not know it (decision J4): `1.0`, **where V17 writes `0.0`**, so a thickness of `1.0` is not a thickness in V16. The value is not
/// recorded and the layer's origin for it keeps the raw spelling.
pub const LAYER_THICKNESS_PLACEHOLDERS: [f64; 1] = [1.0];

// ---------------------------------------------------------------- the lasers (phase 12D)

/// What the name of a laser's group (a child of a layer's group) starts with; the laser's number, in digits, follows it. **V16 keeps the group of a laser that did not fire**, with its two groups of
/// channels and no channel in them (decision D12-4).
pub const LASER_PREFIX: &str = "Laser_";

/// The group of a laser that holds its power and its state.
pub const LASER_DATA: &str = "Laser_Data";
/// The laser's power, one sample for each sample of the stream.
pub const LASER_POWER: &str = "Laser_Power";
/// Whether the laser is on.
pub const LASER_STATE: &str = "Laser_State";
/// The group of a laser that holds its positions.
pub const POSITION: &str = "Position";
pub const ACTUAL_X: &str = "Actual_X";
pub const ACTUAL_Y: &str = "Actual_Y";
pub const ACTUAL_Z: &str = "Actual_Z";
pub const COMMANDED_X: &str = "Commanded_X";
pub const COMMANDED_Y: &str = "Commanded_Y";
pub const COMMANDED_Z: &str = "Commanded_Z";

// ---------------------------------------------------------------- the calibration (phase 12E; decisions D12-6 and D12-7, Q12-2)

/// The group of the mount that holds the calibration: one group for each laser, and the two formula images beside them.
pub const CALIBRATION_DATA: &str = "Laser_Calibration_Data";
/// What the name of a laser's calibration starts with; **the laser's number follows it, and it is the laser's own calibration** (V16 has no optical train).
pub const CALIBRATION_PREFIX: &str = "Laser_Calibration_";

// The 13 attributes of a laser's calibration that have a name in the CDM (Appendix B of the plan). The 12 others are not named here, on purpose: they stay in the snapshot's `extra` as the file
// has them, and naming one is an addition for the owner to decide.
pub const LASER_MODEL: &str = "Laser_Model";
pub const SCAN_HEAD_MODEL: &str = "Scan_Head_Model";
pub const SCANNER_FIELD_SIZE_X: &str = "Scanner_Field_Size_X";
pub const SCANNER_FIELD_SIZE_Y: &str = "Scanner_Field_Size_Y";
pub const SCANNER_FIELD_SIZE_Z: &str = "Scanner_Field_Size_Z";
pub const SCANNER_OFFSET_X: &str = "Scanner_Offset_X";
pub const SCANNER_OFFSET_Y: &str = "Scanner_Offset_Y";
pub const SCANNER_OFFSET_Z: &str = "Scanner_Offset_Z";
pub const SCALING_X: &str = "Scaling_X";
pub const SCALING_Y: &str = "Scaling_Y";
pub const MAJOR_AXIS_ANGLE: &str = "Major_Axis_Angle";
pub const RAYLEIGH_LENGTH_MAJOR_AXIS: &str = "Rayleigh_Length_Major_Axis";
pub const RAYLEIGH_LENGTH_MINOR_AXIS: &str = "Rayleigh_Length_Minor_Axis";

/// The correction grid of a laser's calibration (IP-24 phase 24B): a dataset of the calibration's own group, `int32` of 2 by 257 by 257 in the files we hold. **No file states its domain or the order of
/// its axes, and the name does not say whether it is the grid or its inverse.** It is not an attribute, so it is not among the thirteen that have a name in the CDM.
pub const CORRECTION_DATA: &str = "CorrectionData";

/// The two bit resolutions of the mount, written as text of digits (`20`): **one value for every laser and every axis** (IP-24 finding F2). They are given to each laser's three galvos.
pub const ACTUAL_BIT_RESOLUTION: &str = "Actual_Bit_Resolution";
pub const COMMANDED_BIT_RESOLUTION: &str = "Commanded_Bit_Resolution";

// ---------------------------------------------------------------- the build's context (IP-02 phase 2V; decisions D02-20 and D02-21)

/// The machine's serial number (text).
pub const MACHINE_SERIAL: &str = "Machine_Serial";
/// The scanner's identifier (text).
pub const SCANNER_ID: &str = "Scanner_ID";
/// The identifier of the material's parameter set (text, `NA` in the files we hold).
pub const PARAMETER_ID_FOR_MATERIAL: &str = "Parameter_ID_For_Material";
/// The kind of recoater blade (text).
pub const RECOATER_BLADE_TYPE: &str = "Recoater_Blade_Type";
/// The height the build starts at (a number; no file states its unit). The placeholder `1.0` in every file we hold.
pub const STARTING_HEIGHT: &str = "Starting_Height";
/// The plate's thickness and its weight (numbers; no file states their unit). The placeholder `1.0` in every file we hold.
pub const BUILD_PLATE_THICKNESS: &str = "Build_Plate_Thickness";
pub const BUILD_PLATE_WEIGHT: &str = "Build_Plate_Weight";

/// The attributes of the mount that make the build's context: the attribute, the name the CDM gives its value, its kind (the toolkit's), and the attribute that holds its unit (V16 has none).
/// The adapter says which attribute holds which name and nothing else: the Model maps a name to a field. **V16 has the plate's thickness and weight and neither vocabulary of its shape.**
pub const CONTEXT: [ContextRow; 7] = [
    (MACHINE_SERIAL, "machine_serial", ContextKind::Text, None),
    (SCANNER_ID, "scanner_id", ContextKind::Text, None),
    (
        PARAMETER_ID_FOR_MATERIAL,
        "material_parameter_id",
        ContextKind::Text,
        None,
    ),
    (
        RECOATER_BLADE_TYPE,
        "recoater_blade",
        ContextKind::Text,
        None,
    ),
    (
        STARTING_HEIGHT,
        "starting_height",
        ContextKind::Quantity,
        None,
    ),
    (
        BUILD_PLATE_THICKNESS,
        "plate.thickness",
        ContextKind::Quantity,
        None,
    ),
    (
        BUILD_PLATE_WEIGHT,
        "plate.weight",
        ContextKind::Quantity,
        None,
    ),
];

/// Every attribute of the mount that the context types: each value's attribute, and the attribute of each unit. The build level does not keep them in `extra`.
pub fn context_attributes() -> Vec<&'static str> {
    CONTEXT
        .iter()
        .flat_map(|(attribute, _, _, unit)| std::iter::once(*attribute).chain(*unit))
        .collect()
}

/// What V16 writes for a value of the context when it does not know it (J4, decision D02-21), by the CDM's name: **`1.0`, for the starting height, the plate's thickness and its weight**. A
/// value of `1.0` is not a possible value of any of the three, so it is not recorded and the build's origin for it keeps the raw spelling.
pub const CONTEXT_PLACEHOLDERS: [(&str, f64); 3] = [
    ("starting_height", 1.0),
    ("plate.thickness", 1.0),
    ("plate.weight", 1.0),
];

/// The two tables above, as the toolkit's reader of the context takes them (IP-12 phase 12A).
pub const CONTEXT_TABLE: ContextTable = ContextTable {
    rows: &CONTEXT,
    placeholders: &CONTEXT_PLACEHOLDERS,
};
