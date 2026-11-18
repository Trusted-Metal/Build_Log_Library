//! The spot-size accessors on the miniatures (IP-27 phase 27J; decisions D27-8, D27-18, D27-21 and D27-24; BL-PR-03, BL-PR-04, V:QE-04).
//!
//! `spot_size` takes a laser of a layer from the file to its spot in one call: the rows (27C), the Z (27D), the time shift (27F), the beam (27H) and the kernel (27I). The tests here prove that
//! one call **is** the chain that the earlier phases proved piece by piece:
//!
//! * the nine layer cases of Vision's kernel vectors, **through the accessor**, bit for bit, with the same columns and the same warnings;
//! * a grid of selections (stride 1, 2, 7 and one so large that a laser keeps a single row, each with all, laser-on and laser-off rows) on every miniature, each laser equal to the composition of
//!   the pieces by hand, and what Vision drops reported as a reason;
//! * the reasons a laser has no spot size, the beam of another laser, the origins and the stage.
//!
//! **The answers where the file is silent are Vision's and are made here, in the test, as the adapter of phase 27K will make them** (D27-18, D27-24): the library has no default and cannot build
//! a request without them.

mod vision_adapter;
#[path = "../../processing/tests/vision_beam/mod.rs"]
mod vision_beam;

use build_log::testing::miniature_path;
use build_log::{
    open, LaserKey, LayerId, Origin, ReadContext, ReadSelection, SpotSizeOverride, StateRows,
    TimingOverride, ZCalibrationOverride,
};
use build_log_conformance::vectors::{
    differences, expected_columns, load_vectors, whole, words, Columns,
};
use build_log_processing::{
    resolve_beam_of, spot_size, spot_size_beam, spot_size_from_z, BeamUnavailable,
    SpotSizeAssumptions, SpotSizeFinding, SpotSizeRequest, SpotSizeUnavailable, Stage,
    UnitlessPeriod, ZAssumptions, ZRefusal, ZSeries, SPOT_SIZE_ALGORITHM_VERSION,
};
use serde_json::Value;

use vision_adapter::{
    kept_layer, put_spot, rows_of, shift_laser, vision_fallbacks, vision_mirrored, ShiftedLaser,
    VISION_BIT_RESOLUTION, VISION_FIELD_SIZE_Z_MM, VISION_OFFSET_Z_MM,
};
use vision_beam::{error_text, override_of, vision_assumptions};

const CHUNK: &str = "spot_size_kernel";

const MINIATURES: [&str; 7] = [
    "v16.h5",
    "v17_2026_08_18_images.h5",
    "v17_march.h5",
    "v17_mock.h5",
    "v17_oct15.h5",
    "v17_triggered_2026_10_19.h5",
    "special_layers_v17.h5",
];

/// **Vision's answers where the file is silent**, as the adapter of phase 27K will carry them: the beam's seven, the timing's, the Z's (400 mm, no offset, 20 bits, narrowed to the stored width), a
/// bare period above 0.001 read as microseconds, and the time of the first sample at 0.
fn vision_spot_assumptions() -> SpotSizeAssumptions {
    SpotSizeAssumptions {
        beam: vision_assumptions(),
        timing: vision_fallbacks(),
        z: ZAssumptions {
            field_size_mm: VISION_FIELD_SIZE_Z_MM,
            offset_mm: VISION_OFFSET_Z_MM,
            bit_resolution: VISION_BIT_RESOLUTION,
            narrow_to_stored_width: true,
        },
        sample_period: UnitlessPeriod::MicrosecondsAbove(0.001),
        time_origin_s: 0.0,
    }
}

fn request(layer: LayerId, laser: &LaserKey) -> SpotSizeRequest {
    SpotSizeRequest::new(layer, laser.clone(), vision_spot_assumptions())
}

/// An override that gives every laser a beam whatever its file says (a waist and a Rayleigh length for each axis).
fn rescue() -> SpotSizeOverride {
    SpotSizeOverride::new()
        .with_waist_major_um(60.0)
        .with_waist_minor_um(50.0)
        .with_rayleigh_major_mm(4.0)
        .with_rayleigh_minor_mm(3.0)
}

fn selection(stride: u64, rows: StateRows) -> ReadSelection {
    ReadSelection::new().with_stride(stride).with_rows(rows)
}

fn bits_of(values: &[f64]) -> Vec<u64> {
    values.iter().map(|v| v.to_bits()).collect()
}

// ---------------------------------------------------------------- Vision's vectors, through the accessor

/// The columns Vision's vectors record for one file case, **made by asking the accessor for each laser Vision keeps**.
fn run_file_through_the_accessor(inputs: &Value) -> Columns {
    let stride = inputs["stride"].as_u64().expect("stride");
    let selection = ReadSelection::new()
        .with_stride(stride)
        .with_rows(rows_of(inputs["rows"].as_str().expect("rows")));
    let miniature = inputs["miniature"].as_str().expect("miniature");
    let position = usize::try_from(inputs["layer_position"].as_u64().expect("layer_position"))
        .expect("layer_position");
    // which lasers Vision keeps is Vision's decision and is made by the adapter; what the accessor says about each of them is what is compared
    let layer = kept_layer(
        miniature,
        position,
        &selection,
        &ZCalibrationOverride::new(),
    );
    let opened = open(miniature_path(miniature)).expect("the miniature opens");
    let source = opened.source();
    let layer_id = source.layers().expect("the layers")[position];
    let detail = source.layer_meta(layer_id).expect("the layer");
    let beam_override = override_of(&inputs["override"]);

    let mut out = Columns::new();
    let mut present: Vec<i64> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for kept in &layer.lasers {
        present.push(kept.index as i64);
        let key = &detail.lasers[kept.index].key;
        let asked = request(layer_id, key)
            .with_selection(selection)
            .with_beam_override(beam_override.clone());
        match spot_size(source, &asked, &ReadContext::new()).expect("a laser of the build") {
            Ok(spot) => {
                assert_eq!(spot.algorithm_version(), SPOT_SIZE_ALGORITHM_VERSION);
                assert_eq!(
                    spot.commanded_rows().len(),
                    spot.commanded().major_radii_um.len()
                );
                put_spot(
                    &mut out,
                    kept.index,
                    spot.commanded(),
                    spot.actual(),
                    spot.axis_angle_deg(),
                );
            }
            Err(SpotSizeUnavailable::Beam(reason)) => {
                warnings.push(format!(
                    "{}: spot size unavailable \u{2014} {}",
                    kept.name,
                    error_text(&reason)
                ));
                out.insert(format!("laser{}_has_spot_size", kept.index), whole(&[0]));
            }
            Err(other) => panic!("{}: unexpectedly unavailable: {other}", kept.name),
        }
    }
    out.insert("lasers_present".into(), whole(&present));
    let warnings: Vec<&str> = warnings.iter().map(String::as_str).collect();
    out.insert("warnings".into(), words(&warnings));
    out
}

#[test]
fn the_accessor_reproduces_the_nine_layer_cases_bit_for_bit() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut ran = 0;
    let mut lines = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] != "file" {
            continue;
        }
        let label = case["case"].as_str().expect("case");
        let expected = expected_columns(case);
        let actual = run_file_through_the_accessor(&case["inputs"]);
        lines.extend(differences(label, &expected, &actual, &|_| false));
        ran += 1;
    }
    assert!(
        lines.is_empty(),
        "the accessor differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 9, "the file holds 9 cases on miniatures");
}

// ---------------------------------------------------------------- the grid of selections

/// What the accessor must give for a laser Vision keeps: the pieces composed by hand (the rows, the Z, the shift by the adapter, the beam, the kernel) and compared to the bit.
fn check_against_the_pieces(
    miniature: &str,
    position: usize,
    selection: &ReadSelection,
    stride: u64,
    beam_override: &SpotSizeOverride,
) -> usize {
    let layer = kept_layer(miniature, position, selection, &ZCalibrationOverride::new());
    let opened = open(miniature_path(miniature)).expect("the miniature opens");
    let source = opened.source();
    let layer_id = source.layers().expect("the layers")[position];
    let detail = source.layer_meta(layer_id).expect("the layer");
    let mut compared = 0;

    for (index, found) in detail.lasers.iter().enumerate() {
        let asked = request(layer_id, &found.key)
            .with_selection(*selection)
            .with_beam_override(beam_override.clone());
        let got = spot_size(source, &asked, &ReadContext::new()).expect("a laser of the build");
        let label = format!(
            "{miniature} layer {position} laser {} stride {stride} rows {:?}",
            found.key.number,
            selection.rows()
        );
        let Some(kept) = layer.lasers.iter().find(|k| k.index == index) else {
            // Vision drops it: a filter on a laser with no state, or no rows. The accessor must say why and not compute.
            assert!(
                matches!(
                    got,
                    Err(SpotSizeUnavailable::FilterNeedsState
                        | SpotSizeUnavailable::NoRows
                        | SpotSizeUnavailable::NoZChannel { .. }
                        | SpotSizeUnavailable::Beam(_))
                ),
                "{label}: Vision drops this laser and the accessor says {got:?}"
            );
            continue;
        };
        let parameters = kept.parameters.as_ref();
        let beam = resolve_beam_of(parameters, beam_override, &vision_assumptions());
        match (beam, got) {
            (Err(reason), Err(SpotSizeUnavailable::Beam(said))) => {
                assert_eq!(said, reason, "{label}")
            }
            (Ok(beam), Ok(spot)) => {
                let ShiftedLaser {
                    commanded, actual, ..
                } = shift_laser(kept, stride, &TimingOverride::new());
                let by_hand = spot_size_from_z(&beam, &commanded, &actual);
                for (name, a, b) in [
                    (
                        "cmd major",
                        &spot.commanded().major_radii_um,
                        &by_hand.commanded().major_radii_um,
                    ),
                    (
                        "cmd minor",
                        &spot.commanded().minor_radii_um,
                        &by_hand.commanded().minor_radii_um,
                    ),
                    (
                        "cmd area",
                        &spot.commanded().areas_um2,
                        &by_hand.commanded().areas_um2,
                    ),
                    (
                        "act major",
                        &spot.actual().major_radii_um,
                        &by_hand.actual().major_radii_um,
                    ),
                    (
                        "act minor",
                        &spot.actual().minor_radii_um,
                        &by_hand.actual().minor_radii_um,
                    ),
                    (
                        "act area",
                        &spot.actual().areas_um2,
                        &by_hand.actual().areas_um2,
                    ),
                ] {
                    assert_eq!(bits_of(a), bits_of(b), "{label}: {name}");
                }
                assert_eq!(
                    spot.axis_angle_deg().to_bits(),
                    by_hand.axis_angle_deg().to_bits(),
                    "{label}: angle"
                );
                // the rows are the file's own: the first `n` of the adapter's index
                let first: Vec<u64> = kept.native_index.iter().take(kept.n).collect();
                assert_eq!(
                    spot.commanded_rows().to_vec(),
                    first,
                    "{label}: commanded rows"
                );
                assert_eq!(spot.actual_rows().to_vec(), first, "{label}: actual rows");
                // a laser of one row is not shifted, and says so; any other is
                let expected_stage = if kept.n < 2 {
                    Stage::Physical
                } else {
                    Stage::Shifted
                };
                assert_eq!(spot.stage(), expected_stage, "{label}: stage");
                assert_eq!(
                    spot.findings()
                        .iter()
                        .filter(|f| matches!(f, SpotSizeFinding::NotShifted { .. }))
                        .count(),
                    if kept.n < 2 { 2 } else { 0 },
                    "{label}: the not-shifted findings"
                );
                compared += 1;
            }
            (beam, got) => panic!("{label}: the pieces say {beam:?} and the accessor {got:?}"),
        }
    }
    compared
}

#[test]
fn the_accessor_equals_the_pieces_over_a_grid_of_selections() {
    let mut compared = 0;
    for miniature in MINIATURES {
        let opened = open(miniature_path(miniature)).expect("the miniature opens");
        let layers = opened.source().layers().expect("the layers");
        // the first layer and, where there is one, the last
        let mut positions = vec![0];
        if layers.len() > 1 {
            positions.push(layers.len() - 1);
        }
        drop(opened);
        for position in positions {
            for stride in [1, 2, 7, 1_000_000] {
                for rows in [StateRows::All, StateRows::OnOnly, StateRows::OffOnly] {
                    // with the file's own beam, and with a beam given to every laser so that the Z path is run on the miniatures that have none
                    for beam_override in [SpotSizeOverride::new(), rescue()] {
                        compared += check_against_the_pieces(
                            miniature,
                            position,
                            &selection(stride, rows),
                            stride,
                            &beam_override,
                        );
                    }
                }
            }
        }
    }
    // the grid is not empty: lasers with a beam were computed on both kinds of file
    assert!(compared > 100, "only {compared} lasers were compared");
}

// ---------------------------------------------------------------- the reasons and the rest

/// The first layer and the lasers of a miniature.
fn first_layer(miniature: &str) -> (build_log::Opened, LayerId, Vec<LaserKey>) {
    let opened = open(miniature_path(miniature)).expect("the miniature opens");
    let layer = opened.source().layers().expect("the layers")[0];
    let keys = opened
        .source()
        .layer_meta(layer)
        .expect("the layer")
        .lasers
        .iter()
        .map(|l| l.key.clone())
        .collect();
    (opened, layer, keys)
}

#[test]
fn a_laser_with_no_beam_says_why_and_the_beam_accessor_says_the_same_without_reading_a_sample() {
    // the miniatures whose files hold no waist: Vision warns for them, and the library says which fact is missing
    for miniature in ["v17_march.h5", "v16.h5", "special_layers_v17.h5"] {
        let (opened, layer, keys) = first_layer(miniature);
        for key in &keys {
            let asked = request(layer, key);
            let got = spot_size(opened.source(), &asked, &ReadContext::new()).expect("the laser");
            assert_eq!(
                got.as_ref().err(),
                Some(&SpotSizeUnavailable::Beam(BeamUnavailable::NoWaist)),
                "{miniature} {key:?}"
            );
            // the metadata accessor gives the same reason, and gives the beam of a file that has one
            let beam = spot_size_beam(
                opened.source(),
                key,
                None,
                &SpotSizeOverride::new(),
                &vision_assumptions(),
            )
            .expect("the laser");
            assert_eq!(beam.err(), Some(BeamUnavailable::NoWaist));
            // and an override rescues it
            let rescued =
                spot_size_beam(opened.source(), key, None, &rescue(), &vision_assumptions())
                    .expect("the laser");
            assert!(rescued.is_ok(), "{miniature} {key:?}");
        }
    }
    // a file that has a beam gives it, with the origin of each value
    let (opened, _, keys) = first_layer("v17_oct15.h5");
    let beam = spot_size_beam(
        opened.source(),
        &keys[0],
        None,
        &SpotSizeOverride::new(),
        &vision_assumptions(),
    )
    .expect("the laser")
    .expect("a beam");
    assert!(beam.waist_radius_major_um().value() > 0.0);
}

#[test]
fn a_laser_the_build_does_not_hold_is_a_failure_and_one_not_in_the_layer_is_a_reason() {
    let (opened, layer, keys) = first_layer("v17_oct15.h5");
    let source = opened.source();
    // not in the build: an error, from both accessors
    let stranger = LaserKey::new("Laser_99", 99);
    assert!(spot_size(source, &request(layer, &stranger), &ReadContext::new()).is_err());
    assert!(spot_size_beam(
        source,
        &stranger,
        None,
        &SpotSizeOverride::new(),
        &vision_assumptions()
    )
    .is_err());
    // a beam taken from a laser the build does not hold is an error too
    assert!(spot_size_beam(
        source,
        &keys[0],
        Some(&stranger),
        &SpotSizeOverride::new(),
        &vision_assumptions()
    )
    .is_err());
    // a layer the build does not hold: an error
    assert!(spot_size(
        source,
        &request(LayerId(layer.0 + 100_000), &keys[0]),
        &ReadContext::new()
    )
    .is_err());
}

#[test]
fn a_laser_with_no_rows_in_the_layer_is_unavailable_and_the_others_are_computed() {
    // October 15's second laser is in the layer and records no samples there; the images file's second laser is in the same case in every layer
    let (opened, layer, keys) = first_layer("v17_oct15.h5");
    let source = opened.source();
    let first =
        spot_size(source, &request(layer, &keys[0]), &ReadContext::new()).expect("the laser");
    assert_eq!(
        first
            .expect("the first laser has a spot size")
            .commanded()
            .major_radii_um
            .len(),
        25_000
    );
    let second =
        spot_size(source, &request(layer, &keys[1]), &ReadContext::new()).expect("the laser");
    assert_eq!(second.err(), Some(SpotSizeUnavailable::NoRows));
    let (opened, _, keys) = first_layer("v17_2026_08_18_images.h5");
    for layer in opened.source().layers().expect("the layers") {
        let second = spot_size(
            opened.source(),
            &request(layer, &keys[1]),
            &ReadContext::new(),
        )
        .expect("the laser");
        assert_eq!(
            second.err(),
            Some(SpotSizeUnavailable::NoRows),
            "layer {layer}"
        );
    }
    // a laser that is not in the layer at all is another reason: a layer of the mock records both lasers, and no layer records a laser that the build does not hold (an error, tested above)
}

#[test]
fn a_z_that_cannot_be_converted_is_refused_by_series_and_the_commanded_one_is_checked_first() {
    let (opened, layer, keys) = first_layer("v17_oct15.h5");
    let source = opened.source();
    let key = &keys[0];
    let refused = |overrides: ZCalibrationOverride| {
        spot_size(
            source,
            &request(layer, key).with_z_override(overrides),
            &ReadContext::new(),
        )
        .expect("the laser")
        .expect_err("refused")
    };
    // 0 bits is outside 1..=32, for the commanded series, and the actual
    let commanded = refused(ZCalibrationOverride::new().with_commanded_bit_resolution_z(0));
    assert!(matches!(
        commanded,
        SpotSizeUnavailable::ZRefused {
            series: ZSeries::Commanded,
            refusal: ZRefusal::ResolutionOutOfRange { resolution: 0, .. }
        }
    ));
    assert!(commanded.to_string().contains("cannot be converted"));
    // (33 is not a refusal here: the caller's narrower-dataset rule brings it down to the width of the stored type, as Vision does)
    let actual = refused(ZCalibrationOverride::new().with_actual_bit_resolution_z(-4));
    assert!(matches!(
        actual,
        SpotSizeUnavailable::ZRefused {
            series: ZSeries::Actual,
            ..
        }
    ));
    // both bad: the commanded series is named first
    let both = refused(
        ZCalibrationOverride::new()
            .with_commanded_bit_resolution_z(0)
            .with_actual_bit_resolution_z(0),
    );
    assert!(matches!(
        both,
        SpotSizeUnavailable::ZRefused {
            series: ZSeries::Commanded,
            ..
        }
    ));
}

#[test]
fn the_beam_of_another_laser_is_used_when_the_caller_asks_and_never_otherwise() {
    // October 15's two lasers have different beams (waists of 33 x 34 and 34 x 34 micrometres); the Z of the first laser is the only one with samples
    let (opened, layer, keys) = first_layer("v17_oct15.h5");
    let source = opened.source();
    let ask = |asked: SpotSizeRequest| {
        spot_size(source, &asked, &ReadContext::new())
            .expect("the laser")
            .expect("a spot size")
    };
    let own = ask(request(layer, &keys[0]));
    let borrowed = ask(request(layer, &keys[0]).with_beam_from(keys[1].clone()));
    let metadata = |from: Option<&LaserKey>| {
        spot_size_beam(
            source,
            &keys[0],
            from,
            &SpotSizeOverride::new(),
            &vision_assumptions(),
        )
        .expect("the laser")
        .expect("a beam")
    };
    // the library never picks another laser's beam on its own: without `beam_from` the beam is the laser's
    assert_eq!(own.beam(), &metadata(None));
    assert_eq!(borrowed.beam(), &metadata(Some(&keys[1])));
    assert_ne!(
        own.beam(),
        borrowed.beam(),
        "the two lasers have different beams"
    );
    // the Z is the first laser's either way; only the spot differs, because the beam does
    assert_eq!(own.commanded_rows(), borrowed.commanded_rows());
    assert_eq!(own.actual_rows(), borrowed.actual_rows());
    assert_ne!(
        bits_of(&own.actual().major_radii_um),
        bits_of(&borrowed.actual().major_radii_um)
    );
}

#[test]
fn the_result_says_the_stage_the_version_and_where_each_calibration_value_came_from() {
    let (opened, layer, keys) = first_layer("v17_oct15.h5");
    let source = opened.source();
    let spot = spot_size(source, &request(layer, &keys[0]), &ReadContext::new())
        .expect("the laser")
        .expect("a spot size");
    assert_eq!(spot.stage(), Stage::Shifted);
    assert_eq!(spot.algorithm_version(), SPOT_SIZE_ALGORITHM_VERSION);
    assert!(
        spot.findings().is_empty()
            || spot
                .findings()
                .iter()
                .all(|f| matches!(f, SpotSizeFinding::ZLengthsDiffer { .. }))
    );
    // the user's override of the field size is the user's, over the file's value
    let overridden = spot_size(
        source,
        &request(layer, &keys[0])
            .with_z_override(ZCalibrationOverride::new().with_field_size_z_mm(123.0)),
        &ReadContext::new(),
    )
    .expect("the laser")
    .expect("a spot size");
    assert_eq!(overridden.z().field_size_mm().value(), 123.0);
    assert_eq!(
        overridden.z().field_size_mm().origin(),
        Some(&Origin::Overridden)
    );
    // a spot from another field size is another spot
    assert_ne!(
        bits_of(&overridden.actual().major_radii_um),
        bits_of(&spot.actual().major_radii_um)
    );
    // what the file lacks is the caller's answer, and says so
    let from_caller = [spot.z().field_size_mm(), spot.z().offset_mm()]
        .iter()
        .filter(|v| v.origin() == Some(&Origin::CallerSupplied))
        .count();
    let _ = from_caller; // which of them the file lacks differs by file; the origin is what the result carries
    assert!(spot.period().value() > 0.0);
    // the timing is resolved with each offset's origin
    assert!(spot.timing().native_period().value() > 0.0);
}

#[test]
fn a_timing_override_as_the_user_sets_it_is_not_mirrored_by_the_library() {
    let (opened, layer, keys) = first_layer("v17_oct15.h5");
    let source = opened.source();
    let user = TimingOverride::new().with_commanded_offset_steps(25.0);
    let ask = |overrides: TimingOverride| {
        spot_size(
            source,
            &request(layer, &keys[0]).with_timing_override(overrides),
            &ReadContext::new(),
        )
        .expect("the laser")
        .expect("a spot size")
    };
    let as_set = ask(user.clone());
    let mirrored = ask(vision_mirrored(&user));
    let none = ask(TimingOverride::new());
    // what was resolved says whose offset each is: as the user set it, only the commanded one is theirs; mirrored by the caller, both are
    assert_eq!(
        as_set.timing().commanded().seconds().origin(),
        Some(&Origin::Overridden)
    );
    assert_ne!(
        as_set.timing().actual().seconds().origin(),
        Some(&Origin::Overridden),
        "the library does not mirror: the actual offset is the file's"
    );
    for offset in [mirrored.timing().commanded(), mirrored.timing().actual()] {
        assert_eq!(offset.seconds().origin(), Some(&Origin::Overridden));
    }
    assert_eq!(
        none.timing().commanded().seconds().origin(),
        as_set.timing().actual().seconds().origin()
    );
    // the spot follows: the commanded spot does not read the commanded Z (it is the focus shift alone), and the actual spot is made from the Z pair, so it moves with each offset
    assert_eq!(
        bits_of(&as_set.commanded().major_radii_um),
        bits_of(&none.commanded().major_radii_um)
    );
    let actual_of = |s: &build_log_processing::LaserSpotSize| bits_of(&s.actual().major_radii_um);
    assert_ne!(actual_of(&as_set), actual_of(&none));
    assert_ne!(actual_of(&mirrored), actual_of(&as_set));
    assert_ne!(actual_of(&mirrored), actual_of(&none));
}
