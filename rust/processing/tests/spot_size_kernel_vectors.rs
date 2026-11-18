//! The spot-size kernel against the pure cases of Vision's vectors (IP-27 phase 27I; decisions D27-6, D27-14 and D27-15; requirements BL-PR-03, BL-PR-04 and BL-PR-07).
//!
//! `testdata/vectors/spot_size_kernel.json` was generated in Vision from its own code (the `spot_size` crate's CPU `f64` path) at the commit it names, and is held to its hash by the test kit: **it is
//! the golden file of the algorithm, and the checksum guard fails when a vector changes without it** (`the_vectors_match_their_checksums`). Each **pure** case is one of two operations, both run
//! **through the library** and compared **bit for bit** (a float input is read from its exact bits):
//!
//! * `defocus`: `commanded_defocus` and `actual_defocus` on a count, a focus shift and two Z slices;
//! * `kernel`: `compute_spot_size` on a beam and a request: the commanded and the actual radii and areas, and the angle.
//!
//! Nothing here is Vision's decision: the kernel has no policy. The nine cases on the miniatures (a layer through Tier 1 and the whole spot-size path) are run by
//! `rust/build-log/tests/spot_size_kernel_vectors.rs`, which needs a file to open.

use build_log_conformance::vectors::{
    bits, differences, exact_f64, exact_f64s, expected_columns, load_vectors, Columns,
};
use build_log_processing::{
    actual_defocus, commanded_defocus, compute_spot_size, BeamShape, SpotKernelInput, SpotSeries,
};
use serde_json::Value;

const CHUNK: &str = "spot_size_kernel";

fn count(inputs: &Value, key: &str) -> usize {
    usize::try_from(
        inputs[key]
            .as_u64()
            .unwrap_or_else(|| panic!("{key} is a count")),
    )
    .expect("a count")
}

fn defocus(inputs: &Value) -> Columns {
    let n = count(inputs, "n");
    let shift = exact_f64(inputs, "focus_shift_mm");
    let actual_z = exact_f64s(inputs, "actual_z");
    let commanded_z = exact_f64s(inputs, "commanded_z");
    let mut out = Columns::new();
    out.insert("commanded".into(), bits(&commanded_defocus(n, shift)));
    out.insert(
        "actual".into(),
        bits(&actual_defocus(n, &actual_z, &commanded_z, shift)),
    );
    out
}

fn put_series(out: &mut Columns, prefix: &str, series: &SpotSeries) {
    out.insert(
        format!("{prefix}_major_radii_um"),
        bits(&series.major_radii_um),
    );
    out.insert(
        format!("{prefix}_minor_radii_um"),
        bits(&series.minor_radii_um),
    );
    out.insert(format!("{prefix}_areas_um2"), bits(&series.areas_um2));
}

fn kernel(inputs: &Value) -> Columns {
    let beam = &inputs["beam"];
    let shape = BeamShape {
        waist_radius_major_um: exact_f64(beam, "waist_radius_major_um"),
        waist_radius_minor_um: exact_f64(beam, "waist_radius_minor_um"),
        rayleigh_major_mm: exact_f64(beam, "rayleigh_major_mm"),
        rayleigh_minor_mm: exact_f64(beam, "rayleigh_minor_mm"),
        focus_shift_mm: exact_f64(beam, "focus_shift_mm"),
        axis_angle_deg: exact_f64(beam, "major_axis_angle_deg"),
    };
    let commanded_z = exact_f64s(inputs, "commanded_z");
    let actual_z = exact_f64s(inputs, "actual_z");
    let result = compute_spot_size(
        &shape,
        &SpotKernelInput {
            commanded_samples: count(inputs, "commanded_samples"),
            actual_samples: count(inputs, "actual_samples"),
            commanded_z_mm: &commanded_z,
            actual_z_mm: &actual_z,
        },
    );
    let mut out = Columns::new();
    put_series(&mut out, "commanded", &result.commanded);
    put_series(&mut out, "actual", &result.actual);
    out.insert(
        "major_axis_angle_deg".into(),
        bits(&[result.axis_angle_deg]),
    );
    out
}

fn run(inputs: &Value) -> Columns {
    match inputs["operation"].as_str().expect("operation") {
        "defocus" => defocus(inputs),
        "kernel" => kernel(inputs),
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
        "the library's spot-size kernel differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 29, "the file holds 29 pure cases");
    assert_eq!(by_operation["defocus"], 11);
    assert_eq!(by_operation["kernel"], 18);
}
