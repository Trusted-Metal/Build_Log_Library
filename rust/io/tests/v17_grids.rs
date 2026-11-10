//! The correction grids of V17 (IP-24 phase 24B, task T24.2; decision D24-11): the descriptors that the metadata lists for each laser, and the values that the source returns for them.
//!
//! * `the_v17_grid_reads_equal_h5py`: each grid of each laser of each of the six miniatures, read through the facade's source in full as stored, equals the fingerprint that the independent script wrote
//!   (the count, the type, how many values are not a number, the minimum and the maximum, and the SHA-256 of the stored bytes), **and equals what the binding itself reads** from the optical train of
//!   the laser's number.
//! * **The grids are mostly NaN** (about 85% of each: what lies outside the field that was calibrated): the library returns them as stored, bit for bit, and does not repair them.
//! * `a_grid_is_a_descriptor_in_the_metadata_and_bulk_through_the_accessor`, `the_grid_descriptor_says_its_domain_is_unrecorded`: a snapshot lists its grids (the name as the file spells it, which grid the
//!   name says it is, the stored type and the extents) and the values come through `read_grid`.
//! * The rest are what V17 says about a request: a grid that the metadata does not list is `NotPresent` and the file is not touched, one that it lists and the file lacks is `Corrupt`, a grid that the engine
//!   does not read is reported on the snapshot and kept where it is, and a read can be cancelled.

mod common;

use build_log_conformance::{check_grids, grid_expectations, load_all, Expectation};
use build_log_io::adapters::v17::source::V17Source;
use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildMeta, CancelToken, ColumnData, ColumnSource, ErrorKind, GridInterpretation, GridRequest,
    GridRole, LaserKey, NodeKind, NotReadCause, NumericType, Origin, ReadContext,
};
use build_log_testkit::{miniature, miniature_path};
use common::{laser, layer, monitoring, put_text, temp_h5, Through};
use hdf5::File;

const SIX: [&str; 6] = [
    miniature::V17_MOCK,
    miniature::V17_OCT15,
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
    miniature::V17_MARCH,
    miniature::V17_SPECIAL_LAYERS,
];

fn expectation(file: &str) -> Expectation {
    load_all()
        .expect("the expectation files")
        .into_iter()
        .find(|e| e.miniature() == file)
        .expect("an expectation for the miniature")
}

fn laser_key(number: u32) -> LaserKey {
    LaserKey::new(format!("{}{number}", layout::LASER_PREFIX), number)
}

fn bits(values: &[f64]) -> Vec<u64> {
    values.iter().map(|v| v.to_bits()).collect()
}

fn train_path(number: u32, name: &str) -> String {
    format!(
        "{}/{}/{}{number}/{name}",
        layout::MOUNT,
        layout::CALIBRATION_DATA,
        layout::OPTICAL_TRAIN_PREFIX
    )
}

fn build_of(file: &str) -> BuildMeta {
    let opened = open(production(), miniature_path(file)).expect("the file opens");
    (*opened.source().build().expect("the metadata")).clone()
}

// ---------------------------------------------------------------- the six miniatures

#[test]
fn the_v17_grid_reads_equal_h5py() {
    let mut checked = 0;
    for name in SIX {
        let expectation = expectation(name);
        // the oracle describes both grids of each of the two lasers, and the check cannot pass by having nothing to check
        let described = grid_expectations(&expectation).expect("the grids");
        assert_eq!(described.len(), 4, "{name}");
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let found = check_grids(&Through(opened.source()), &expectation);
        assert!(
            found.is_empty(),
            "{name}: the grids do not match their fingerprints:\n  {}",
            found.join("\n  ")
        );
        // and what the binding itself reads from the optical train of the laser's number, bit for bit
        let file = File::open(miniature_path(name)).expect("the file");
        for wanted in &described {
            let got = opened
                .source()
                .read_grid(
                    &GridRequest::new(wanted.laser.clone(), wanted.name.clone()),
                    &ReadContext::new(),
                )
                .unwrap_or_else(|e| panic!("{name}: {} {}: {e}", wanted.laser.key, wanted.name));
            let direct = file
                .dataset(&train_path(wanted.laser.number, &wanted.name))
                .expect("the dataset")
                .read_raw::<f64>()
                .expect("the values");
            match &got.values {
                ColumnData::F64(values) => assert_eq!(
                    bits(values.as_slice()),
                    bits(&direct),
                    "{name}: {} {}",
                    wanted.laser.key,
                    wanted.name
                ),
                other => panic!("{name}: expected float64, got {other:?}"),
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 24);
}

#[test]
fn the_v17_grids_come_back_with_their_nan_as_stored() {
    // about 85% of each grid is NaN, and what the file holds is what comes back: nothing is filled in, zeroed or dropped
    let opened = open(production(), miniature_path(miniature::V17_MARCH)).expect("the file opens");
    let expectation = expectation(miniature::V17_MARCH);
    let described = grid_expectations(&expectation).expect("the grids");
    for wanted in &described {
        let got = opened
            .source()
            .read_grid(
                &GridRequest::new(wanted.laser.clone(), wanted.name.clone()),
                &ReadContext::new(),
            )
            .expect("the grid");
        let ColumnData::F64(values) = &got.values else {
            panic!("expected float64")
        };
        let not_a_number = values.as_slice().iter().filter(|v| v.is_nan()).count() as u64;
        assert_eq!(not_a_number, wanted.fingerprint.not_a_number);
        assert!(
            not_a_number * 100 / values.len() as u64 >= 80,
            "{} {}: {not_a_number} of {}",
            wanted.laser.key,
            wanted.name,
            values.len()
        );
        // the number of values is the product of the extents, whichever way the file orders them
        assert_eq!(got.extents.as_slice(), [257, 257, 2]);
        assert_eq!(values.len(), 257 * 257 * 2);
    }
}

#[test]
fn the_grid_of_a_laser_is_the_grid_of_the_optical_train_of_its_number() {
    // laser 1 and laser 2 have their own grids, which differ, and each is read from the train of its own number (decision D13-18)
    let opened = open(production(), miniature_path(miniature::V17_MOCK)).expect("the file opens");
    let read = |laser: u32, name: &str| {
        opened
            .source()
            .read_grid(
                &GridRequest::new(laser_key(laser), name),
                &ReadContext::new(),
            )
            .expect("the grid")
    };
    for name in [layout::CORRECTION_DATA, layout::INVERSE_CORRECTION_DATA] {
        let (one, two) = (read(1, name), read(2, name));
        assert_ne!(
            one.values, two.values,
            "{name}: two lasers give one grid: the pairing is by number, not by position"
        );
    }
    // and the forward grid of a laser is not its inverse
    assert_ne!(
        read(1, layout::CORRECTION_DATA).values,
        read(1, layout::INVERSE_CORRECTION_DATA).values
    );
}

#[test]
fn a_grid_is_a_descriptor_in_the_metadata_and_bulk_through_the_accessor() {
    let build = build_of(miniature::V17_TRIGGERED_2026_10_19);
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("the file opens");
    for laser in build.lasers() {
        let parameters = laser.parameters.as_ref().expect("a snapshot");
        // the descriptors are in the metadata, in the order the file names them, with nothing but what is recorded
        let described: Vec<(&str, Option<GridRole>, NumericType, Vec<u64>)> = parameters
            .grids
            .iter()
            .map(|g| {
                (
                    g.name.as_str(),
                    g.role,
                    g.stored_as,
                    g.extents.as_slice().to_vec(),
                )
            })
            .collect();
        assert_eq!(
            described,
            [
                (
                    layout::CORRECTION_DATA,
                    Some(GridRole::Forward),
                    NumericType::F64,
                    vec![257, 257, 2]
                ),
                (
                    layout::INVERSE_CORRECTION_DATA,
                    Some(GridRole::Inverse),
                    NumericType::F64,
                    vec![257, 257, 2]
                ),
            ],
            "{}",
            laser.key
        );
        // and the values are bulk, behind the accessor: as many as the extents hold, in the type that the descriptor says
        for grid in &parameters.grids {
            let got = opened
                .source()
                .read_grid(
                    &GridRequest::new(laser.key.clone(), grid.name.clone()),
                    &ReadContext::new(),
                )
                .expect("the grid");
            assert_eq!(got.extents, grid.extents);
            assert_eq!(got.len() as u64, grid.len());
            assert!(matches!(got.values, ColumnData::F64(_)));
        }
        // a grid is typed, so it is not an untyped node as well
        for grid in &parameters.grids {
            assert!(
                parameters
                    .extra
                    .node(std::slice::from_ref(&grid.name))
                    .is_none(),
                "{}",
                grid.name
            );
        }
    }
}

#[test]
fn the_grid_descriptor_says_its_domain_is_unrecorded() {
    // no file states whether a grid is in bits or in millimetres, or which way its axes run, and the library does not guess
    for name in SIX {
        for laser in build_of(name).lasers() {
            for grid in &laser.parameters.as_ref().expect("a snapshot").grids {
                assert_eq!(
                    grid.interpretation,
                    GridInterpretation::Unrecorded,
                    "{name} {}",
                    grid.name
                );
            }
        }
    }
}

// ---------------------------------------------------------------- what V17 says about a request

#[test]
fn a_grid_that_is_not_listed_is_not_present() {
    let opened = open(production(), miniature_path(miniature::V17_MOCK)).expect("the file opens");
    let not_present = |asked: GridRequest, what: &str| {
        let error = opened
            .source()
            .read_grid(&asked, &ReadContext::new())
            .expect_err(what);
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{what}: {error}"
        );
        assert_eq!(error.location().laser, Some(asked.laser.clone()), "{what}");
        assert_eq!(
            error.location().operation.as_deref(),
            Some("read_grid"),
            "{what}"
        );
    };
    not_present(
        GridRequest::new(laser_key(7), layout::CORRECTION_DATA),
        "a laser that the build does not have",
    );
    not_present(
        GridRequest::new(laser_key(1), "Not_A_Grid"),
        "a grid that the snapshot does not list",
    );
    // V16's name for its grid is not a name of V17's
    not_present(
        GridRequest::new(laser_key(1), "CorrectionData"),
        "the name that another version uses",
    );
}

/// A build made at test time whose first laser has the optical train 1, which holds the grids that `write` makes; the source over it, and the file kept open for the test.
fn source_over(
    label: &str,
    write: impl FnOnce(&hdf5::Group),
) -> (common::TempH5, File, V17Source, BuildMeta) {
    let on_disk = temp_h5(label, |mount| {
        put_text(mount, layout::BUILD_ID, "a build");
        laser(&layer(&monitoring(mount), 1), 1, 6, None);
        let train = mount
            .create_group(layout::CALIBRATION_DATA)
            .and_then(|c| c.create_group(&format!("{}1", layout::OPTICAL_TRAIN_PREFIX)))
            .expect("the optical train");
        write(&train);
    });
    let file = File::open_rw(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let build = v17::read::read_build(&mount).expect("the build reads");
    let source = V17Source::new(build.clone(), mount);
    (on_disk, file, source, build)
}

fn grid_of(train: &hdf5::Group, name: &str, extents: &[usize], values: &[f64]) {
    let dataset = train
        .new_dataset::<f64>()
        .shape(extents.to_vec())
        .create(name)
        .expect("a grid");
    dataset.write_raw(values).expect("written");
}

#[test]
fn a_grid_that_the_metadata_lists_and_the_file_lacks_is_corrupt() {
    // the metadata is read while the file has the grid, and then the grid is taken out, so the metadata promises what the file does not hold
    let stored = [0.5, f64::NAN, -2.0, 4.0, 8.0, f64::NAN];
    let (_disk, file, source, build) = source_over("v17-grid-missing", |train| {
        grid_of(train, layout::CORRECTION_DATA, &[1, 3, 2], &stored);
    });
    assert_eq!(
        build
            .laser(&laser_key(1))
            .unwrap()
            .parameters
            .as_ref()
            .unwrap()
            .grids
            .len(),
        1
    );
    let ok = source
        .read_grid(
            &GridRequest::new(laser_key(1), layout::CORRECTION_DATA),
            &ReadContext::new(),
        )
        .expect("the grid is there");
    match &ok.values {
        ColumnData::F64(values) => assert_eq!(bits(values.as_slice()), bits(&stored)),
        other => panic!("{other:?}"),
    }
    file.group(&format!(
        "Mount/{}/{}1",
        layout::CALIBRATION_DATA,
        layout::OPTICAL_TRAIN_PREFIX
    ))
    .expect("the train")
    .unlink(layout::CORRECTION_DATA)
    .expect("the grid is removed");
    let error = source
        .read_grid(
            &GridRequest::new(laser_key(1), layout::CORRECTION_DATA),
            &ReadContext::new(),
        )
        .expect_err("the file does not hold it");
    match error.kind() {
        ErrorKind::Corrupt { detail } => assert!(detail.contains("no dataset"), "{detail}"),
        other => panic!("expected Corrupt, got {other:?}"),
    }
    assert_eq!(error.location().laser, Some(laser_key(1)));
}

#[test]
fn a_grid_that_the_engine_does_not_read_is_reported_on_the_snapshot_and_kept_where_it_is() {
    // the forward grid is a table of four dimensions, and the inverse is fine: the snapshot lists the inverse, says why it does not list the other, and keeps it as an untyped node; the build opens
    let (_disk, _file, _source, build) = source_over("v17-grid-damaged", |train| {
        grid_of(train, layout::CORRECTION_DATA, &[1, 1, 1, 2], &[1.0, 2.0]);
        grid_of(
            train,
            layout::INVERSE_CORRECTION_DATA,
            &[2, 2],
            &[1.0, 2.0, 3.0, 4.0],
        );
    });
    let parameters = build
        .laser(&laser_key(1))
        .unwrap()
        .parameters
        .as_ref()
        .unwrap();
    let names: Vec<&str> = parameters.grids.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, [layout::INVERSE_CORRECTION_DATA]);
    match parameters
        .origins
        .get(&format!("grid {}", layout::CORRECTION_DATA))
    {
        Some(Origin::NotRead { cause, reason }) => {
            // a grid of a shape that the engine does not read is not supported, which is not a fault of the file
            assert_eq!(*cause, NotReadCause::Unsupported);
            assert!(reason.contains("dimensions"), "{reason}");
        }
        other => panic!("expected a reason, got {other:?}"),
    }
    let kept = parameters
        .extra
        .node(&[layout::CORRECTION_DATA.to_string()])
        .expect("the grid that could not be described is kept");
    assert_eq!(kept.kind, NodeKind::Dataset);
    assert_eq!(kept.dims.as_slice(), [1, 1, 1, 2]);
}

#[test]
fn a_train_with_no_grid_lists_none_and_says_nothing() {
    let (_disk, _file, source, build) = source_over("v17-grid-none", |_| {});
    let parameters = build
        .laser(&laser_key(1))
        .unwrap()
        .parameters
        .as_ref()
        .unwrap();
    assert!(parameters.grids.is_empty());
    assert!(parameters
        .origins
        .iter()
        .all(|(name, _)| !name.starts_with("grid ")));
    let error = source
        .read_grid(
            &GridRequest::new(laser_key(1), layout::CORRECTION_DATA),
            &ReadContext::new(),
        )
        .expect_err("not listed");
    assert!(matches!(error.kind(), ErrorKind::NotPresent { .. }));
}

#[test]
fn a_grid_read_can_be_cancelled_before_it_starts() {
    let opened = open(production(), miniature_path(miniature::V17_MOCK)).expect("the file opens");
    let token = CancelToken::new();
    token.cancel();
    let error = opened
        .source()
        .read_grid(
            &GridRequest::new(laser_key(1), layout::CORRECTION_DATA),
            &ReadContext::new().with_cancel(token),
        )
        .expect_err("cancelled");
    assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error}");
    // and the same grid reads when it is not
    assert!(opened
        .source()
        .read_grid(
            &GridRequest::new(laser_key(1), layout::CORRECTION_DATA),
            &ReadContext::new()
        )
        .is_ok());
}
