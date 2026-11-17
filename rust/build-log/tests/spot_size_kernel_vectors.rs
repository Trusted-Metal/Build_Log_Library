//! The spot size against the cases of Vision's vectors that are on the library's miniatures (IP-27 phase 27I; decisions D27-6, D27-14, D27-15, D27-18 and D27-23; BL-PR-03 and BL-PR-07).
//!
//! `testdata/vectors/spot_size_kernel.json` was generated in Vision from its own code at the commit it names, and is the golden file of the algorithm. Each **file** case names one of the
//! miniatures, a layer by its position, a selection and optional overrides of the beam; Vision took the layer through **its whole path** (Tier 1, then the spot size of each laser through the
//! beam of the laser's train and the kernel on the processed Z) and recorded, for each laser it keeps, the count, a digest of each of its six series, the first and last values of three of them, and
//! the angle, and the warning of each laser that has no beam. Here each is run **through the library, piece by piece**: the file is opened and the rows of each laser are chosen (27C), the Z is
//! converted to millimetres (27D) and shifted in time (27F), the beam is resolved from the laser's parameters and the user's overrides (27H), and the spot is calculated (27I); everything is
//! compared **bit for bit**. The 29 pure cases are run by `rust/processing/tests/spot_size_kernel_vectors.rs`; this file is here because it needs a file to open.
//!
//! **The decisions are Vision's, made as the adapter of phase 27K will make them** (D27-18): in `vision_adapter` (Vision's Z defaults, which lasers it keeps, the answers for what the file does not
//! say about the timing) and in `vision_beam` (the seven answers for the beam and the text of a warning).

mod vision_adapter;
#[path = "../../processing/tests/vision_beam/mod.rs"]
mod vision_beam;

use build_log::{ReadSelection, TimingOverride, ZCalibrationOverride};
use build_log_conformance::vectors::{
    bits, differences, digest_f64, expected_columns, head_and_tail, load_vectors, whole, words,
    Columns,
};
use build_log_processing::{
    resolve_beam_of, spot_size_from_z, SpotSeries, SPOT_SIZE_ALGORITHM_VERSION,
};
use serde_json::Value;

use vision_adapter::{kept_layer, rows_of, shift_laser, ShiftedLaser};
use vision_beam::{error_text, override_of, vision_assumptions};

const CHUNK: &str = "spot_size_kernel";

/// The six series of a laser's spot, by the names the vectors give them.
fn named<'a>(
    commanded: &'a SpotSeries,
    actual: &'a SpotSeries,
) -> [(&'static str, &'a Vec<f64>); 6] {
    [
        ("cmd_major", &commanded.major_radii_um),
        ("cmd_minor", &commanded.minor_radii_um),
        ("cmd_area", &commanded.areas_um2),
        ("act_major", &actual.major_radii_um),
        ("act_minor", &actual.minor_radii_um),
        ("act_area", &actual.areas_um2),
    ]
}

fn run_file(inputs: &Value) -> Columns {
    let stride = inputs["stride"].as_u64().expect("stride");
    let selection = ReadSelection::new()
        .with_stride(stride)
        .with_rows(rows_of(inputs["rows"].as_str().expect("rows")));
    let layer = kept_layer(
        inputs["miniature"].as_str().expect("miniature"),
        usize::try_from(inputs["layer_position"].as_u64().expect("layer_position"))
            .expect("layer_position"),
        &selection,
        &ZCalibrationOverride::new(),
    );
    let beam_override = override_of(&inputs["override"]);
    let assumptions = vision_assumptions();

    let mut out = Columns::new();
    let mut present: Vec<i64> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for laser in &layer.lasers {
        present.push(laser.index as i64);
        let p = format!("laser{}", laser.index);
        // the Z after the time shift, with no timing override (the layer cases have none)
        let ShiftedLaser {
            commanded, actual, ..
        } = shift_laser(laser, stride, &TimingOverride::new());
        let beam = match resolve_beam_of(laser.parameters.as_ref(), &beam_override, &assumptions) {
            Ok(beam) => beam,
            Err(error) => {
                // a laser with no beam has no spot size, and Vision warns
                warnings.push(format!(
                    "{}: spot size unavailable \u{2014} {}",
                    laser.name,
                    error_text(&error)
                ));
                out.insert(format!("{p}_has_spot_size"), whole(&[0]));
                continue;
            }
        };
        let spot = spot_size_from_z(&beam, &commanded, &actual);
        assert_eq!(spot.algorithm_version(), SPOT_SIZE_ALGORITHM_VERSION);
        out.insert(format!("{p}_has_spot_size"), whole(&[1]));
        out.insert(
            format!("{p}_n"),
            whole(&[spot.commanded().major_radii_um.len() as i64]),
        );
        out.insert(
            format!("{p}_major_axis_angle_deg"),
            bits(&[spot.axis_angle_deg()]),
        );
        let series = named(spot.commanded(), spot.actual());
        for (name, values) in series {
            out.insert(format!("{p}_{name}_digest"), whole(&[digest_f64(values)]));
        }
        for (name, values) in series {
            if matches!(name, "cmd_major" | "act_major" | "act_area") {
                let (head, tail) = head_and_tail(values);
                out.insert(format!("{p}_{name}_head"), bits(&head));
                out.insert(format!("{p}_{name}_tail"), bits(&tail));
            }
        }
    }
    out.insert("lasers_present".into(), whole(&present));
    let warnings: Vec<&str> = warnings.iter().map(String::as_str).collect();
    out.insert("warnings".into(), words(&warnings));
    out
}

fn case_inputs(doc: &Value, label: &str) -> Value {
    doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["case"] == label)
        .unwrap_or_else(|| panic!("no case {label}"))["inputs"]
        .clone()
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
        "the library's spot size differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 9, "the file holds 9 cases on miniatures");
}

/// The cases above would pass if the library calculated nothing and the vectors recorded that. They do not: a layer with a beam has a spot, one without has none and a warning, an override of the
/// beam changes the spot and the angle, and the mock's actual spot is far from its commanded one (the file's own data: its actual Z does not follow its command).
#[test]
fn the_spot_follows_the_beam_and_the_z_and_a_laser_with_no_beam_has_none() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let oct15 = run_file(&case_inputs(&doc, "file_layer_v17_oct15_all_rows"));
    let mock = run_file(&case_inputs(&doc, "file_layer_v17_mock_all_rows"));
    let overridden = run_file(&case_inputs(
        &doc,
        "file_layer_v17_mock_overrides_change_the_spot",
    ));
    let march = run_file(&case_inputs(
        &doc,
        "file_layer_v17_march_has_no_beam_so_no_spot_size_and_a_warning",
    ));
    let rescued = run_file(&case_inputs(
        &doc,
        "file_layer_v17_march_overrides_rescue_a_beam_and_a_spot_size_is_made",
    ));
    assert_eq!(oct15["laser0_has_spot_size"], whole(&[1]));
    assert_eq!(march["laser0_has_spot_size"], whole(&[0]));
    assert_eq!(march["warnings"].clone(), {
        let two = [
            "Laser_1: spot size unavailable \u{2014} no beam waist stored (Beam_Waist_Major/Minor missing or zero)",
            "Laser_2: spot size unavailable \u{2014} no beam waist stored (Beam_Waist_Major/Minor missing or zero)",
        ];
        words(&two)
    });
    // an override rescues a layer with no beam
    assert_eq!(rescued["laser0_has_spot_size"], whole(&[1]));
    // an override changes the spot, and the angle (the scanner rotation override is 20 degrees)
    assert_ne!(
        mock["laser0_cmd_major_digest"],
        overridden["laser0_cmd_major_digest"]
    );
    assert_ne!(
        mock["laser0_major_axis_angle_deg"],
        overridden["laser0_major_axis_angle_deg"]
    );
    // the mock's actual spot is not its commanded one; October 15's is within a hair of it
    assert_ne!(
        mock["laser0_cmd_major_digest"],
        mock["laser0_act_major_digest"]
    );
    assert_ne!(
        oct15["laser0_cmd_major_digest"],
        oct15["laser0_act_major_digest"]
    );
}
