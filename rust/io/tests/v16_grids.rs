//! The correction grid of V16 (IP-24 phase 24B, task T24.2; decision D24-11; the owner's answer to Q24-6, 2026-11-03: **both versions**): the descriptor that the metadata lists for each laser, and the
//! values that the source returns for it.
//!
//! * `the_v16_grid_reads_equal_h5py`: the grid of each laser of the miniature, read through the facade's source in full as stored, equals the fingerprint that the independent script wrote (the count,
//!   the type, how many values are not a number, the minimum and the maximum, and the SHA-256 of the stored bytes), **and equals what the binding itself reads** from the laser's own calibration.
//! * **V16's grid is not V17's**: a calibration holds one dataset, `int32` of 2 by 257 by 257, and its name does not say whether it is the grid or its inverse, so the descriptor has no role. The model
//!   holds an element type and a list of extents, and neither shape is its own.
//! * The rest are what V16 says about a request: a grid that the metadata does not list is `NotPresent` and the file is not touched, one that it lists and the file lacks is `Corrupt`, a grid that the
//!   engine does not read is reported on the snapshot and kept where it is, and a read can be cancelled.

mod common;

use std::sync::Arc;

use build_log_conformance::{check_grids, grid_expectations, load_all, Expectation};
use build_log_io::adapters::v16::columns::V16Locator;
use build_log_io::adapters::v16::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_io::toolkit::source::SampleSource;
use build_log_model::{
    BuildMeta, CancelToken, ColumnData, ColumnSource, ErrorKind, GridInterpretation, GridRequest,
    LaserKey, NodeKind, NotReadCause, NumericType, Origin, ReadContext,
};
use build_log_testkit::{miniature, miniature_path};
use common::v16::{calibration, laser, layer, monitoring, mount_file};
use common::{put_text, Through};
use hdf5::File;

fn expectation() -> Expectation {
    load_all()
        .expect("the expectation files")
        .into_iter()
        .find(|e| e.miniature() == miniature::V16)
        .expect("the V16 expectation")
}

fn laser_key(number: u32) -> LaserKey {
    LaserKey::new(format!("{}{number}", layout::LASER_PREFIX), number)
}

fn calibration_path(number: u32, name: &str) -> String {
    format!(
        "{}/{}/{}{number}/{name}",
        layout::MOUNT,
        layout::CALIBRATION_DATA,
        layout::CALIBRATION_PREFIX
    )
}

fn build_of_the_miniature() -> BuildMeta {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    (*opened.source().build().expect("the metadata")).clone()
}

// ---------------------------------------------------------------- the miniature

#[test]
fn the_v16_grid_reads_equal_h5py() {
    let expectation = expectation();
    // the oracle describes the grid of each of the two lasers, and the check cannot pass by having nothing to check
    let described = grid_expectations(&expectation).expect("the grids");
    assert_eq!(described.len(), 2);
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    let found = check_grids(&Through(opened.source()), &expectation);
    assert!(
        found.is_empty(),
        "the grids do not match their fingerprints:\n  {}",
        found.join("\n  ")
    );
    // and what the binding itself reads from the laser's own calibration: whole numbers, in the order the file stores them
    let file = File::open(miniature_path(miniature::V16)).expect("the file");
    for wanted in &described {
        let got = opened
            .source()
            .read_grid(
                &GridRequest::new(wanted.laser.clone(), wanted.name.clone()),
                &ReadContext::new(),
            )
            .expect("the grid");
        let direct = file
            .dataset(&calibration_path(wanted.laser.number, &wanted.name))
            .expect("the dataset")
            .read_raw::<i32>()
            .expect("the values");
        match &got.values {
            ColumnData::I32(values) => assert_eq!(values.as_slice(), direct.as_slice()),
            other => panic!("expected int32, got {other:?}"),
        }
        assert_eq!(got.extents.as_slice(), [2, 257, 257]);
    }
}

#[test]
fn a_grid_is_a_descriptor_in_the_metadata_and_bulk_through_the_accessor() {
    let build = build_of_the_miniature();
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    for laser in build.lasers() {
        let parameters = laser.parameters.as_ref().expect("a snapshot");
        // one grid, by the name the file spells, with no role (the name does not say which grid it is), `int32` of 2 by 257 by 257
        assert_eq!(parameters.grids.len(), 1, "{}", laser.key);
        let grid = &parameters.grids[0];
        assert_eq!(grid.name, layout::CORRECTION_DATA);
        assert_eq!(grid.role, None);
        assert_eq!(grid.stored_as, NumericType::I32);
        assert_eq!(grid.extents.as_slice(), [2, 257, 257]);
        // the values are bulk, behind the accessor: as many as the extents hold, in the type that the descriptor says
        let got = opened
            .source()
            .read_grid(
                &GridRequest::new(laser.key.clone(), grid.name.clone()),
                &ReadContext::new(),
            )
            .expect("the grid");
        assert_eq!(got.extents, grid.extents);
        assert_eq!(got.len() as u64, grid.len());
        assert!(matches!(got.values, ColumnData::I32(_)));
        // a grid is typed, so it is not an untyped node as well
        assert!(parameters
            .extra
            .node(std::slice::from_ref(&grid.name))
            .is_none());
    }
}

#[test]
fn the_grid_descriptor_says_its_domain_is_unrecorded() {
    // no file states whether a grid is in bits or in millimetres, or which way its axes run, and the library does not guess
    for laser in build_of_the_miniature().lasers() {
        for grid in &laser.parameters.as_ref().expect("a snapshot").grids {
            assert_eq!(grid.interpretation, GridInterpretation::Unrecorded);
        }
    }
}

// ---------------------------------------------------------------- what V16 says about a request

#[test]
fn a_grid_that_is_not_listed_is_not_present() {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
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
    // V17's names for its grids are not names of V16's
    not_present(
        GridRequest::new(laser_key(1), "Correction_Data"),
        "a name that another version uses",
    );
    not_present(
        GridRequest::new(laser_key(2), "Inverse_Correction_Data"),
        "a name that another version uses",
    );
}

type V16Source = SampleSource<V16Locator>;

fn source_from(mount: &hdf5::Group) -> (V16Source, BuildMeta) {
    let build = Arc::new(v16::read::read_build(mount).expect("the build reads"));
    let locator = V16Locator::new(Arc::clone(&build), mount.clone());
    (
        SampleSource::new(Arc::clone(&build), locator),
        (*build).clone(),
    )
}

fn grid_of<T: hdf5::H5Type>(calibration: &hdf5::Group, extents: &[usize], values: &[T]) {
    let dataset = calibration
        .new_dataset::<T>()
        .shape(extents.to_vec())
        .create(layout::CORRECTION_DATA)
        .expect("a grid");
    dataset.write_raw(values).expect("written");
}

#[test]
fn the_grid_of_a_laser_is_the_grid_of_its_own_calibration() {
    // two lasers with their own grids, whose calibrations the file made in the other order: each laser reads the grid of the calibration of its own number
    let on_disk = mount_file("v16-grid-own", |mount| {
        let first = layer(&monitoring(mount), 1);
        laser(&first, 1, 3);
        laser(&first, 2, 3);
        grid_of(&calibration(mount, 2), &[2, 2], &[20i32, 21, 22, 23]);
        grid_of(&calibration(mount, 1), &[2, 2], &[10i32, 11, 12, 13]);
    });
    let file = File::open(&on_disk.0).expect("the file opens");
    let (source, build) = source_from(&file.group("Mount").expect("the mount"));
    for (number, expected) in [(1u32, vec![10i32, 11, 12, 13]), (2, vec![20, 21, 22, 23])] {
        assert_eq!(
            build
                .laser(&laser_key(number))
                .unwrap()
                .parameters
                .as_ref()
                .unwrap()
                .grids
                .len(),
            1
        );
        let got = source
            .read_grid(
                &GridRequest::new(laser_key(number), layout::CORRECTION_DATA),
                &ReadContext::new(),
            )
            .expect("the grid");
        match got.values {
            ColumnData::I32(values) => {
                assert_eq!(values.as_slice(), expected.as_slice(), "laser {number}")
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_grid_that_the_metadata_lists_and_the_file_lacks_is_corrupt() {
    // the metadata is read while the file has the grid, and then the grid is taken out, so the metadata promises what the file does not hold
    let on_disk = mount_file("v16-grid-missing", |mount| {
        laser(&layer(&monitoring(mount), 1), 1, 3);
        grid_of(&calibration(mount, 1), &[1, 2], &[5i32, 6]);
    });
    let file = File::open_rw(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let (source, build) = source_from(&mount);
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
    mount
        .group(&format!(
            "{}/{}1",
            layout::CALIBRATION_DATA,
            layout::CALIBRATION_PREFIX
        ))
        .expect("the calibration")
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
    // the grid is a table of four dimensions: the snapshot lists none, says why, and keeps the dataset as an untyped node; the calibration is still read and the build opens
    let build = common::v16::with_lasers("v16-grid-damaged", &[1], |mount| {
        let group = calibration(mount, 1);
        put_text(&group, layout::LASER_MODEL, "a laser");
        grid_of(&group, &[1, 1, 1, 2], &[1.0f64, 2.0]);
    })
    .expect("the build opens");
    let parameters = build
        .laser(&laser_key(1))
        .unwrap()
        .parameters
        .as_ref()
        .unwrap();
    assert!(parameters.grids.is_empty());
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
    // and the values of the calibration are read as before
    assert!(parameters
        .values()
        .iter()
        .any(|(name, _)| name == "light_source.model"));
}

#[test]
fn a_calibration_with_no_grid_lists_none_and_says_nothing() {
    let build = common::v16::with_lasers("v16-grid-none", &[1, 2], |mount| {
        put_text(&calibration(mount, 1), layout::LASER_MODEL, "a laser");
    })
    .expect("the build opens");
    let first = build
        .laser(&laser_key(1))
        .unwrap()
        .parameters
        .as_ref()
        .unwrap();
    assert!(first.grids.is_empty());
    assert!(first
        .origins
        .iter()
        .all(|(name, _)| !name.starts_with("grid ")));
    // a laser with no calibration has no snapshot, and so no grid to ask for
    assert!(build.laser(&laser_key(2)).unwrap().parameters.is_none());
}

#[test]
fn a_grid_read_can_be_cancelled_before_it_starts() {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
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
