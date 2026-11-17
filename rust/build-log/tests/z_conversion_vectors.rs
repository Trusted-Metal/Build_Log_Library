//! The Z calibration and conversion against the cases of Vision's vectors that are on the library's miniatures (IP-27 phase 27D; decisions D27-6, D27-14, D27-15 and D27-18; BL-PR-03).
//!
//! `testdata/vectors/z_conversion.json` was generated in Vision from its own code at the commit it names. Each **file** case names one of the miniatures, a layer by its position, a
//! selection and optional overrides of the Z calibration; Vision recorded, for each laser it keeps, the count and **a digest of its converted Z series (commanded and actual)** and their first and last
//! values. Here each is run **through the library**: the file is opened, the rows of each laser are chosen (27C), the calibration that the file and the user give is resolved, Vision's defaults and rules are applied to it
//! as its adapter will, the Z codes are read for those rows and converted by the library, and everything is compared **bit for bit**. The 24 pure cases are run by `rust/processing/tests/z_conversion_vectors.rs`; this file is
//! here because it needs the IO layer (a file to open) and the processing layer together.
//!
//! **The decisions are Vision's and are made in `vision_adapter`, as the adapter of phase 27K will make them** (D27-18): Vision's defaults and its narrower-dataset rule (the library reports a missing value as
//! missing); which lasers the raw tier reports (a laser is in the answer when a stream that was asked for has its X and Y rows, and `n` is the shortest of them); that a laser whose Z cannot be converted is
//! dropped whole, X and Y included, the commanded series being checked first; and that a Z series that is shorter than `n` is zeros. The library reports each of those facts and decides none of them.

mod vision_adapter;

use build_log::{ReadSelection, ZCalibrationOverride};
use build_log_conformance::vectors::{
    bits, differences, digest_f64, expected_columns, head_and_tail, load_vectors, opt_f64, opt_i64,
    whole, words, Columns,
};
use serde_json::Value;

use vision_adapter::{kept_layer, rows_of};

const CHUNK: &str = "z_conversion";

fn override_of(value: &Value) -> ZCalibrationOverride {
    let mut o = ZCalibrationOverride::new();
    if let Some(v) = opt_f64(&value["field_size_z"]) {
        o = o.with_field_size_z_mm(v);
    }
    if let Some(v) = opt_i64(&value["commanded_bit_res_z"]) {
        o = o.with_commanded_bit_resolution_z(v);
    }
    if let Some(v) = opt_i64(&value["actual_bit_res_z"]) {
        o = o.with_actual_bit_resolution_z(v);
    }
    o
}

fn run_file(inputs: &Value) -> Columns {
    let selection = ReadSelection::new()
        .with_stride(inputs["stride"].as_u64().expect("stride"))
        .with_rows(rows_of(inputs["rows"].as_str().expect("rows")));
    let layer = kept_layer(
        inputs["miniature"].as_str().expect("miniature"),
        usize::try_from(inputs["layer_position"].as_u64().expect("layer_position"))
            .expect("layer_position"),
        &selection,
        &override_of(&inputs["override"]),
    );

    let mut out = Columns::new();
    out.insert("layer_id".into(), words(&[&layer.layer_id]));
    let mut present: Vec<i64> = Vec::new();
    for laser in &layer.lasers {
        present.push(laser.index as i64);
        let p = format!("laser{}", laser.index);
        out.insert(format!("{p}_n"), whole(&[laser.n as i64]));
        out.insert(
            format!("{p}_cmd_z_digest"),
            whole(&[digest_f64(&laser.commanded_mm)]),
        );
        out.insert(
            format!("{p}_act_z_digest"),
            whole(&[digest_f64(&laser.actual_mm)]),
        );
        for (column, values) in [("cmd_z", &laser.commanded_mm), ("act_z", &laser.actual_mm)] {
            let (head, tail) = head_and_tail(values);
            out.insert(format!("{p}_{column}_head"), bits(&head));
            out.insert(format!("{p}_{column}_tail"), bits(&tail));
        }
    }
    out.insert("lasers_present".into(), whole(&present));
    out
}

#[test]
fn the_cases_on_the_miniatures_pass_bit_for_bit() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut ran = 0;
    let mut lines = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] != "file" {
            continue;
        }
        let label = case["case"].as_str().expect("case");
        let expected = expected_columns(case);
        let actual = run_file(&case["inputs"]);
        lines.extend(differences(label, &expected, &actual, &|_| false));
        ran += 1;
    }
    assert!(
        lines.is_empty(),
        "the library's Z conversion differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 12, "the file holds 12 cases on miniatures");
}

#[test]
fn an_override_changes_the_converted_z_and_the_laser_that_cannot_convert_is_dropped_whole() {
    // the same layer with and without an override gives different millimetres (so the cases above are not all the same number), and an invalid resolution drops the laser
    let doc = load_vectors(CHUNK).expect("the vectors");
    let inputs = |label: &str| -> Value {
        doc["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|c| c["case"] == label)
            .unwrap_or_else(|| panic!("no case {label}"))["inputs"]
            .clone()
    };
    let plain = run_file(&inputs("file_v17_mock_default"));
    let wide = run_file(&inputs("file_v17_mock_override_field_size_z_100"));
    assert_ne!(plain["laser0_cmd_z_digest"], wide["laser0_cmd_z_digest"]);
    assert_eq!(plain["laser0_n"], wide["laser0_n"]);
    let dropped = run_file(&inputs(
        "file_v17_mock_invalid_override_resolution_makes_the_laser_drop_out",
    ));
    assert_eq!(dropped["lasers_present"], whole(&[]));
}
