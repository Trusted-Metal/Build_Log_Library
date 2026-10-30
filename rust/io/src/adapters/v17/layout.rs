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
/// The build's status, text. tauri writes `In_Progress` when it creates the file and `complete` when the build is marked finished, and only with its
/// `opcua-client` feature (CT-QE-13): the files of the 2026-08-18 capture and of 2026-10-19 have it.
pub const BUILD_STATUS: &str = "Build_Status";
/// The status of the post-capture join of OPC UA events to layers, text.
pub const OPCUA_JOIN_STATUS: &str = "Opcua_Join_Status";
/// The version of that join, text (`2` in the files we hold).
pub const OPCUA_JOIN_VERSION: &str = "Opcua_Join_Version";

/// The build status that says the build was marked finished. Matched exactly, as tauri writes it.
pub const STATUS_COMPLETE: &str = "complete";
/// The build status that tauri writes when it creates the file, and that stays if the build is never marked finished.
pub const STATUS_IN_PROGRESS: &str = "In_Progress";
/// The join status that says the join ran to its end.
pub const JOIN_STATUS_COMPLETE: &str = "complete";

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
/// Whether the laser received any point in the layer, a flag of one element, 0 or 1. tauri writes it from the current output of 2026-08-18 on, and it is
/// absent from the March files. It is read as written and never worked out from the samples.
pub const COMPLETE: &str = "complete";
/// The attributes of a laser's group that the laser level reads. Every other attribute of the group is kept in the extra of the laser in the layer.
pub const TYPED_ON_A_LASER: [&str; 3] = [START_LASER_TIME, END_LASER_TIME, COMPLETE];

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

// ---------------------------------------------------------------- the parameters of a laser: its optical train (phase 13H)

/// The groups below an optical train that the parameters read.
pub const SCANNER: &str = "Scanner";
pub const LIGHT_SOURCE: &str = "Light_Source";
/// The three galvos below the scanner, one for each axis.
pub const GALVO_X: &str = "Galvo_X";
pub const GALVO_Y: &str = "Galvo_Y";
pub const GALVO_Z: &str = "Galvo_Z";

// the attributes of the optical train's own group
pub const OPTICAL_TRAIN_ID: &str = "ID";
pub const SCANNER_NUMBER: &str = "Scanner_Number";
pub const METADATA: &str = "Metadata";
pub const BEAM_PROFILE_TYPE: &str = "Beam_Profile_Type";
pub const BEAM_WAIST_DEFINITION: &str = "Beam_Waist_Definition";
pub const BEAM_WAIST_MAJOR: &str = "Beam_Waist_Major";
pub const BEAM_WAIST_MAJOR_UNIT: &str = "Beam_Waist_Major_unit";
pub const BEAM_WAIST_MINOR: &str = "Beam_Waist_Minor";
pub const BEAM_WAIST_MINOR_UNIT: &str = "Beam_Waist_Minor_unit";
pub const BEAM_WAIST_OFFSET_Z: &str = "Beam_Waist_Offset_Z";
pub const BEAM_WAIST_OFFSET_Z_UNIT: &str = "Beam_Waist_Offset_Z_unit";
pub const BUILD_PLANE_OFFSET_MAJOR: &str = "Build_Plane_Offset_Major";
pub const BUILD_PLANE_OFFSET_MAJOR_UNIT: &str = "Build_Plane_Offset_Major_unit";
pub const BUILD_PLANE_OFFSET_MINOR: &str = "Build_Plane_Offset_Minor";
pub const BUILD_PLANE_OFFSET_MINOR_UNIT: &str = "Build_Plane_Offset_Minor_unit";
pub const COLLIMATOR_FOCAL_LENGTH: &str = "Collimator_Focal_Length";
pub const COLLIMATOR_FOCAL_LENGTH_UNIT: &str = "Collimator_Focal_Length_unit";
pub const M2_MAJOR: &str = "M2_Major";
pub const M2_MINOR: &str = "M2_Minor";
pub const MAJOR_AXIS_ANGLE: &str = "Major_Axis_Angle";
pub const MAJOR_AXIS_ANGLE_UNIT: &str = "Major_Axis_Angle_unit";
pub const MAJOR_WAIST_D4SIGMA: &str = "Major_Waist_D4Sigma";
pub const MINOR_WAIST_D4SIGMA: &str = "Minor_Waist_D4Sigma";
pub const NOMINAL_WORKING_DISTANCE: &str = "Nominal_Working_Distance";
pub const OPTICAL_TRAIN_DEFAULT_POSITION: &str = "Optical_Train_Default_Position";
pub const OPTICAL_TRAIN_MOVEABLE: &str = "Optical_Train_Moveable";
pub const OPTICAL_TRAIN_POSITION: &str = "Optical_Train_Position";
pub const RAYLEIGH_LENGTH_MAJOR: &str = "Rayleigh_Length_Major";
pub const RAYLEIGH_LENGTH_MAJOR_UNIT: &str = "Rayleigh_Length_Major_unit";
pub const RAYLEIGH_LENGTH_MINOR: &str = "Rayleigh_Length_Minor";
pub const RAYLEIGH_LENGTH_MINOR_UNIT: &str = "Rayleigh_Length_Minor_unit";
pub const THERMAL_LENSING_FOCAL_PLANE_SHIFT: &str = "Thermal_Lensing_Focal_Plane_Shift";
pub const THERMAL_LENSING_FOCAL_PLANE_SHIFT_UNIT: &str = "Thermal_Lensing_Focal_Plane_Shift_unit";
pub const THERMAL_LENSING_TEST_PASSED: &str = "Thermal_Lensing_Test_Passed";
pub const THERMAL_LENSING_THRESHOLD: &str = "Thermal_Lensing_Threshold";
pub const THERMAL_LENSING_THRESHOLD_UNIT: &str = "Thermal_Lensing_Threshold_unit";

// the attributes of the scanner's group
pub const SCAN_HEAD_MODEL: &str = "Scan_Head_Model";
pub const SCANNER_SERIAL_NUMBER: &str = "Scanner_Serial_Number";
pub const SCANNER_FIELD_SIZE_X: &str = "Scanner_Field_Size_X";
pub const SCANNER_FIELD_SIZE_Y: &str = "Scanner_Field_Size_Y";
pub const SCANNER_FIELD_SIZE_Z: &str = "Scanner_Field_Size_Z";
pub const SCANNER_OFFSET_X: &str = "Scanner_Offset_X";
pub const SCANNER_OFFSET_Y: &str = "Scanner_Offset_Y";
pub const SCANNER_OFFSET_Z: &str = "Scanner_Offset_Z";
pub const SCANNER_ROTATION: &str = "Scanner_Rotation";
pub const SCALING_X: &str = "Scaling_X";
pub const SCALING_Y: &str = "Scaling_Y";
/// Spelled as the files spell it.
pub const X_GALVO_TO_EXIT_APERTURE: &str = "X_Galvo_to_Exit_Aperature";
pub const Y_GALVO_TO_EXIT_APERTURE: &str = "Y_Galvo_to_Exit_Aperature";

// the attributes of a galvo's group
pub const ACTUAL_BIT_RESOLUTION: &str = "Actual_Bit_Resolution";
pub const COMMANDED_BIT_RESOLUTION: &str = "Commanded_Bit_Resolution";
pub const COMMUNICATION_PROTOCOL: &str = "Communication_Protocol";
pub const GALVO_RANGE_OF_MOTION: &str = "Galvo_Range_of_Motion";
pub const SMOOTHING_KERNEL: &str = "Smoothing_Kernel";
pub const SMOOTHING_PARAMETERS: &str = "Smoothing_Parameters";
pub const TUNING_PARAMETERS: &str = "Tuning_Parameters";
pub const TUNING_TYPE: &str = "Tuning_Type";
/// What the March files call the tuning type: one value under two names.
pub const GALVO_TUNING_TYPE: &str = "Galvo_Tuning_Type";
/// The words of a galvo's protocol that are named, matched exactly.
pub const PROTOCOL_SL2_100: &str = "SL2-100";
pub const PROTOCOL_XY2_100: &str = "XY2-100";

// the attributes of the scanner card's group (its sample period is `SAMPLE_PERIOD`, above)
pub const ACTUAL_POSITION_TIMING_OFFSET: &str = "Actual_Position_Timing_Offset";
pub const COMMANDED_POSITION_TIMING_OFFSET: &str = "Commanded_Position_Timing_Offset";

// the attributes of the light source's group
pub const LASER_MODEL: &str = "Laser_Model";
pub const LASER_SERIAL_NUMBER: &str = "Laser_Serial_Number";
pub const LIGHT_WAVELENGTH: &str = "Light_Wavelength";
pub const LIGHT_WAVELENGTH_UNIT: &str = "Light_Wavelength_unit";
pub const POWER_BIT_RESOLUTION: &str = "Power_Bit_Resolution";
pub const POWER_MAX_ACTUAL: &str = "Power_Max_Actual";
pub const POWER_MAX_NOMINAL: &str = "Power_Max_Nominal";
pub const POWER_MIN_ACTUAL: &str = "Power_Min_Actual";
pub const POWER_MIN_NOMINAL: &str = "Power_Min_Nominal";
/// What the March files hold in place of the four limits above: text, in which the writer put the bytes of a number (decision D13-16). They are not read as numbers.
pub const POWER_MIN: &str = "Power_Min";
pub const POWER_MAX: &str = "Power_Max";
/// The algorithm of the volts-to-watts equation and its constants, written as text in the order of the algorithm.
pub const VOLTS_TO_WATTS_ALGORITHM: &str = "Volts_To_Watts_Algorithm";
pub const VOLTS_TO_WATTS_PARAMS: &str = "Volts_To_Watts_Params";
/// The algorithms that are named, matched exactly.
pub const ALGORITHM_LINEAR: &str = "LINEAR";
pub const ALGORITHM_POLYNOMIAL: &str = "POLYNOMIAL";

// ---------------------------------------------------------------- the OPC UA events (phase 13K)

/// The bucket of events that no layer owns, a child of the top-level events group (`GLOBAL_SENSORS`, beside the mount).
pub const UNASSIGNED: &str = "unassigned";
/// The events groups of an assignment, one for each kind of series.
pub const SENSOR_EVENTS: &str = "sensor_events";
pub const TASK_EVENTS: &str = "task_events";
pub const RUN_EVENTS: &str = "run_events";
/// The dataset of a series that holds the source timestamps: its length is the number of records, and a group that has it is a series.
pub const SOURCE_TS: &str = "Source_ts";
/// The values of a series, as text.
pub const EVENT_VALUE: &str = "Value";
/// The start and the stop of each record of a task.
pub const START_NS: &str = "Start_ns";
pub const STOP_NS: &str = "Stop_ns";
/// The type that a series declares for its values, an attribute of the series' group.
pub const EVENT_DATA_TYPE: &str = "DataType";
/// The height of a task, an attribute of the series' group.
pub const Z_HEIGHT: &str = "ZHeight";

// ---------------------------------------------------------------- the camera images of a layer (phase 13F)

/// The image group taken before a layer's exposure, a child of the layer's group.
pub const PRE_EXPOSURE: &str = "Pre_Exposure";
/// The image group taken after a layer's exposure, a child of the layer's group.
pub const POST_EXPOSURE: &str = "Post_Exposure";
/// The dataset of an image group that holds the frame.
pub const RAW: &str = "raw";

// ---------------------------------------------------------------- beside the mount (decision D13-19)

/// A group of V17 that sits next to the mount, not in it.
pub const CONFIGURATION: &str = "Configuration";
/// The OPC UA events that belong to no layer; also next to the mount.
pub const GLOBAL_SENSORS: &str = "Global_Sensors";
/// The groups beside the mount that V17 defines. Any other group beside the mount is not V17's and is not kept.
pub const BESIDE_THE_MOUNT: [&str; 2] = [CONFIGURATION, GLOBAL_SENSORS];

/// The attributes of the mount that the build level reads or consumes, the marker included. Every other attribute of the mount is kept in `extra`.
pub const TYPED_ON_THE_MOUNT: [&str; 11] = [
    MARKER,
    BUILD_ID,
    DATE,
    USERNAME,
    ALLOYS,
    NUMBER_OF_PARTS,
    LAYER_QUANTITY,
    LAYER_THICKNESS,
    BUILD_STATUS,
    OPCUA_JOIN_STATUS,
    OPCUA_JOIN_VERSION,
];
