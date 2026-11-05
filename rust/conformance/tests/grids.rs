//! The correction grids, held to an expectation (IP-24 phase 24B, task T24.2; decision D24-11).
//!
//! * The fingerprint of a grid is computed here **as the independent script computes it**: a test holds both to the same literals, worked out with the standard library alone, a NaN included.
//! * A source that serves the grids as stored passes, and each way of serving one wrongly is reported, on that grid and on no other.
//! * **A V16 or V17 expectation must list the grids of every laser that has a snapshot**, so that the check cannot pass by having nothing to check.
//! * The descriptors in the metadata are held field by field, on the oracle's own files for V16 and V17.

use build_log_conformance::{
    check_grids, check_parts, grid_expectations, load_all, synthesise, Expectation,
    GridFingerprint, Part,
};
use build_log_model::{
    ColumnData, Dims, GridData, GridRole, InMemorySource, LaserKey, NumericType, Shared,
};
use serde_json::{json, Value};

fn dims(extents: &[u64]) -> Dims {
    Dims::new(extents).expect("a shape")
}

fn f64_grid(extents: &[u64], values: Vec<f64>) -> GridData {
    GridData::new(dims(extents), ColumnData::F64(Shared::new(values))).expect("a grid")
}

// ---------------------------------------------------------------- the fingerprint: the same literals as the script's tests

#[test]
fn the_fingerprint_of_known_grids_is_what_hashlib_gives() {
    // two NaN among six values of float64: the count, how many are not a number, the extremes of those that are, and the bytes as stored (a quiet NaN is 00 00 00 00 00 00 f8 7f)
    let grid = f64_grid(&[2, 3], vec![1.5, f64::NAN, -2.0, f64::NAN, 0.25, 8.0]);
    assert_eq!(
        GridFingerprint::of(&grid).expect("numbers"),
        GridFingerprint {
            count: 6,
            stored_as: "F64".into(),
            not_a_number: 2,
            min: Some(-2.0),
            max: Some(8.0),
            sha256: "1285dcfaf05d613a84e703dec55853b6ece29285e6f86f38dd463581cca710f3".into(),
        }
    );
    // an int32 grid of three dimensions is hashed row-major: the last index runs fastest, as in the file
    let cube = GridData::new(
        dims(&[2, 3, 4]),
        ColumnData::I32(Shared::new((-7..17).collect())),
    )
    .expect("a grid");
    let fingerprint = GridFingerprint::of(&cube).expect("numbers");
    assert_eq!(
        (
            fingerprint.count,
            fingerprint.not_a_number,
            fingerprint.min,
            fingerprint.max
        ),
        (24, 0, Some(-7.0), Some(16.0))
    );
    assert_eq!(
        fingerprint.sha256,
        "6a14ec752521669c3c05c88746c8c3e6c6d17e9acf187d74c9d6130c31865c4d"
    );
    // a grid of only NaN has no extreme, and a grid of text has no fingerprint
    let none = f64_grid(&[2], vec![f64::NAN, f64::NAN]);
    let only = GridFingerprint::of(&none).expect("numbers");
    assert_eq!((only.not_a_number, only.min, only.max), (2, None, None));
    let text = GridData::new(
        dims(&[1]),
        ColumnData::Text(Shared::new(vec!["a".to_string()])),
    )
    .expect("a grid");
    assert!(GridFingerprint::of(&text).is_none());
}

#[test]
fn a_nan_with_a_payload_is_another_value_to_the_hash() {
    // the hash is of the bytes that are stored, so a NaN that was canonicalised on the way is not the NaN that the file holds
    let plain = f64_grid(&[1], vec![f64::NAN]);
    let odd = f64_grid(&[1], vec![f64::from_bits(0x7ff8_0000_0000_1234)]);
    assert_ne!(
        GridFingerprint::of(&plain).unwrap().sha256,
        GridFingerprint::of(&odd).unwrap().sha256
    );
    assert_eq!(GridFingerprint::of(&plain).unwrap().not_a_number, 1);
    assert_eq!(GridFingerprint::of(&odd).unwrap().not_a_number, 1);
}

// ---------------------------------------------------------------- a synthetic build: two lasers, two grids each

const LASERS: [u32; 2] = [1, 2];
const GRIDS: [&str; 2] = ["the forward", "the inverse"];

fn laser(number: u32) -> LaserKey {
    LaserKey::new(format!("Laser_{number}"), number)
}

/// The grid of a laser: different for each, with NaN in it, as the files have.
fn grid(laser: u32, which: usize) -> GridData {
    let values: Vec<f64> = (0..12)
        .map(|i| {
            if (i + laser as usize + which) % 3 == 0 {
                f64::NAN
            } else {
                (i * 37 + laser as usize * 11 + which * 101) as f64 / 8.0 - 20.0
            }
        })
        .collect();
    f64_grid(&[2, 3, 2], values)
}

/// An expectation that describes the grids as stored; `listed` says whether the snapshots list `grids` at all.
fn expectation(version: i64, listed: bool) -> Expectation {
    let lasers: Vec<Value> = LASERS
        .iter()
        .map(|&number| {
            let mut parameters = json!({
                "optical_train": null,
                "values": [],
                "volts_to_watts": null,
                "power_limits_not_recorded": false,
            });
            if listed {
                let grids: Vec<Value> = GRIDS
                    .iter()
                    .enumerate()
                    .map(|(which, name)| {
                        let fingerprint =
                            GridFingerprint::of(&grid(number, which)).expect("numbers");
                        json!({
                            "name": name,
                            "role": if which == 0 { json!("Forward") } else { json!("Inverse") },
                            "stored_as": "F64",
                            "extents": [2, 3, 2],
                            "interpretation": "Unrecorded",
                            "fingerprint": {
                                "count": fingerprint.count,
                                "stored_as": fingerprint.stored_as,
                                "not_a_number": fingerprint.not_a_number,
                                "min": fingerprint.min,
                                "max": fingerprint.max,
                                "sha256": fingerprint.sha256,
                            },
                        })
                    })
                    .collect();
                parameters["grids"] = json!(grids);
            }
            json!({ "key": format!("Laser_{number}"), "number": number, "parameters": parameters })
        })
        .collect();
    let text = json!({
        "expectations_version": 1,
        "miniature": { "file": "synthetic.h5" },
        "file_version": version,
        "lasers": lasers,
    })
    .to_string();
    Expectation::parse(&text).expect("an expectation")
}

/// The source that serves every grid as stored; `change` may serve one differently, or not at all (`None`).
fn source_with(change: impl Fn(u32, usize, GridData) -> Option<GridData>) -> InMemorySource {
    let mut source = InMemorySource::new();
    for number in LASERS {
        for (which, name) in GRIDS.iter().enumerate() {
            if let Some(data) = change(number, which, grid(number, which)) {
                source.insert_grid(laser(number), *name, data);
            }
        }
    }
    source
}

#[test]
fn a_source_that_serves_the_grids_as_stored_passes_and_each_wrong_way_is_reported_on_its_grid() {
    let expected = expectation(17, true);
    assert_eq!(grid_expectations(&expected).expect("grids").len(), 4);
    let honest = source_with(|_, _, data| Some(data));
    assert_eq!(check_grids(&honest, &expected), Vec::<String>::new());

    // one value changed, in laser 2's inverse: the hash says so, and nothing else does
    let found = check_grids(
        &source_with(|laser, which, data| {
            if (laser, which) != (2, 1) {
                return Some(data);
            }
            let ColumnData::F64(values) = &data.values else {
                unreachable!()
            };
            let mut changed = values.as_slice().to_vec();
            let at = changed.iter().position(|v| !v.is_nan()).expect("a number");
            changed[at] += 0.125;
            Some(f64_grid(&[2, 3, 2], changed))
        }),
        &expected,
    );
    assert!(
        found
            .iter()
            .all(|l| l.contains("Laser_2") && l.contains("the inverse")),
        "{found:?}"
    );
    assert!(found.iter().any(|l| l.contains("SHA-256")), "{found:?}");

    // the NaN filled in with zero: how many are not a number, the hash and (here) the extremes all say so, on that grid
    let found = check_grids(
        &source_with(|laser, which, data| {
            if (laser, which) != (1, 0) {
                return Some(data);
            }
            let ColumnData::F64(values) = &data.values else {
                unreachable!()
            };
            let repaired: Vec<f64> = values
                .as_slice()
                .iter()
                .map(|v| if v.is_nan() { 0.0 } else { *v })
                .collect();
            Some(f64_grid(&[2, 3, 2], repaired))
        }),
        &expected,
    );
    assert!(
        found
            .iter()
            .any(|l| l.contains("how many are not a number")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .all(|l| l.contains("Laser_1") && l.contains("the forward")),
        "{found:?}"
    );

    // the same values in other extents (a transposed shape), another stored type, and a grid that is not served at all
    let found = check_grids(
        &source_with(|laser, which, data| {
            if (laser, which) != (2, 0) {
                return Some(data);
            }
            let ColumnData::F64(values) = &data.values else {
                unreachable!()
            };
            Some(f64_grid(&[3, 2, 2], values.as_slice().to_vec()))
        }),
        &expected,
    );
    assert!(found.iter().any(|l| l.contains("its extents")), "{found:?}");
    // half of the values: the count says so, on that grid, besides its extents and its hash
    let found = check_grids(
        &source_with(|laser, which, data| {
            if (laser, which) != (1, 0) {
                return Some(data);
            }
            let ColumnData::F64(values) = &data.values else {
                unreachable!()
            };
            Some(f64_grid(&[2, 3, 1], values.as_slice()[..6].to_vec()))
        }),
        &expected,
    );
    assert!(
        found.iter().any(|l| l.contains("the count of values")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .all(|l| l.contains("Laser_1") && l.contains("the forward")),
        "{found:?}"
    );
    let found = check_grids(
        &source_with(|laser, which, data| {
            if (laser, which) != (1, 1) {
                return Some(data);
            }
            let ColumnData::F64(values) = &data.values else {
                unreachable!()
            };
            let narrowed: Vec<f32> = values.as_slice().iter().map(|v| *v as f32).collect();
            Some(GridData::new(dims(&[2, 3, 2]), ColumnData::F32(Shared::new(narrowed))).unwrap())
        }),
        &expected,
    );
    assert!(
        found.iter().any(|l| l.contains("the stored type")),
        "{found:?}"
    );
    let found = check_grids(
        &source_with(|laser, which, data| {
            if (laser, which) == (2, 0) {
                None
            } else {
                Some(data)
            }
        }),
        &expected,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("Laser_2") && found[0].contains("could not be read"),
        "{found:?}"
    );
}

#[test]
fn an_expectation_that_lists_no_grids_for_a_snapshot_is_reported_for_v16_and_v17_and_not_for_v18() {
    // a V16 or V17 expectation must list the grids of every laser that has a snapshot, so that a check cannot pass by having nothing to check
    let source = source_with(|_, _, data| Some(data));
    for version in [16, 17] {
        let lines = check_grids(&source, &expectation(version, false));
        assert_eq!(lines.len(), 1, "V{version}: {lines:?}");
        assert!(lines[0].contains("lists no grids"), "V{version}: {lines:?}");
    }
    // V18 has none yet, and so has none to check
    assert!(check_grids(&source, &expectation(18, false)).is_empty());
    assert!(grid_expectations(&expectation(18, false))
        .expect("none")
        .is_empty());
}

#[test]
fn the_expectations_of_the_files_describe_every_grid() {
    let all = load_all().expect("the expectation files");
    let (mut described, mut grids) = (0, 0);
    for expectation in &all {
        let list = grid_expectations(expectation)
            .unwrap_or_else(|e| panic!("{}: {e}", expectation.miniature()));
        if !list.is_empty() {
            described += 1;
            grids += list.len();
        }
    }
    // the six V17 files describe four grids each (the grid and its inverse, for two lasers) and the V16 file two; V18 describes none
    assert_eq!((described, grids), (7, 26));
}

// ---------------------------------------------------------------- the descriptors in the metadata, on the oracle's own files

fn expectation_of(file: &str) -> Expectation {
    load_all()
        .expect("the expectation files")
        .into_iter()
        .find(|e| e.miniature() == file)
        .expect("an expectation for the miniature")
}

/// What the parameters part says of the build made from `file`'s expectation after `change` is made to the first grid of laser `index`.
fn after(
    file: &str,
    index: usize,
    change: impl FnOnce(&mut build_log_model::CorrectionGrid),
) -> Vec<String> {
    let expectation = expectation_of(file);
    let mut meta = synthesise(&expectation).expect("a synthetic CDM");
    let mut laser = meta.lasers()[index].clone();
    change(&mut laser.parameters.as_mut().expect("a snapshot").grids[0]);
    meta.add_laser(laser);
    check_parts(&meta, &expectation, &[Part::Parameters])
}

#[test]
fn a_synthetic_build_gives_the_grids_each_expectation_lists_and_each_field_that_is_wrong_is_reported(
) {
    for file in ["v16.h5", "v17_mock.h5"] {
        let expectation = expectation_of(file);
        let meta = synthesise(&expectation).expect("a synthetic CDM");
        assert_eq!(
            check_parts(&meta, &expectation, &[Part::Parameters]),
            Vec::<String>::new(),
            "{file}"
        );
        // the grids are there: one for a V16 laser, two for a V17 laser
        let per_laser = if file == "v16.h5" { 1 } else { 2 };
        assert!(meta
            .lasers()
            .iter()
            .all(|l| l.parameters.as_ref().unwrap().grids.len() == per_laser));

        for (what, expected_in_line) in [
            ("its name", "its name"),
            (
                "which grid its name says it is",
                "which grid its name says it is",
            ),
            ("the type it is stored as", "the type it is stored as"),
            ("its extents", "its extents"),
        ] {
            let found = after(file, 0, |grid| match what {
                "its name" => grid.name = "another name".into(),
                "which grid its name says it is" => {
                    grid.role = match grid.role {
                        Some(GridRole::Forward) => Some(GridRole::Inverse),
                        _ => Some(GridRole::Forward),
                    }
                }
                "the type it is stored as" => grid.stored_as = NumericType::F32,
                _ => grid.extents = dims(&[257, 257, 3]),
            });
            assert_eq!(found.len(), 1, "{file}, {what}: {found:?}");
            assert!(
                found[0].contains(expected_in_line),
                "{file}, {what}: {found:?}"
            );
        }
    }
}

#[test]
fn a_grid_that_is_missing_or_extra_is_reported_by_its_count() {
    for file in ["v16.h5", "v17_mock.h5"] {
        let expectation = expectation_of(file);
        let mut meta = synthesise(&expectation).expect("a synthetic CDM");
        let mut laser = meta.lasers()[0].clone();
        let grids = &mut laser.parameters.as_mut().unwrap().grids;
        let extra = grids[0].clone();
        grids.push(extra);
        meta.add_laser(laser);
        let found = check_parts(&meta, &expectation, &[Part::Parameters]);
        assert!(
            found.iter().any(|l| l.contains("how many there are")),
            "{file}: {found:?}"
        );
        let mut laser = meta.lasers()[0].clone();
        laser.parameters.as_mut().unwrap().grids.clear();
        meta.add_laser(laser);
        let found = check_parts(&meta, &expectation, &[Part::Parameters]);
        assert!(
            found.iter().any(|l| l.contains("how many there are")),
            "{file}: {found:?}"
        );
        assert!(
            found.iter().any(|l| l.contains("not there, expected")),
            "{file}: {found:?}"
        );
    }
}
