//! The attribute reader (IP-03 phase 3D, task T03.2), held to the oracle `testdata/primitives.json` (phase 3A, written by `h5py`) and to the eight miniatures.
//!
//! * `every_oracle_attribute_instance_is_read_as_the_oracle_says`: one instance of every distinct encoding is read, and its class, shape and value equal the oracle's.
//! * `every_attribute_of_every_miniature_reads_and_keeps_its_encoding`: all 1,614 attributes are read, and each value agrees with its own encoding (the
//!   variant, the number of elements, the length of a fixed string). It needs no oracle.
//! * the awkward kinds, the raw-bytes string, `File_Version` and a missing attribute have a test each.

mod common;

use build_log_io::hdf5::{
    read_attribute, strict_text, ByteOrder, Charset, Class, Encoding, Padding, Shape, Stored,
    TextForm, TextLength, Value,
};
use build_log_model::ErrorKind;
use build_log_testkit::{miniature, miniature_path};
use common::{class_of, flatten, hex, int_of, locate, locations, oracle, shape_of, MINIATURES};
use hdf5::types::{FixedAscii, FixedUnicode, VarLenUnicode};
use hdf5::{File, H5Type};
use serde_json::Value as Json;

/// What differs between a value that was read and the oracle's value (empty when they agree). A fixed string is its bytes up to the first zero byte.
fn value_differences(expected: &Json, stored: &Stored) -> Vec<String> {
    let mut flat = Vec::new();
    flatten(expected, &mut flat);
    let mut found = Vec::new();
    macro_rules! ints {
        ($v:expr) => {{
            let got: Vec<i128> = $v.iter().map(|x| i128::from(*x)).collect();
            let want: Vec<i128> = flat.iter().map(int_of).collect();
            if got != want {
                found.push(format!("integers {got:?}, the oracle {want:?}"));
            }
        }};
    }
    macro_rules! floats {
        ($v:expr) => {{
            let got: Vec<f64> = $v.iter().map(|x| f64::from(*x)).collect();
            let want: Vec<f64> = flat.iter().map(|j| j.as_f64().expect("a float")).collect();
            if got != want {
                found.push(format!("floats {got:?}, the oracle {want:?}"));
            }
        }};
    }
    match &stored.value {
        Value::I8(v) => ints!(v),
        Value::I16(v) => ints!(v),
        Value::I32(v) => ints!(v),
        Value::I64(v) => ints!(v),
        Value::U8(v) => ints!(v),
        Value::U16(v) => ints!(v),
        Value::U32(v) => ints!(v),
        Value::U64(v) => ints!(v),
        Value::F32(v) => floats!(v),
        Value::F64(v) => floats!(v),
        Value::Bytes(v) => {
            let want: Vec<Vec<u8>> = flat
                .iter()
                .map(|j| {
                    let stored = hex(j["bytes_hex"].as_str().expect("the bytes"));
                    stored.split(|b| *b == 0).next().unwrap_or(&[]).to_vec()
                })
                .collect();
            if *v != want {
                found.push(format!("bytes {v:02x?}, the oracle {want:02x?}"));
            }
        }
        Value::Text(v) => {
            let want: Vec<String> = flat
                .iter()
                .map(|j| j["text"].as_str().expect("text").to_string())
                .collect();
            if *v != want {
                found.push(format!("text {v:?}, the oracle {want:?}"));
            }
        }
        other => found.push(format!("a kind of value the test does not know: {other:?}")),
    }
    found
}

/// Reads an oracle instance and says what differs from the oracle.
fn instance_differences(encoding: &Json, instance: &Json) -> Vec<String> {
    let file_name = instance["file"].as_str().expect("a file");
    let path = instance["path"].as_str().expect("a path");
    let name = instance["name"].as_str().expect("a name");
    let file = File::open(miniature_path(file_name)).expect("a miniature opens");
    let stored = match read_attribute(&locate(&file, path), name) {
        Ok(stored) => stored,
        Err(e) => return vec![format!("{file_name} {path} `{name}`: {e}")],
    };
    let mut found = Vec::new();
    let want = Encoding {
        class: class_of(&encoding["type"]),
        shape: shape_of(&encoding["space"]),
    };
    if stored.encoding != want {
        found.push(format!(
            "encoding {:?}, the oracle {want:?}",
            stored.encoding
        ));
    }
    found.extend(value_differences(&instance["value"], &stored));
    found
        .into_iter()
        .map(|d| format!("{file_name} {path} `{name}`: {d}"))
        .collect()
}

#[test]
fn every_oracle_attribute_instance_is_read_as_the_oracle_says() {
    let oracle = oracle();
    let mut instances = 0;
    let mut differences = Vec::new();
    for e in oracle["attribute_encodings"].as_array().expect("encodings") {
        for instance in e["instances"].as_array().expect("instances") {
            instances += 1;
            differences.extend(instance_differences(&e["encoding"], instance));
        }
    }
    assert!(
        instances >= 14,
        "the oracle holds at least one instance of each kind of Appendix B"
    );
    assert!(
        differences.is_empty(),
        "differences from the oracle:\n{}",
        differences.join("\n")
    );
}

/// Does a value agree with its own encoding: the variant for the class, the number of elements for the shape, and the length of a fixed string?
fn consistent(s: &Stored) -> Result<(), String> {
    let (leaf, per_element) = match &s.encoding.class {
        Class::Array { dims, base } => (&**base, dims.iter().product::<usize>()),
        other => (other, 1),
    };
    let elements = match &s.encoding.shape {
        Shape::Scalar => 1,
        Shape::Array(dims) => dims.iter().product::<usize>(),
        other => return Err(format!("a shape the test does not know: {other:?}")),
    };
    let wanted = elements * per_element;
    let (got, variant_fits) = match (&s.value, leaf) {
        (
            Value::I8(v),
            Class::Integer {
                bits: 8,
                signed: true,
                ..
            },
        ) => (v.len(), true),
        (
            Value::I16(v),
            Class::Integer {
                bits: 16,
                signed: true,
                ..
            },
        ) => (v.len(), true),
        (
            Value::I32(v),
            Class::Integer {
                bits: 32,
                signed: true,
                ..
            },
        ) => (v.len(), true),
        (
            Value::I64(v),
            Class::Integer {
                bits: 64,
                signed: true,
                ..
            },
        ) => (v.len(), true),
        (
            Value::U8(v),
            Class::Integer {
                bits: 8,
                signed: false,
                ..
            },
        ) => (v.len(), true),
        (
            Value::U16(v),
            Class::Integer {
                bits: 16,
                signed: false,
                ..
            },
        ) => (v.len(), true),
        (
            Value::U32(v),
            Class::Integer {
                bits: 32,
                signed: false,
                ..
            },
        ) => (v.len(), true),
        (
            Value::U64(v),
            Class::Integer {
                bits: 64,
                signed: false,
                ..
            },
        ) => (v.len(), true),
        (Value::F32(v), Class::Float { bits: 32, .. }) => (v.len(), true),
        (Value::F64(v), Class::Float { bits: 64, .. }) => (v.len(), true),
        (
            Value::Text(v),
            Class::Text(TextForm {
                length: TextLength::Variable,
                ..
            }),
        ) => (v.len(), true),
        (
            Value::Bytes(v),
            Class::Text(TextForm {
                length: TextLength::Fixed { bytes, .. },
                ..
            }),
        ) => (v.len(), v.iter().all(|b| b.len() <= *bytes)),
        (value, class) => {
            return Err(format!(
                "the value {value:?} does not fit the class {class:?}"
            ))
        }
    };
    if !variant_fits {
        return Err("a fixed string is longer than its length".to_string());
    }
    if got != wanted {
        return Err(format!(
            "{got} elements, the shape {:?} says {wanted}",
            s.encoding.shape
        ));
    }
    Ok(())
}

#[test]
fn every_attribute_of_every_miniature_reads_and_keeps_its_encoding() {
    let mut attributes = 0;
    let mut problems = Vec::new();
    for name in MINIATURES {
        let file = File::open(miniature_path(name)).expect("a miniature opens");
        for (path, location) in locations(&file) {
            for attr_name in location.attr_names().expect("attribute names") {
                attributes += 1;
                match read_attribute(&location, &attr_name) {
                    Ok(stored) => {
                        if let Err(why) = consistent(&stored) {
                            problems.push(format!("{name} {path} `{attr_name}`: {why}"));
                        }
                    }
                    Err(e) => problems.push(format!("{name} {path} `{attr_name}`: {e}")),
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "attributes that were not read as their encoding says:\n{}",
        problems.join("\n")
    );
    // all of them were reached: the total of Appendix B
    assert_eq!(attributes, 1_614);
}

#[test]
fn a_raw_bytes_string_attribute_is_returned_as_raw_bytes_not_text() {
    // V17 March keeps the eight bytes of a float64 in a 9-byte null-terminated string
    let file = File::open(miniature_path(miniature::V17_MARCH)).expect("the March miniature");
    let read = |train: &str| {
        let group = locate(
            &file,
            &format!("/Built_Part/Calibration_Data/{train}/Light_Source"),
        );
        read_attribute(&group, "Power_Min").expect("Power_Min is read")
    };
    let first = read("Optical_Train_1");
    assert_eq!(
        first.encoding,
        Encoding {
            class: Class::Text(TextForm {
                charset: Charset::Ascii,
                length: TextLength::Fixed {
                    bytes: 9,
                    padding: Padding::Unknown
                },
            }),
            shape: Shape::Scalar,
        }
    );
    // the bytes of 101.7, not decoded: they are not UTF-8, so there is no text view
    let Value::Bytes(elements) = &first.value else {
        panic!("a fixed string is bytes, not text")
    };
    assert_eq!(
        elements,
        &vec![vec![0xcd, 0xcc, 0xcc, 0xcc, 0xcc, 0x6c, 0x59, 0x40]]
    );
    assert_eq!(
        f64::from_le_bytes(elements[0].clone().try_into().expect("eight bytes")),
        101.7
    );
    assert!(strict_text(&elements[0]).is_none());
    // the bytes of 100.6 happen to be ASCII (`fffff&Y@`): the view shows it, and it is still not what the value is; only an adapter knows
    let second = read("Optical_Train_2");
    let Value::Bytes(elements) = &second.value else {
        panic!("bytes")
    };
    assert_eq!(
        f64::from_le_bytes(elements[0].clone().try_into().expect("eight bytes")),
        100.6
    );
    assert_eq!(strict_text(&elements[0]), Some("fffff&Y@"));
}

#[test]
fn file_version_reads_as_a_scalar_or_an_array_of_one_with_its_encoding_kept() {
    let oracle = oracle();
    let marker = |name: &str| {
        let file = File::open(miniature_path(name)).expect("a miniature opens");
        let top = if name == miniature::V18 {
            "/Build"
        } else {
            "/Built_Part"
        };
        read_attribute(&locate(&file, top), "File_Version").expect("the marker is read")
    };
    let int32 = |shape| Encoding {
        class: Class::Integer {
            bits: 32,
            signed: true,
            order: ByteOrder::Little,
        },
        shape,
    };
    // a scalar in some files, an array of one in the others, and the integer itself in both
    assert_eq!(
        marker(miniature::V16),
        Stored {
            encoding: int32(Shape::Scalar),
            value: Value::I32(vec![16])
        }
    );
    assert_eq!(
        marker(miniature::V18),
        Stored {
            encoding: int32(Shape::Scalar),
            value: Value::I32(vec![18])
        }
    );
    assert_eq!(
        marker(miniature::V17_MARCH),
        Stored {
            encoding: int32(Shape::Scalar),
            value: Value::I32(vec![17])
        }
    );
    assert_eq!(
        marker(miniature::V17_TRIGGERED_2026_10_19),
        Stored {
            encoding: int32(Shape::Array(vec![1])),
            value: Value::I32(vec![17])
        }
    );
    // and every file of the oracle, as it says
    for entry in oracle["file_versions"].as_array().expect("the markers") {
        let name = entry["file"].as_str().expect("a file");
        let read = marker(name);
        assert_eq!(
            read.encoding.shape,
            shape_of(&entry["encoding"]["space"]),
            "{name}"
        );
        assert!(
            value_differences(&entry["value"], &read).is_empty(),
            "{name}"
        );
    }
}

#[test]
fn a_missing_attribute_is_not_present() {
    let file = File::open(miniature_path(miniature::V16)).expect("the V16 miniature");
    let err = read_attribute(&locate(&file, "/Built_Part"), "No_Such_Attribute")
        .expect_err("there is no such attribute");
    assert!(matches!(err.kind(), ErrorKind::NotPresent { .. }), "{err}");
}

/// The compound of V18's laser calibration: a UTF-8 name of 64 bytes and a `float64`.
#[derive(H5Type, Clone, Debug)]
#[repr(C)]
struct EquationConstant {
    name: FixedUnicode<64>,
    value: f64,
}

/// The compound of V18's sensor calibration: the same, but its name is ASCII. The binding converts neither into the other, so a reader of compound
/// datasets (phase 3E) has to choose the type from the descriptor of the dataset.
#[derive(H5Type, Clone, Debug)]
#[repr(C)]
struct EquationConstantAscii {
    name: FixedAscii<64>,
    value: f64,
}

#[test]
fn the_awkward_kinds_read_as_the_oracle_says_by_the_way_d03_4_records() {
    // D03-4 records the way: the typed API of the binding for every kind of the miniatures (the method check, `tests/method_check.rs`)
    let oracle = oracle();
    // the attribute kinds of probes 3 to 5: fixed-length strings of every length and filling, and the one element that is an array of 64 bytes
    let mut seen_kinds = std::collections::BTreeSet::new();
    let mut differences = Vec::new();
    for e in oracle["attribute_encodings"].as_array().expect("encodings") {
        let class = e["encoding"]["type"]["class"].as_str().expect("a class");
        let fixed = class == "string" && e["encoding"]["type"]["length"] != "variable";
        if fixed || class == "array" {
            seen_kinds.insert(e["kind"].as_str().expect("a kind").to_string());
            for instance in e["instances"].as_array().expect("instances") {
                differences.extend(instance_differences(&e["encoding"], instance));
            }
        }
    }
    assert!(seen_kinds.contains("string, fixed length, ascii, null-terminated"));
    assert!(seen_kinds.contains("string, fixed length, ascii, null-padded"));
    assert!(seen_kinds.contains("array of 64 uint8, array of one"));
    assert!(
        differences.is_empty(),
        "differences from the oracle:\n{}",
        differences.join("\n")
    );
    // the dataset kinds of probes 6 and 7: the compound, and the variable-length strings
    let mut datasets = 0;
    for e in oracle["dataset_encodings"]
        .as_array()
        .expect("dataset kinds")
    {
        let kind = e["kind"].as_str().expect("a kind");
        let instance = &e["instance"];
        let file = File::open(miniature_path(instance["file"].as_str().expect("a file")))
            .expect("a miniature opens");
        let dataset = file
            .dataset(instance["path"].as_str().expect("a path"))
            .expect("the dataset");
        let first = instance["first_values"].as_array().expect("first values");
        if kind == "compound, 1D" {
            datasets += 1;
            let ascii = instance["descriptor"]["type"]["fields"][0]["type"]["cset"] == "ascii";
            let rows: Vec<(String, f64)> = if ascii {
                let rows = dataset
                    .read_raw::<EquationConstantAscii>()
                    .expect("a typed read");
                rows.iter()
                    .map(|r| (r.name.as_str().to_string(), r.value))
                    .collect()
            } else {
                let rows = dataset
                    .read_raw::<EquationConstant>()
                    .expect("a typed read");
                rows.iter()
                    .map(|r| (r.name.as_str().to_string(), r.value))
                    .collect()
            };
            assert_eq!(rows.len(), first.len());
            for ((name, value), want) in rows.iter().zip(first) {
                assert_eq!(Some(name.as_str()), want["name"]["text"].as_str());
                assert_eq!(Some(*value), want["value"].as_f64());
            }
        } else if kind == "string, variable length, 1D" {
            datasets += 1;
            let values = dataset.read_raw::<VarLenUnicode>().expect("a typed read");
            for (value, want) in values.iter().zip(first) {
                assert_eq!(Some(value.as_str()), want["text"].as_str());
            }
            assert!(values.len() >= first.len());
        }
    }
    assert!(
        datasets >= 3,
        "the oracle holds compound datasets and string datasets"
    );
}
