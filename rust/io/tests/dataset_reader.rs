//! The dataset descriptor, compound and string readers (IP-03 phase 3E, tasks T03.3 to T03.5), held to the oracle `testdata/primitives.json` (phase 3A) and to
//! the eight miniatures.
//!
//! * `every_oracle_dataset_instance_is_read_as_the_oracle_says`: one dataset of every distinct kind is described, and its class, shape, maximum shape, layout,
//!   chunks, filters with their parameters, fill and allocation equal the oracle's.
//! * `every_dataset_of_every_miniature_is_described`: all 736 datasets are described, and each description agrees with itself.
//! * `compound_rows_equal_the_oracle_in_order` and `variable_length_strings_equal_the_oracle`: the two kinds that are not plain numbers.
//! * `the_light_description_of_a_dataset_equals_the_full_one_where_they_overlap`: the class, the shape and whether the dataspace is null, asked of every dataset of every miniature, are what the
//!   full description says (IP-07 phase 7E: a laser in a layer asks for these three only, which is a few calls into the file and not about eighteen).
//! * `a_dataset_with_a_null_dataspace_is_described_as_null_and_a_scalar_is_not`: HDF5 allows a type with no data and no dimension (IP-07 phase 7A, D07-2); a scalar has no dimension either and holds one value.
//!
//! The values of numeric datasets are read in blocks by phase 3F, which holds them to the first values of the oracle.

mod common;

use build_log_io::hdf5::{
    describe_dataset, describe_shape, has_null_dataspace, read_name_value_rows, read_strings,
    Allocation, DatasetDescriptor, DatasetLayout, Fill, FillWhen, Filter, FilterKind,
};
use build_log_model::ErrorKind;
use build_log_testkit::miniature_path;
use common::{class_of, datasets, oracle, MINIATURES};
use hdf5::File;
use serde_json::Value as Json;

fn sizes(j: &Json) -> Vec<usize> {
    j.as_array()
        .expect("a list")
        .iter()
        .map(|d| d.as_u64().expect("a size") as usize)
        .collect()
}

/// The descriptor that the oracle gives.
fn expected(d: &Json) -> DatasetDescriptor {
    DatasetDescriptor {
        class: class_of(&d["type"]),
        shape: sizes(&d["shape"]),
        // none of the datasets of the miniatures has a null dataspace (IP-07 phase 7A: a made file has one, below)
        null: false,
        max_shape: d["max_shape"]
            .as_array()
            .expect("the maximum shape")
            .iter()
            .map(|m| match m {
                Json::String(s) if s == "unlimited" => None,
                n => Some(n.as_u64().expect("a size") as usize),
            })
            .collect(),
        layout: match d["layout"].as_str().expect("a layout") {
            "compact" => DatasetLayout::Compact,
            "contiguous" => DatasetLayout::Contiguous,
            "chunked" => DatasetLayout::Chunked,
            "virtual" => DatasetLayout::Virtual,
            other => panic!("the oracle gives the layout {other}"),
        },
        chunks: match &d["chunks"] {
            Json::Null => None,
            c => Some(sizes(c)),
        },
        filters: d["filters"]
            .as_array()
            .expect("the filters")
            .iter()
            .map(|f| Filter {
                kind: match f["id"].as_str().expect("a filter") {
                    "deflate" => FilterKind::Deflate,
                    "shuffle" => FilterKind::Shuffle,
                    "fletcher32" => FilterKind::Fletcher32,
                    other => panic!("the oracle gives the filter {other}"),
                },
                parameters: f["parameters"]
                    .as_array()
                    .expect("parameters")
                    .iter()
                    .map(|p| p.as_u64().expect("a parameter") as u32)
                    .collect(),
            })
            .collect(),
        fill: match d["fill_value"].as_str().expect("a fill value") {
            "undefined" => Fill::Undefined,
            "default" => Fill::Default,
            "user_defined" => Fill::UserDefined,
            other => panic!("the oracle gives the fill value {other}"),
        },
        fill_when: match d["fill_time"].as_str().expect("a fill time") {
            "ifset" => FillWhen::IfSet,
            "alloc" => FillWhen::Alloc,
            "never" => FillWhen::Never,
            other => panic!("the oracle gives the fill time {other}"),
        },
        allocation: match d["allocation"].as_str().expect("an allocation") {
            "early" => Allocation::Early,
            "incremental" => Allocation::Incremental,
            "late" => Allocation::Late,
            other => panic!("the oracle gives the allocation {other}"),
        },
    }
}

fn open(instance: &Json) -> (String, hdf5::Dataset) {
    let file_name = instance["file"].as_str().expect("a file");
    let path = instance["path"].as_str().expect("a path");
    let file = File::open(miniature_path(file_name)).expect("a miniature opens");
    let dataset = file.dataset(path).expect("the dataset of the oracle");
    (format!("{file_name} {path}"), dataset)
}

#[test]
fn every_oracle_dataset_instance_is_read_as_the_oracle_says() {
    let oracle = oracle();
    let kinds = oracle["dataset_encodings"]
        .as_array()
        .expect("dataset kinds");
    assert!(
        kinds.len() >= 11,
        "the oracle holds a dataset of each kind of Appendix B"
    );
    let mut differences = Vec::new();
    for kind in kinds {
        let instance = &kind["instance"];
        let (label, dataset) = open(instance);
        match describe_dataset(&dataset) {
            Ok(got) => {
                let want = expected(&instance["descriptor"]);
                if got != want {
                    differences.push(format!("{label}:\n   read   {got:?}\n   oracle {want:?}"));
                }
            }
            Err(e) => differences.push(format!("{label}: {e}")),
        }
    }
    assert!(
        differences.is_empty(),
        "differences from the oracle:\n{}",
        differences.join("\n")
    );
}

#[test]
fn every_dataset_of_every_miniature_is_described() {
    let mut count = 0;
    let mut problems = Vec::new();
    for name in MINIATURES {
        let file = File::open(miniature_path(name)).expect("a miniature opens");
        for (path, dataset) in datasets(&file) {
            count += 1;
            match describe_dataset(&dataset) {
                Ok(d) => {
                    let rank_agrees =
                        d.shape.len() == d.max_shape.len() && d.shape.len() == dataset.ndim();
                    let chunks_agree = match (&d.layout, &d.chunks) {
                        (DatasetLayout::Chunked, Some(c)) => c.len() == d.shape.len(),
                        (DatasetLayout::Chunked, None) => false,
                        (_, chunks) => chunks.is_none(),
                    };
                    let within_max = d
                        .shape
                        .iter()
                        .zip(&d.max_shape)
                        .all(|(s, m)| m.is_none_or(|m| *s <= m));
                    if !(rank_agrees && chunks_agree && within_max) {
                        problems.push(format!("{name} {path}: {d:?} does not agree with itself"));
                    }
                }
                Err(e) => problems.push(format!("{name} {path}: {e}")),
            }
        }
    }
    assert!(
        problems.is_empty(),
        "datasets that were not described:\n{}",
        problems.join("\n")
    );
    // all of them were reached: the total of Appendix B
    assert_eq!(count, 736);
}

#[test]
fn compound_rows_equal_the_oracle_in_order() {
    let oracle = oracle();
    let mut compounds = 0;
    let mut kinds_of_name = std::collections::BTreeSet::new();
    for kind in oracle["dataset_encodings"]
        .as_array()
        .expect("dataset kinds")
    {
        if kind["kind"] != "compound, 1D" {
            continue;
        }
        compounds += 1;
        let instance = &kind["instance"];
        let (label, dataset) = open(instance);
        kinds_of_name.insert(
            instance["descriptor"]["type"]["fields"][0]["type"]["cset"]
                .as_str()
                .expect("a set")
                .to_string(),
        );
        let rows = read_name_value_rows(&dataset).unwrap_or_else(|e| panic!("{label}: {e}"));
        let want = instance["first_values"].as_array().expect("the rows");
        assert_eq!(rows.len(), want.len(), "{label}");
        for (row, want) in rows.iter().zip(want) {
            // the name is its bytes up to the first zero byte; the oracle holds all 64 stored bytes
            let stored = common::hex(want["name"]["bytes_hex"].as_str().expect("the bytes"));
            let content = stored.split(|b| *b == 0).next().unwrap_or(&[]).to_vec();
            assert_eq!(row.name, content, "{label}");
            assert_eq!(Some(row.value), want["value"].as_f64(), "{label}");
        }
    }
    // both compounds of V18 (the laser calibration, UTF-8; the sensor calibration, ASCII) are read
    assert!(compounds >= 2);
    assert_eq!(
        kinds_of_name.into_iter().collect::<Vec<_>>(),
        ["ascii", "utf-8"]
    );
}

#[test]
fn variable_length_strings_equal_the_oracle() {
    let oracle = oracle();
    let mut checked = 0;
    for kind in oracle["dataset_encodings"]
        .as_array()
        .expect("dataset kinds")
    {
        if kind["kind"] != "string, variable length, 1D" {
            continue;
        }
        checked += 1;
        let instance = &kind["instance"];
        let (label, dataset) = open(instance);
        let first = instance["first_values"]
            .as_array()
            .expect("the first values");
        let want: Vec<&str> = first
            .iter()
            .map(|v| v["text"].as_str().expect("text"))
            .collect();
        let got = read_strings(&dataset, 0..want.len()).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(got, want, "{label}");
        // the whole dataset has the length of its shape, and a range in the middle is the same values
        let length = dataset.shape()[0];
        assert_eq!(
            read_strings(&dataset, 0..length).expect("all of it").len(),
            length,
            "{label}"
        );
        assert_eq!(
            read_strings(&dataset, 1..want.len()).expect("a range"),
            want[1..],
            "{label}"
        );
        assert!(read_strings(&dataset, 3..3)
            .expect("an empty range")
            .is_empty());
        // a range outside the dataset is an index error, never a short read
        let err = read_strings(&dataset, 0..length + 1).expect_err("one value too many");
        assert!(
            matches!(err.kind(), ErrorKind::InvalidIndex { .. }),
            "{err}"
        );
    }
    assert!(
        checked >= 2,
        "the oracle holds the OPC UA values of V17 and of V18"
    );
}

#[test]
fn a_dataset_that_is_not_what_a_reader_reads_is_named_not_guessed() {
    let file =
        File::open(miniature_path(build_log_testkit::miniature::V18)).expect("the V18 miniature");
    // a dataset of numbers is neither a compound nor strings
    let (_, numbers) = datasets(&file)
        .into_iter()
        .find(|(_, d)| {
            d.dtype()
                .ok()
                .and_then(|t| t.to_descriptor().ok())
                .is_some_and(|t| matches!(t, hdf5::types::TypeDescriptor::Integer(_)))
        })
        .expect("a dataset of integers");
    for err in [
        read_name_value_rows(&numbers).expect_err("not a compound"),
        read_strings(&numbers, 0..1).expect_err("not strings"),
    ] {
        assert!(
            matches!(err.kind(), ErrorKind::UnsupportedInVersion { .. }),
            "{err}"
        );
    }
}

#[test]
fn a_dataset_with_a_null_dataspace_is_described_as_null_and_a_scalar_is_not() {
    // a dataset of each kind of dataspace, made at test time: null (a type, no data, no dimension), scalar (no dimension, one value), one-dimensional with values, and one-dimensional with none
    let path =
        std::env::temp_dir().join(format!("build-log-io-null-space-{}.h5", std::process::id()));
    {
        let file = File::create(&path).expect("a file");
        file.new_dataset_builder()
            .empty::<f32>()
            .shape(hdf5::Extents::Null)
            .create("null")
            .expect("a null dataspace");
        file.new_dataset::<f32>()
            .create("scalar")
            .expect("a scalar");
        file.new_dataset::<f32>()
            .shape(3)
            .create("three")
            .expect("a dataset of three");
        file.new_dataset::<f32>()
            .shape(0)
            .create("zero")
            .expect("a dataset of none");
    }
    let file = File::open(&path).expect("it opens");
    let described = |name: &str| {
        let dataset = file.dataset(name).expect("a dataset");
        let descriptor = describe_dataset(&dataset).expect("it is described");
        // the primitive that asks the one question says the same as the descriptor
        assert_eq!(
            has_null_dataspace(&dataset).expect("the question"),
            descriptor.null,
            "{name}"
        );
        descriptor
    };
    let null = described("null");
    assert!(null.null && null.shape.is_empty(), "{null:?}");
    // the type is there, though no value is
    assert!(
        matches!(
            null.class,
            build_log_io::hdf5::Class::Float { bits: 32, .. }
        ),
        "{:?}",
        null.class
    );
    // a scalar has no dimension too, and is not null; a dataset of three or of none is neither
    let scalar = described("scalar");
    assert!(!scalar.null && scalar.shape.is_empty(), "{scalar:?}");
    let three = described("three");
    assert!(!three.null && three.shape == [3], "{three:?}");
    let zero = described("zero");
    assert!(!zero.null && zero.shape == [0], "{zero:?}");
    drop(file);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_light_description_of_a_dataset_equals_the_full_one_where_they_overlap() {
    // every dataset of the eight miniatures (736 of them)
    let mut checked = 0;
    for name in MINIATURES {
        let file = File::open(miniature_path(name)).expect("a miniature opens");
        for (path, dataset) in datasets(&file) {
            let full = describe_dataset(&dataset).expect("the full description");
            let light = describe_shape(&dataset).expect("the light description");
            assert_eq!(light.class, full.class, "{name} {path}");
            assert_eq!(light.shape, full.shape, "{name} {path}");
            assert_eq!(light.null, full.null, "{name} {path}");
            checked += 1;
        }
    }
    assert_eq!(checked, 736);
    // the kinds of dataspace that no miniature holds, made at test time: null, scalar, and one-dimensional with none
    let path = std::env::temp_dir().join(format!(
        "build-log-io-light-space-{}.h5",
        std::process::id()
    ));
    {
        let file = File::create(&path).expect("a file");
        file.new_dataset_builder()
            .empty::<f32>()
            .shape(hdf5::Extents::Null)
            .create("null")
            .expect("a null dataspace");
        file.new_dataset::<u8>().create("scalar").expect("a scalar");
        file.new_dataset::<i16>()
            .shape(0)
            .create("zero")
            .expect("a dataset of none");
        file.new_dataset::<f64>()
            .shape([2, 3])
            .create("table")
            .expect("a table");
    }
    let file = File::open(&path).expect("it opens");
    for name in ["null", "scalar", "zero", "table"] {
        let dataset = file.dataset(name).expect("a dataset");
        let full = describe_dataset(&dataset).expect("the full description");
        let light = describe_shape(&dataset).expect("the light description");
        assert_eq!(
            (light.class, light.shape, light.null),
            (full.class, full.shape, full.null),
            "{name}"
        );
    }
    drop(file);
    let _ = std::fs::remove_file(&path);
}
