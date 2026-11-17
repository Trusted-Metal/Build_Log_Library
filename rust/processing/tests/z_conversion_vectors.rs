//! The Z calibration and conversion against the pure cases of Vision's vectors (IP-27 phase 27D; decisions D27-6, D27-14, D27-15 and D27-18; requirement BL-PR-03).
//!
//! `testdata/vectors/z_conversion.json` was generated in Vision from its own code at the commit it names, and is held to its hash by the test kit. Each **pure** case gives the file's
//! calibration values, the user's overrides, the width of each Z dataset and the codes; Vision recorded the resolved calibration, the resolution of each series, the millimetres, or why
//! the laser is unavailable. Each is run **through the library's resolver and converter** and compared **bit for bit**.
//!
//! * **Vision drops the whole laser** when either series is refused (the commanded one is checked first). The library reports each series, so this test makes Vision's decision in
//!   two lines (`first_refusal`), exactly as the adapter of phase 27K will; D27-18.
//! * **What is not compared** is Vision's wording of a reason. What is checked instead is that Vision's reason **contains** the library's sentence, which is the same rule in the same
//!   words (Vision's pipeline puts "coordinate conversion failed: invalid parameter:" in front of the first).
//!
//! The twelve cases on the miniatures are run by `rust/build-log/tests/z_conversion_vectors.rs`, because they need a file to be opened.

use build_log_conformance::vectors::{
    bits, differences, expected_columns, load_vectors, opt_f64, opt_i64, whole, Columns,
};
use build_log_model::{LaserParameters, Quantity, ZCalibrationOverride};
use build_log_processing::{
    resolve_z_calibration, Resolved, StoredWidths, ZConverter, ZRefusal, ZSeries,
};
use serde_json::Value;

const CHUNK: &str = "z_conversion";

// ---------------------------------------------------------------- what Vision does, and the library does not (decision D27-18)

/// Vision's defaults for a Z calibration that the file and the user do not give (Vision's `CalibrationParams::default()`). **They are Vision's:** the library reports a missing value as
/// missing, and the adapter of phase 27K will carry these, as this test does.
const VISION_FIELD_SIZE_Z_MM: f64 = 400.0;
const VISION_OFFSET_Z_MM: f64 = 0.0;
const VISION_BIT_RESOLUTION: i64 = 20;

/// Vision's narrower-dataset rule: a series is converted at the width of its stored type when that is smaller than the calibrated resolution (itself Vision's 20 when none is given).
fn vision_resolution(calibrated: Option<i64>, stored_width: Option<u32>) -> i64 {
    let calibrated = calibrated.unwrap_or(VISION_BIT_RESOLUTION);
    match stored_width {
        Some(width) if i64::from(width) < calibrated => i64::from(width),
        _ => calibrated,
    }
}

/// The file's calibration of a case, as the parameter snapshot a V17 or V16 adapter would hold: a value that the case leaves out is a value that the file does not have.
fn parameters_of(calibration: &Value) -> LaserParameters {
    let mut p = LaserParameters::new();
    p.scanner.field_size_z =
        opt_f64(&calibration["field_size_z"]).map(|v| Quantity::from_file(v, Some("mm")));
    p.scanner.offset_z =
        opt_f64(&calibration["offset_z"]).map(|v| Quantity::from_file(v, Some("mm")));
    p.galvo_z.commanded_bit_resolution = opt_i64(&calibration["commanded_bit_res_z"]);
    p.galvo_z.actual_bit_resolution = opt_i64(&calibration["actual_bit_res_z"]);
    // the vector gives the sample period in seconds, so it is a number with the unit of seconds
    p.scanner_card.sample_period =
        opt_f64(&calibration["sample_period_secs"]).map(|v| Quantity::from_file(v, Some("s")));
    p
}

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

fn width_of(value: &Value) -> Option<u32> {
    opt_i64(value).map(|w| u32::try_from(w).expect("a width"))
}

fn codes(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .expect("codes")
        .iter()
        .map(|c| f64::from(i32::try_from(c.as_i64().expect("a whole code")).expect("an i32 code")))
        .collect()
}

/// The library's answer for a case, as columns, and the sentence of the first refusal, if any. **The library reports what the file and the user give; Vision's defaults and rules are applied
/// here, as its adapter will apply them** (D27-18), and Vision's decision that either series refused makes the laser unavailable, the commanded one checked first.
fn run(inputs: &Value) -> (Columns, Option<String>) {
    let parameters = parameters_of(&inputs["calibration"]);
    let calibration = resolve_z_calibration(Some(&parameters), &override_of(&inputs["override"]));
    let widths = StoredWidths::new(
        width_of(&inputs["dataset_width_commanded"]),
        width_of(&inputs["dataset_width_actual"]),
    );

    // Vision's: the defaults, and the narrower-dataset rule
    let field_size = calibration
        .field_size_mm()
        .map_or(VISION_FIELD_SIZE_Z_MM, Resolved::value);
    let offset = calibration
        .offset_mm()
        .map_or(VISION_OFFSET_Z_MM, Resolved::value);
    let commanded_resolution = vision_resolution(
        calibration.commanded_resolution().map(Resolved::value),
        widths.commanded,
    );
    let actual_resolution = vision_resolution(
        calibration.actual_resolution().map(Resolved::value),
        widths.actual,
    );

    let mut out = Columns::new();
    out.insert("field_size_z".into(), bits(&[field_size]));
    out.insert("offset_z".into(), bits(&[offset]));
    out.insert("commanded_bit_res_z".into(), whole(&[commanded_resolution]));
    out.insert("actual_bit_res_z".into(), whole(&[actual_resolution]));
    // the library's: each series reports its own refusal; Vision checks the commanded one first
    let commanded = ZConverter::new(ZSeries::Commanded, field_size, offset, commanded_resolution);
    let actual = ZConverter::new(ZSeries::Actual, field_size, offset, actual_resolution);
    let refusal: Option<ZRefusal> = commanded.as_ref().err().or(actual.as_ref().err()).cloned();
    let (commanded_mm, actual_mm) = match (&commanded, &actual) {
        (Ok(commanded), Ok(actual)) => (
            codes(&inputs["bits_commanded"])
                .into_iter()
                .map(|c| commanded.to_mm(c))
                .collect(),
            codes(&inputs["bits_actual"])
                .into_iter()
                .map(|c| actual.to_mm(c))
                .collect(),
        ),
        _ => (Vec::new(), Vec::new()),
    };
    out.insert("commanded_z_mm".into(), bits(&commanded_mm));
    out.insert("actual_z_mm".into(), bits(&actual_mm));
    (out, refusal.map(|r| r.to_string()))
}

#[test]
fn the_pure_cases_of_the_vectors_pass_bit_for_bit() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut ran = 0;
    let mut lines = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] != "pure" {
            continue;
        }
        let label = case["case"].as_str().expect("case");
        let expected = expected_columns(case);
        let (actual, refusal) = run(&case["inputs"]);
        // Vision's own words for a refusal are not compared (see the header)
        // (the sample period is not the library's: it passes through Vision's calibration and is the time shift's, so it is not compared)
        lines.extend(differences(label, &expected, &actual, &|name| {
            name == "laser_unavailable" || name == "sample_period_secs"
        }));
        // but the refusal is: none when Vision recorded none, and when it did, Vision's reason contains the library's sentence
        let recorded = match &expected["laser_unavailable"] {
            build_log_conformance::vectors::Col::Text(t) => t.first().cloned(),
            other => panic!("{label}: laser_unavailable is {other:?}"),
        };
        match (&recorded, &refusal) {
            (None, None) => {}
            (Some(vision), Some(ours)) if vision.contains(ours.as_str()) => {}
            (vision, ours) => lines.push(format!(
                "{label}: Vision's reason was {vision:?} and the library's refusal {ours:?}"
            )),
        }
        // and the columns say the laser has no millimetres when it is refused
        if refusal.is_some() {
            assert_eq!(actual["commanded_z_mm"], bits(&[]), "{label}");
        }
        ran += 1;
    }
    assert!(
        lines.is_empty(),
        "the library's Z conversion differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 24, "the file holds 24 pure cases");
}

#[test]
fn the_refused_cases_name_each_cause_in_the_words_the_vectors_have() {
    // the six refusals of the vectors, each by cause: Vision's reason contains the library's sentence, so these are the library's sentences
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut sentences = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] == "pure" {
            if let (_, Some(sentence)) = run(&case["inputs"]) {
                sentences.push(sentence);
            }
        }
    }
    sentences.sort();
    assert_eq!(
        sentences,
        [
            "actual Z bit resolution -4 is outside 1..=32",
            "commanded Z bit resolution 0 is outside 1..=32",
            "commanded Z bit resolution 0 is outside 1..=32",
            "commanded Z bit resolution 33 is outside 1..=32",
            "field_size_z must be finite, got NaN",
            "offset_z must be finite, got NaN",
        ]
    );
    // and the cases that look odd but are accepted are not among them
    let accepted: Vec<&str> = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .filter(|c| c["kind"] == "pure")
        .filter(|c| run(&c["inputs"]).1.is_none())
        .map(|c| c["case"].as_str().expect("case"))
        .filter(|label| {
            label.contains("field_size_of_zero") || label.contains("negative_z_field_size")
        })
        .collect();
    assert_eq!(accepted.len(), 2, "{accepted:?}");
}
