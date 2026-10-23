//! Helpers that the integration tests of the primitives share: the eight miniatures, and a walk over every object of a file that can hold attributes.

#![allow(dead_code)]

use build_log_io::hdf5::{ByteOrder, Charset, Class, Field, Padding, Shape, TextForm, TextLength};
use build_log_testkit::{miniature, testdata_dir};
use hdf5::{File, Location};
use serde_json::Value as Json;

/// The eight committed miniatures, by name.
pub const MINIATURES: [&str; 8] = [
    miniature::V16,
    miniature::V17_MARCH,
    miniature::V17_SPECIAL_LAYERS,
    miniature::V17_MOCK,
    miniature::V17_OCT15,
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
    miniature::V18,
];

/// Every object of a file that can hold attributes (the root, every group and every dataset), with its path, in the order of a walk.
pub fn locations(file: &File) -> Vec<(String, Location)> {
    let mut found: Vec<(String, Location)> = Vec::new();
    let mut stack = vec![file.group("/").expect("the root group")];
    while let Some(group) = stack.pop() {
        found.push((group.name(), (*group).clone()));
        for name in group.member_names().expect("the members of a group") {
            if let Ok(sub) = group.group(&name) {
                stack.push(sub);
            } else if let Ok(dataset) = group.dataset(&name) {
                found.push((dataset.name(), (**dataset).clone()));
            }
        }
    }
    found
}

/// Every dataset of a file, with its path.
pub fn datasets(file: &File) -> Vec<(String, hdf5::Dataset)> {
    let mut found = Vec::new();
    let mut stack = vec![file.group("/").expect("the root group")];
    while let Some(group) = stack.pop() {
        for name in group.member_names().expect("the members of a group") {
            if let Ok(sub) = group.group(&name) {
                stack.push(sub);
            } else if let Ok(dataset) = group.dataset(&name) {
                found.push((dataset.name(), dataset));
            }
        }
    }
    found
}

/// The group or dataset at a path.
pub fn locate(file: &File, path: &str) -> Location {
    match file.group(path) {
        Ok(group) => (*group).clone(),
        Err(_) => (**file
            .dataset(path)
            .unwrap_or_else(|e| panic!("`{path}` is neither a group nor a dataset: {e}")))
        .clone(),
    }
}

// ---------------------------------------------------------------- small HDF5 files made at test time (IP-04)

/// A temporary HDF5 file, removed when the value is dropped, even if the test fails.
pub struct TempH5(pub std::path::PathBuf);

impl Drop for TempH5 {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A temporary file with one group, `Mount`, that `write` fills. The label must be different in each test, because tests run side by side.
pub fn temp_h5(label: &str, write: impl FnOnce(&hdf5::Group)) -> TempH5 {
    let path = std::env::temp_dir().join(format!("build-log-io-{}-{label}.h5", std::process::id()));
    {
        let file = File::create(&path).expect("a new HDF5 file can be created");
        let group = file.create_group("Mount").expect("a group can be created");
        write(&group);
    }
    TempH5(path)
}

/// A temporary file that `write` fills, with whatever groups it likes. The label must be different in each test.
pub fn temp_file(label: &str, write: impl FnOnce(&File)) -> TempH5 {
    let path = std::env::temp_dir().join(format!("build-log-io-{}-{label}.h5", std::process::id()));
    {
        let file = File::create(&path).expect("a new HDF5 file can be created");
        write(&file);
    }
    TempH5(path)
}

pub fn put_scalar<T: hdf5::H5Type>(group: &hdf5::Group, name: &str, value: T) {
    group
        .new_attr::<T>()
        .create(name)
        .expect("an attribute can be created")
        .write_scalar(&value)
        .expect("it can be written");
}

pub fn put_array<T: hdf5::H5Type>(group: &hdf5::Group, name: &str, values: &[T]) {
    group
        .new_attr::<T>()
        .shape(values.len())
        .create(name)
        .expect("an attribute can be created")
        .write_raw(values)
        .expect("it can be written");
}

pub fn put_text(group: &hdf5::Group, name: &str, text: &str) {
    put_scalar(
        group,
        name,
        text.parse::<hdf5::types::VarLenUnicode>().expect("text"),
    );
}

// ---------------------------------------------------------------- the oracle, `testdata/primitives.json` (phase 3A)

pub fn oracle() -> Json {
    let text = std::fs::read_to_string(testdata_dir().join("primitives.json"))
        .expect("testdata/primitives.json (phase 3A)");
    serde_json::from_str(&text).expect("the oracle is JSON")
}

pub fn flatten(value: &Json, out: &mut Vec<Json>) {
    match value {
        Json::Array(items) => items.iter().for_each(|i| flatten(i, out)),
        other => out.push(other.clone()),
    }
}

pub fn hex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("hex"))
        .collect()
}

pub fn order_of(t: &Json) -> ByteOrder {
    match t["order"].as_str().expect("an order") {
        "little" => ByteOrder::Little,
        "big" => ByteOrder::Big,
        other => panic!("the oracle gives the byte order {other}"),
    }
}

/// The class the oracle describes. A string that is not null-padded is `Unknown`, as the safe binding cannot say more (see `Padding::Unknown`).
pub fn class_of(t: &Json) -> Class {
    match t["class"].as_str().expect("a class") {
        "integer" => Class::Integer {
            bits: t["bits"].as_u64().expect("bits") as u8,
            signed: t["signed"].as_bool().expect("signed"),
            order: order_of(t),
        },
        "float" => Class::Float {
            bits: t["bits"].as_u64().expect("bits") as u8,
            order: order_of(t),
        },
        "string" => {
            let charset = match t["cset"].as_str().expect("a character set") {
                "ascii" => Charset::Ascii,
                "utf-8" => Charset::Utf8,
                other => panic!("the oracle gives the character set {other}"),
            };
            let length = match &t["length"] {
                Json::String(s) if s == "variable" => TextLength::Variable,
                n => TextLength::Fixed {
                    bytes: n.as_u64().expect("a length") as usize,
                    padding: if t["pad"] == "nullpad" {
                        Padding::NullPadded
                    } else {
                        Padding::Unknown
                    },
                },
            };
            Class::Text(TextForm { charset, length })
        }
        "array" => Class::Array {
            dims: t["dims"]
                .as_array()
                .expect("dims")
                .iter()
                .map(|d| d.as_u64().expect("a dimension") as usize)
                .collect(),
            base: Box::new(class_of(&t["base"])),
        },
        "compound" => Class::Compound {
            fields: t["fields"]
                .as_array()
                .expect("fields")
                .iter()
                .map(|f| Field {
                    name: f["name"].as_str().expect("a name").to_string(),
                    class: class_of(&f["type"]),
                })
                .collect(),
        },
        other => panic!("the oracle gives the class {other}"),
    }
}

pub fn shape_of(space: &Json) -> Shape {
    match space["kind"].as_str().expect("a kind of space") {
        "scalar" => Shape::Scalar,
        "array" => Shape::Array(
            space["shape"]
                .as_array()
                .expect("a shape")
                .iter()
                .map(|d| d.as_u64().expect("a dimension") as usize)
                .collect(),
        ),
        other => panic!("the oracle gives the space {other}"),
    }
}

pub fn int_of(j: &Json) -> i128 {
    j.as_i64()
        .map(i128::from)
        .or_else(|| j.as_u64().map(i128::from))
        .unwrap_or_else(|| panic!("{j} is not an integer"))
}

// ---------------------------------------------------------------- V17 payloads made at test time (IP-13)

use build_log_io::adapters::v17::layout;
use hdf5::Group;

pub fn monitoring(mount: &Group) -> Group {
    mount
        .create_group(layout::MONITORING_DATA)
        .expect("the layers' group")
}

pub fn layer(monitoring: &Group, id: u32) -> Group {
    monitoring
        .create_group(&format!("{}{id}", layout::LAYER_PREFIX))
        .expect("a layer's group")
}

pub fn dataset<T: hdf5::H5Type>(group: &Group, name: &str, samples: usize) {
    group
        .new_dataset::<T>()
        .shape(samples)
        .create(name)
        .expect("a dataset");
}

/// A laser in a layer with all eight channels of `samples` samples each: the power as `float32`, the state as `uint8`, the positions as `int32`.
pub fn laser(layer: &Group, number: u32, samples: usize, window: Option<(i64, i64)>) -> Group {
    let group = layer
        .create_group(&format!("{}{number}", layout::LASER_PREFIX))
        .expect("a laser's group");
    if let Some((start, end)) = window {
        put_array(&group, layout::START_LASER_TIME, &[start]);
        put_array(&group, layout::END_LASER_TIME, &[end]);
    }
    let data = group
        .create_group(layout::LASER_DATA)
        .expect("the group of the power");
    dataset::<f32>(&data, layout::LASER_POWER, samples);
    dataset::<u8>(&data, layout::LASER_STATE, samples);
    let position = group
        .create_group(layout::POSITION)
        .expect("the group of the positions");
    for name in [
        layout::ACTUAL_X,
        layout::ACTUAL_Y,
        layout::ACTUAL_Z,
        layout::COMMANDED_X,
        layout::COMMANDED_Y,
        layout::COMMANDED_Z,
    ] {
        dataset::<i32>(&position, name, samples);
    }
    group
}

/// A scanner card with a sample period, in the optical train `train`.
pub fn scanner_card(mount: &Group, train: u32, period: f32) {
    let calibration = mount
        .create_group(layout::CALIBRATION_DATA)
        .or_else(|_| mount.group(layout::CALIBRATION_DATA))
        .expect("the calibration group");
    let train = calibration
        .create_group(&format!("{}{train}", layout::OPTICAL_TRAIN_PREFIX))
        .expect("an optical train");
    let card = train
        .create_group(layout::SCANNER_CARD)
        .expect("a scanner card");
    put_array(&card, layout::SAMPLE_PERIOD, &[period]);
}
