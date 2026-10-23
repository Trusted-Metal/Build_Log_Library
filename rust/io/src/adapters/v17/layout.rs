//! The layout of V17 (IP-13 phase 13B; requirement BL-ST-13): **every group, dataset and attribute name of the version, and nowhere else.**
//!
//! A name is the knowledge of the version and not of the code that reads it, so the other files of this folder say `layout::BUILD_ID` and never spell the
//! name. A guard (`layout_names_appear_only_in_the_layout_module`, `tests/adapter_isolation.rs`) reads the constants of this file and fails if another file of
//! the folder holds one as a string. Every path is **relative to the mount** (decision D04-9).
//!
//! The names here are the ones the files show: tauri's output of every month, current and March alike (the two flavors differ in how a value is stored, not in
//! these names). The layers, lasers and channels join this file with the phases that read them (13C and 13D).

/// The mount: the group that holds the build, and carries the marker.
pub const MOUNT: &str = "Built_Part";

/// The marker, an attribute of the mount: the version of the file. It is the version itself, so it is read by the dispatcher and is not kept.
pub const MARKER: &str = "File_Version";

// ---------------------------------------------------------------- the attributes of the mount that the build level types

/// The build's id (text).
pub const BUILD_ID: &str = "Build_ID";
/// The build's date, as the string the file gives.
pub const DATE: &str = "Date";
/// The operator's name (text).
pub const USERNAME: &str = "Username";
/// The alloy (text; empty in the current output).
pub const ALLOYS: &str = "Alloys";
/// The number of parts.
pub const NUMBER_OF_PARTS: &str = "Number_Of_Parts";
/// The number of layers the file declares.
pub const LAYER_QUANTITY: &str = "Layer_Quantity";
/// The layer thickness of the build (absent in the March files).
pub const LAYER_THICKNESS: &str = "Layer_Thickness";

// ---------------------------------------------------------------- the layers (phase 13C)

/// The group of the mount that holds the layers, one group for each.
pub const MONITORING_DATA: &str = "Monitoring_Data";
/// What the name of a layer's group starts with; the layer's id, in digits, follows it.
pub const LAYER_PREFIX: &str = "Layer_";
/// The start of a layer, as an integer (the file states no unit).
pub const START_LAYER_TIME: &str = "Start_Layer_Time";
/// The end of a layer, as an integer.
pub const END_LAYER_TIME: &str = "End_Layer_Time";
/// The thickness that the file says was laid. A placeholder in every file we hold.
pub const ACTUAL_LAYER_THICKNESS: &str = "Actual_Layer_Thickness";
/// The thickness that the file says was commanded. A placeholder in every file we hold.
pub const COMMANDED_LAYER_THICKNESS: &str = "Commanded_Layer_Thickness";

/// The attributes of a layer's group that the layer level reads. Every other attribute of the group is kept in the layer's `extra`.
pub const TYPED_ON_A_LAYER: [&str; 4] = [
    START_LAYER_TIME,
    END_LAYER_TIME,
    ACTUAL_LAYER_THICKNESS,
    COMMANDED_LAYER_THICKNESS,
];

// ---------------------------------------------------------------- the lasers (phase 13D)

/// What the name of a laser's group (a child of a layer's group) starts with; the laser's number, in digits, follows it.
pub const LASER_PREFIX: &str = "Laser_";
/// The start of a laser's stream in a layer, as an integer. Absent from the March files.
pub const START_LASER_TIME: &str = "Start_Laser_Time";
/// The end of a laser's stream in a layer, as an integer. Absent from the March files.
pub const END_LASER_TIME: &str = "End_Laser_Time";
/// The attributes of a laser's group that the laser level reads. Every other attribute of the group is kept in the extra of the laser in the layer.
pub const TYPED_ON_A_LASER: [&str; 2] = [START_LASER_TIME, END_LASER_TIME];

/// The group of a laser that holds its power and its state.
pub const LASER_DATA: &str = "Laser_Data";
/// The laser's power, one sample for each sample of the stream. Its length is the length of the stream.
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

/// The group of the mount that holds the calibration, with one group for each optical train.
pub const CALIBRATION_DATA: &str = "Calibration_Data";
/// What the name of an optical train's group starts with; the train's number follows it, and laser N uses train N (decision D13-18).
pub const OPTICAL_TRAIN_PREFIX: &str = "Optical_Train_";
/// The group of an optical train that holds the scanner card.
pub const SCANNER_CARD: &str = "Scanner_Card";
/// The scanner card's sample period, a number with no unit: the input of a laser's time axis.
pub const SAMPLE_PERIOD: &str = "Sample_Period";

// ---------------------------------------------------------------- beside the mount (decision D13-19)

/// A group of V17 that sits next to the mount, not in it.
pub const CONFIGURATION: &str = "Configuration";
/// The OPC UA events that belong to no layer; also next to the mount.
pub const GLOBAL_SENSORS: &str = "Global_Sensors";
/// The groups beside the mount that V17 defines. Any other group beside the mount is not V17's and is not kept.
pub const BESIDE_THE_MOUNT: [&str; 2] = [CONFIGURATION, GLOBAL_SENSORS];

/// The attributes of the mount that the build level reads or consumes, the marker included. Every other attribute of the mount is kept in `extra`.
pub const TYPED_ON_THE_MOUNT: [&str; 8] = [
    MARKER,
    BUILD_ID,
    DATE,
    USERNAME,
    ALLOYS,
    NUMBER_OF_PARTS,
    LAYER_QUANTITY,
    LAYER_THICKNESS,
];
