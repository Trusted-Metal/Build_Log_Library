//! The beam resolver against the cases of Vision's vectors that are on the library's miniatures (IP-27 phase 27H; decisions D27-6, D27-14, D27-15, D27-18 and D27-23; BL-PR-03).
//!
//! `testdata/vectors/spot_size_resolver.json` was generated in Vision from its own code at the commit it names. Each **file** case names one of the miniatures and optional overrides of the beam;
//! for each optical train that Vision's loader reads, Vision recorded **the twelve numbers that reach the resolver**, the result of resolving them (the six numbers of the beam and its notes, or
//! the reason), and the build-level warnings. Here each is run **through the library**: the file is opened, each laser's parameters are read, brought to the resolver's units with Vision's table, and
//! resolved; Vision's own texts are rendered from the library's findings, and everything is compared **bit for bit**. The 50 pure cases are run by
//! `rust/processing/tests/spot_size_resolver_vectors.rs`; this file is here because it needs a file to open.
//!
//! **A train of Vision's is a laser of the library's** (laser N uses train N, decision D13-18). **What Vision decides is in `vision_beam`, as the adapter of phase 27K will make it** (D27-18).
//!
//! **One difference is listed, with its cause.** Vision's V16 loader gives each laser a train of zeros (the owner, 2026-11-17: accepted as it is), and so drops the `Major_Axis_Angle` that the V16 file
//! records for each laser: 0 for the first and **180 for the second**. The library reads it, so that one input differs; **the result does not**, because a half turn folds away and both say there is no
//! waist.

#[path = "../../processing/tests/vision_beam/mod.rs"]
mod vision_beam;

use build_log::open;
use build_log::testing::miniature_path;
use build_log_conformance::vectors::{
    differences, expected_columns, load_vectors, whole, words, Columns,
};
use build_log_processing::{resolve_beam_of, BeamInput, RecordedBeam};
use serde_json::Value;

use vision_beam::{error_text, notes_of, override_of, put_input, put_result, vision_assumptions};

const CHUNK: &str = "spot_size_resolver";

/// The case whose second laser's recorded axis angle Vision's loader does not carry, and the column that differs.
const V16_CASE: &str = "file_v16_trains_are_default_zeros_so_there_is_no_waist";
const V16_ANGLE_COLUMN: &str = "train1_input_major_axis_angle_deg";

fn run_file(inputs: &Value) -> Columns {
    let opened = open(miniature_path(
        inputs["miniature"].as_str().expect("miniature"),
    ))
    .expect("the miniature opens");
    let build = opened.source().build().expect("the build");
    let mut lasers: Vec<_> = build.lasers().iter().collect();
    lasers.sort_by_key(|laser| laser.key.number);

    let user = override_of(&inputs["overrides"]);
    let assumptions = vision_assumptions();
    let none = build_log::SpotSizeOverride::new();
    let mut out = Columns::new();
    out.insert("trains".into(), whole(&[lasers.len() as i64]));
    let mut warnings: Vec<String> = Vec::new();
    for (i, laser) in lasers.iter().enumerate() {
        let parameters = laser.parameters.as_ref();
        // a laser with no parameters is, to Vision, a train of zeros
        let input = parameters.map_or_else(BeamInput::default, |p| {
            BeamInput::from_recorded(&RecordedBeam::from_parameters(p), &assumptions)
        });
        let prefix = format!("train{i}_");
        put_input(&mut out, &prefix, &input);
        put_result(
            &mut out,
            &prefix,
            &resolve_beam_of(parameters, &user, &assumptions),
        );
        // the build-level warnings: one pass for each train with no overrides
        match resolve_beam_of(parameters, &none, &assumptions) {
            Ok(beam) => warnings.extend(
                notes_of(&beam)
                    .into_iter()
                    .map(|note| format!("Optical train {}: {note}", i + 1)),
            ),
            Err(error) => warnings.push(format!(
                "Optical train {}: spot size unavailable \u{2014} {}",
                i + 1,
                error_text(&error)
            )),
        }
    }
    let warnings: Vec<&str> = warnings.iter().map(String::as_str).collect();
    out.insert("build_warnings".into(), words(&warnings));
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
        lines.extend(differences(label, &expected, &actual, &|column| {
            label == V16_CASE && column == V16_ANGLE_COLUMN
        }));
        ran += 1;
    }
    assert!(
        lines.is_empty(),
        "the library's beam resolver differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 11, "the file holds 11 cases on miniatures");
}

/// The cases above would pass if the library resolved nothing and the vectors recorded that. They do not: three of the seven miniatures have no beam and four have one, and an override rescues a
/// build that has none (`file_v17_march_overrides_rescue_a_build_without_a_waist`). **A V16 laser has a snapshot and no waist**, so the library says no waist, as Vision does, and not no train.
#[test]
fn the_files_with_no_beam_are_told_from_the_files_with_one() {
    use build_log_processing::BeamUnavailable;
    let assumptions = vision_assumptions();
    let none = build_log::SpotSizeOverride::new();
    let verdict = |name: &str| -> Vec<Result<bool, BeamUnavailable>> {
        let opened = open(miniature_path(name)).expect("opens");
        let build = opened.source().build().expect("build");
        let mut lasers: Vec<_> = build.lasers().iter().collect();
        lasers.sort_by_key(|laser| laser.key.number);
        lasers
            .iter()
            .map(|laser| {
                assert!(
                    laser.parameters.is_some(),
                    "{name}: every laser of a miniature has a snapshot"
                );
                resolve_beam_of(laser.parameters.as_ref(), &none, &assumptions).map(|_| true)
            })
            .collect()
    };
    // V16, March and the special layers: a snapshot with no usable waist
    for name in ["v16.h5", "v17_march.h5", "special_layers_v17.h5"] {
        assert_eq!(
            verdict(name),
            vec![Err(BeamUnavailable::NoWaist), Err(BeamUnavailable::NoWaist)],
            "{name}"
        );
    }
    // the four current files resolve both lasers
    for name in [
        "v17_mock.h5",
        "v17_triggered_2026_10_19.h5",
        "v17_oct15.h5",
        "v17_2026_08_18_images.h5",
    ] {
        assert_eq!(verdict(name), vec![Ok(true), Ok(true)], "{name}");
    }
}

/// **The listed difference.** Vision's V16 loader gives each laser a train of zeros, and so does not carry the `Major_Axis_Angle` that a V16 file records for each laser: 0 for the first and 180 for the
/// second (the second scanner is turned half a turn, as the second train of the V17 files is). The library reads it. It cannot change the beam: an ellipse is unchanged by a half turn, and
/// the sum is folded into `[0, 180)`.
#[test]
fn the_axis_angle_that_vision_does_not_carry_for_v16_is_the_one_listed_difference() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let case = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["case"] == V16_CASE)
        .expect("the case");
    let expected = expected_columns(case);
    let actual = run_file(&case["inputs"]);
    let angle = |columns: &Columns| match &columns[V16_ANGLE_COLUMN] {
        build_log_conformance::vectors::Col::F64(values) => f64::from_bits(values[0]),
        other => panic!("{other:?}"),
    };
    assert_eq!(angle(&expected), 0.0, "Vision's train of zeros");
    assert_eq!(
        angle(&actual),
        180.0,
        "what the V16 file records for the second laser"
    );
    // that column is the only one that differs
    let lines = differences(V16_CASE, &expected, &actual, &|_| false);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(V16_ANGLE_COLUMN));
}
