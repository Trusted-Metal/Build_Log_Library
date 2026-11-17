//! The beam resolver against the pure cases of Vision's vectors (IP-27 phase 27H; decisions D27-6, D27-14, D27-15, D27-18 and D27-23; requirements BL-PR-03 and BL-PR-04).
//!
//! `testdata/vectors/spot_size_resolver.json` was generated in Vision from its own code at the commit it names, and is held to its hash by the test kit. Each **pure** case is one of two operations,
//! both run **through the library** and compared **bit for bit** (a float input is read from its exact bits):
//!
//! * `beam`: the train's numbers and the user's overrides through `resolve_beam`, and Vision's notes and reason rendered from what it returns (see `vision_beam`);
//! * `units`: the reader's own `read_length_attr` and `read_angle_attr_degrees` on attributes with and without a unit companion, through `RecordedBeam::from_parameters` (which attribute is the
//!   value: the first name found wins), `BeamInput::from_recorded` (the unit that the caller assumes, the table and the arithmetic).
//!
//! **What Vision decides, and the library does not, is in `vision_beam`** (D27-18). **One difference is listed, with its cause:** Vision reads `Thermal_Lensing_Focal_Shift` before
//! `Thermal_Lensing_Focal_Plane_Shift`; the library types only the second (decision D27-23: the alias is left until a producer is known), so the two probes of the vectors that have the alias differ.
//! The eleven cases on the miniatures are run by `rust/build-log/tests/spot_size_resolver_vectors.rs`, which needs a file to open.

mod vision_beam;

use build_log_conformance::vectors::{
    bits, differences, exact_f64, expected_columns, load_vectors, whole, Columns,
};
use build_log_model::{LaserParameters, Quantity};
use build_log_processing::{resolve_beam, BeamInput, LengthUnit, RecordedBeam};
use serde_json::Value;

use vision_beam::{override_of, put_result, vision_assumptions};

const CHUNK: &str = "spot_size_resolver";

/// The case whose probes include the alias, and the probes of it that differ.
const ALIAS_CASE: &str = "pure_units_the_thermal_shift_is_read_under_two_names_and_the_first_wins";

// ---------------------------------------------------------------- the beam

fn beam(inputs: &Value) -> Columns {
    let input = &inputs["input"];
    let mut numbers = BeamInput::default();
    numbers.waist_major_um = exact_f64(input, "waist_major_um");
    numbers.waist_minor_um = exact_f64(input, "waist_minor_um");
    numbers.rayleigh_major_mm = exact_f64(input, "rayleigh_major_mm");
    numbers.rayleigh_minor_mm = exact_f64(input, "rayleigh_minor_mm");
    numbers.m2_major = exact_f64(input, "m2_major");
    numbers.m2_minor = exact_f64(input, "m2_minor");
    numbers.wavelength_nm = exact_f64(input, "wavelength_nm");
    numbers.waist_offset_z_mm = exact_f64(input, "waist_offset_z_mm");
    numbers.thermal_shift_mm = exact_f64(input, "thermal_shift_mm");
    numbers.thermal_threshold_mm = exact_f64(input, "thermal_threshold_mm");
    numbers.thermal_test_passed = input["thermal_test_passed"].as_bool();
    numbers.major_axis_angle_deg = exact_f64(input, "major_axis_angle_deg");
    numbers.scanner_rotation_deg = exact_f64(input, "scanner_rotation_deg");
    let result = resolve_beam(
        &numbers,
        &override_of(&inputs["overrides"]),
        &vision_assumptions(),
    );
    let mut out = Columns::new();
    put_result(&mut out, "", &result);
    out
}

// ---------------------------------------------------------------- the units

/// The unit that a multiplier to millimetres in the vectors stands for.
fn unit_of(to_mm: f64) -> LengthUnit {
    [
        LengthUnit::Nanometre,
        LengthUnit::Micrometre,
        LengthUnit::Millimetre,
        LengthUnit::Centimetre,
        LengthUnit::Metre,
    ]
    .into_iter()
    .find(|unit| unit.to_millimetres().to_bits() == to_mm.to_bits())
    .unwrap_or_else(|| panic!("no unit has the multiplier {to_mm}"))
}

/// The snapshot a probe stands for: each attribute is the typed value of that name. **An attribute that the library does not type** (`Thermal_Lensing_Focal_Shift`, the alias) **or does not
/// read for the beam is not in it.**
fn parameters_of(probe: &Value) -> LaserParameters {
    let mut parameters = LaserParameters::new();
    for attribute in probe["attributes"].as_array().expect("attributes") {
        let name = attribute["name"].as_str().expect("name");
        let quantity =
            Quantity::from_file(exact_f64(attribute, "value"), attribute["unit"].as_str());
        match name {
            "Beam_Waist_Major" => parameters.optics.waist_major = Some(quantity),
            "Major_Waist_D4Sigma" => parameters.optics.major_waist_d4sigma = Some(quantity),
            "Thermal_Lensing_Focal_Plane_Shift" => {
                parameters.optics.thermal_lensing_focal_plane_shift = Some(quantity);
            }
            "Rayleigh_Length_Major" => parameters.optics.rayleigh_length_major = Some(quantity),
            "Beam_Waist_Offset_Z" => parameters.optics.waist_offset_z = Some(quantity),
            "Major_Axis_Angle" => parameters.optics.major_axis_angle = Some(quantity),
            // not typed by the library, or not a beam value
            "Thermal_Lensing_Focal_Shift" | "Something_Else" => {}
            other => panic!("a probe names an attribute this test does not know: {other}"),
        }
    }
    parameters
}

/// Whether the library found the value of a probe, and the value in the unit the reader gives it. **The value is the one the first name of the probe stands for, read the way a laser's beam is:
/// through `RecordedBeam::from_parameters` and `BeamInput::from_recorded`, with the unit that the vector's default stands for as the caller's answer** for that kind of value. The answer for
/// the other lengths is a unit that would make any mix-up show (a metre), so a waist read with the unit of a Rayleigh length is caught.
fn read_probe(inputs: &Value, probe: &Value) -> (bool, f64) {
    let recorded = RecordedBeam::from_parameters(&parameters_of(probe));
    let first = probe["names"][0].as_str().expect("a name");
    let mut assumptions = vision_assumptions();
    assumptions.waist_unit = LengthUnit::Metre;
    assumptions.length_unit = LengthUnit::Metre;
    let (found, target, assumed) = match inputs["kind"].as_str().expect("kind") {
        "length" => {
            let assumed = unit_of(exact_f64(inputs, "default_unit_mm"));
            let (found, target) = match first {
                "Beam_Waist_Major" => {
                    assumptions.waist_unit = assumed;
                    (recorded.waist_major().is_some(), LengthUnit::Micrometre)
                }
                "Thermal_Lensing_Focal_Shift" => {
                    assumptions.length_unit = assumed;
                    (recorded.thermal_shift().is_some(), LengthUnit::Millimetre)
                }
                "Rayleigh_Length_Major" => {
                    assumptions.length_unit = assumed;
                    (recorded.rayleigh_major().is_some(), LengthUnit::Millimetre)
                }
                "Beam_Waist_Offset_Z" => {
                    assumptions.length_unit = assumed;
                    (recorded.waist_offset().is_some(), LengthUnit::Millimetre)
                }
                other => panic!("a length probe whose first name is {other}"),
            };
            (found, Some(target), Some(assumed))
        }
        "angle" => {
            assert_eq!(first, "Major_Axis_Angle");
            (recorded.major_axis_angle().is_some(), None, None)
        }
        other => panic!("kind is length or angle, not {other}"),
    };
    // the vector's target is the unit the resolver works in for that value
    if let (Some(target), Some(_)) = (target, assumed) {
        assert_eq!(
            unit_of(exact_f64(inputs, "target_mm_per_unit")),
            target,
            "{first}"
        );
    }
    if !found {
        return (false, f64::NAN);
    }
    let input = BeamInput::from_recorded(&recorded, &assumptions);
    let value = match first {
        "Beam_Waist_Major" => input.waist_major_um,
        "Thermal_Lensing_Focal_Shift" => input.thermal_shift_mm,
        "Rayleigh_Length_Major" => input.rayleigh_major_mm,
        "Beam_Waist_Offset_Z" => input.waist_offset_z_mm,
        "Major_Axis_Angle" => input.major_axis_angle_deg,
        other => panic!("{other}"),
    };
    (true, value)
}

fn units(inputs: &Value) -> Columns {
    let mut found = Vec::new();
    let mut values = Vec::new();
    for probe in inputs["probes"].as_array().expect("probes") {
        let (was_found, value) = read_probe(inputs, probe);
        found.push(i64::from(was_found));
        values.push(value);
    }
    let mut out = Columns::new();
    out.insert("found".into(), whole(&found));
    out.insert("values".into(), bits(&values));
    out
}

fn run(inputs: &Value) -> Columns {
    match inputs["operation"].as_str().expect("operation") {
        "beam" => beam(inputs),
        "units" => units(inputs),
        other => panic!("unknown operation {other:?}"),
    }
}

#[test]
fn the_pure_cases_pass_bit_for_bit() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut ran = 0;
    let mut by_operation = std::collections::BTreeMap::<String, usize>::new();
    let mut lines = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] != "pure" {
            continue;
        }
        let label = case["case"].as_str().expect("case");
        // the one case with the alias is run by its own test, which says where it differs
        if label == ALIAS_CASE {
            continue;
        }
        let expected = expected_columns(case);
        let actual = run(&case["inputs"]);
        lines.extend(differences(label, &expected, &actual, &|_| false));
        ran += 1;
        *by_operation
            .entry(
                case["inputs"]["operation"]
                    .as_str()
                    .expect("operation")
                    .to_string(),
            )
            .or_default() += 1;
    }
    assert!(
        lines.is_empty(),
        "the library's beam resolver differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(
        ran, 49,
        "the file holds 50 pure cases and one of them has its own test"
    );
    assert_eq!(by_operation["beam"], 44);
    assert_eq!(by_operation["units"], 5);
}

/// **A listed difference, with its cause.** Vision reads `Thermal_Lensing_Focal_Shift` before `Thermal_Lensing_Focal_Plane_Shift` and the library types only the second (decision D27-23: the alias
/// is left until a producer is known; no V17 file we hold has it). So of the four probes of the vector, the two that have the alias differ: an alias alone is not found, and an alias beside the
/// plane shift does not win. The other two (the plane shift alone, in millimetres and in micrometres) are the same, bit for bit.
#[test]
fn the_alias_of_the_thermal_shift_is_the_one_listed_difference() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let case = doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["case"] == ALIAS_CASE)
        .expect("the case");
    let expected = expected_columns(case);
    let actual = run(&case["inputs"]);
    // Vision: the plane shift alone 1.178; the alias alone 0.9; both, the alias wins: 0.9; the plane shift in micrometres 1.178
    let vision = [(true, 1.178), (true, 0.9), (true, 0.9), (true, 1.178)];
    // the library: 1.178; the alias is not read, so not found; both, the plane shift: 1.178; 1.178
    let library = [
        (true, 1.178),
        (false, f64::NAN),
        (true, 1.178),
        (true, 1.178),
    ];
    let found = |c: &Columns| match &c["found"] {
        build_log_conformance::vectors::Col::I64(values) => values.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        found(&expected),
        vision
            .iter()
            .map(|(f, _)| i64::from(*f))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        found(&actual),
        library
            .iter()
            .map(|(f, _)| i64::from(*f))
            .collect::<Vec<_>>()
    );
    // probes 0 and 3 agree with Vision to the last bit; 1 and 2 are the listed difference
    let values = |c: &Columns| match &c["values"] {
        build_log_conformance::vectors::Col::F64(values) => values.clone(),
        other => panic!("{other:?}"),
    };
    let (want, got) = (values(&expected), values(&actual));
    for probe in [0, 3] {
        assert_eq!(want[probe], got[probe], "probe {probe} is the same");
    }
    assert_eq!(f64::from_bits(want[1]), 0.9);
    assert!(f64::from_bits(got[1]).is_nan());
    assert_eq!(f64::from_bits(want[2]), 0.9);
    assert_eq!(f64::from_bits(got[2]), 1.178);
}
