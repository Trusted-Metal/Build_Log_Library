//! What Vision does with the library's beam, written once for the tests that run Vision's `spot_size_resolver` vectors (IP-27 phase 27H; decisions D27-18 and D27-23).
//!
//! **Everything in this module is Vision's and not the library's**, and it is made here as the adapter of phase 27K will make it: the seven answers where the file is silent, the text of each note
//! and of each reason (the library gives structured findings and a reason, and Vision's texts are its own), and the columns the vectors record. Both the pure cases (this crate's
//! `spot_size_resolver_vectors.rs`) and the cases on the miniatures (the facade's test of the same name, which includes this file by its path) use it.

#![allow(dead_code)] // each test that uses this module uses a part of it

use build_log_conformance::vectors::{bits, exact_opt_f64, whole, words, Columns};
use build_log_model::SpotSizeOverride;
use build_log_processing::{
    AngleUnit, BeamAssumptions, BeamAxis, BeamFinding, BeamInput, BeamUnavailable, LengthUnit,
    OneWaistMissing, ResolvedBeam,
};
use serde_json::Value;

/// Vision's answers where the file is silent. **They are Vision's:** the library reports each case and has no default, and the adapter of phase 27K will carry these, as this test does.
pub fn vision_assumptions() -> BeamAssumptions {
    BeamAssumptions {
        // the reader's defaults: a waist in micrometres, the other lengths in millimetres, an angle in degrees
        waist_unit: LengthUnit::Micrometre,
        length_unit: LengthUnit::Millimetre,
        angle_unit: AngleUnit::Degree,
        // "nanometres is assumed" for the wavelength, whatever its companion says
        wavelength_unit: LengthUnit::Nanometre,
        // `OpticalTrain` cannot say "absent": a value that is not stored is 0
        absent_value: 0.0,
        // a Rayleigh length that is derived assumes an M2 of 1 when none is known
        m2_when_none: 1.0,
        // one waist missing: a circular beam from the other
        one_waist_missing: OneWaistMissing::AssumeCircular,
    }
}

/// The user's overrides of a case (a `null` is not set), read from their exact bits.
pub fn override_of(overrides: &Value) -> SpotSizeOverride {
    let value = |key: &str| exact_opt_f64(overrides, key);
    let mut set = SpotSizeOverride::new();
    if let Some(v) = value("waist_major_um") {
        set = set.with_waist_major_um(v);
    }
    if let Some(v) = value("waist_minor_um") {
        set = set.with_waist_minor_um(v);
    }
    if let Some(v) = value("rayleigh_major_mm") {
        set = set.with_rayleigh_major_mm(v);
    }
    if let Some(v) = value("rayleigh_minor_mm") {
        set = set.with_rayleigh_minor_mm(v);
    }
    if let Some(v) = value("m2_major") {
        set = set.with_m2_major(v);
    }
    if let Some(v) = value("m2_minor") {
        set = set.with_m2_minor(v);
    }
    if let Some(v) = value("waist_offset_z_mm") {
        set = set.with_waist_offset_z_mm(v);
    }
    if let Some(v) = value("thermal_shift_mm") {
        set = set.with_thermal_shift_mm(v);
    }
    if let Some(v) = value("major_axis_angle_deg") {
        set = set.with_major_axis_angle_deg(v);
    }
    if let Some(v) = value("scanner_rotation_deg") {
        set = set.with_scanner_rotation_deg(v);
    }
    set
}

/// An axis as Vision writes it at the start of a sentence.
fn capitalised(axis: BeamAxis) -> &'static str {
    match axis {
        BeamAxis::Major => "Major",
        BeamAxis::Minor => "Minor",
    }
}

/// Vision's note for each finding, in the order they were made, **and then the thermal-lensing note** (Vision raises it from the flag, which the library carries as recorded and raises no finding for).
pub fn notes_of(beam: &ResolvedBeam) -> Vec<String> {
    let mut notes: Vec<String> = beam
        .findings()
        .iter()
        .map(|finding| match finding {
            BeamFinding::CircularBeamAssumed { missing } => {
                format!("{missing} waist missing; assuming a circular beam")
            }
            BeamFinding::RayleighDerived {
                axis,
                derived_mm,
                m2_assumed: None,
            } => format!(
                "{} Rayleigh length derived from waist, M\u{b2} and wavelength ({derived_mm:.2} mm)",
                capitalised(*axis)
            ),
            BeamFinding::RayleighDerived {
                axis,
                derived_mm,
                m2_assumed: Some(m2),
            } => format!(
                "{} Rayleigh length derived assuming M\u{b2} = {m2} ({derived_mm:.2} mm)",
                capitalised(*axis)
            ),
            BeamFinding::RayleighMismatch {
                axis,
                stored_mm,
                derived_mm,
            } => format!(
                "{} Rayleigh length {stored_mm:.2} mm differs from the value implied by waist, M\u{b2} and wavelength ({derived_mm:.2} mm)",
                capitalised(*axis)
            ),
            other => panic!("a finding that Vision has no note for: {other:?}"),
        })
        .collect();
    let thermal = beam.thermal_test();
    if thermal.passed == Some(false) {
        notes.push(format!(
            "thermal lensing test not passed (focal-plane shift {:.2} mm, threshold {:.2} mm)",
            thermal.shift_mm, thermal.threshold_mm
        ));
    }
    notes
}

/// Vision's text for a laser with no beam. **A V16 laser** (a snapshot with no waist) **is `NoWaist`, as in Vision**, whose V16 loader gives each laser a train of zeros (the owner, 2026-11-17: those
/// default trains are accepted as they are). **`NoTrain` is not Vision's V16 case:** it is a laser with no snapshot at all, for which Vision's adapter uses the first train's beam, so it has no text here.
pub fn error_text(error: &BeamUnavailable) -> String {
    match error {
        BeamUnavailable::NoWaist => {
            "no beam waist stored (Beam_Waist_Major/Minor missing or zero)".to_string()
        }
        BeamUnavailable::NoRayleigh { axis } => format!(
            "{} Rayleigh length is not stored and cannot be derived (no wavelength)",
            capitalised(*axis)
        ),
        // a laser with no snapshot: Vision's adapter asks again with the first train; and Vision assumes a circular beam, so it never asks for the other
        other => panic!("a reason that Vision's policy never gives: {other:?}"),
    }
}

/// The result of resolving one laser, as columns under `prefix`, as the vectors record it: `ok`, then the six numbers of the beam and its notes, or the reason.
pub fn put_result(
    columns: &mut Columns,
    prefix: &str,
    result: &Result<ResolvedBeam, BeamUnavailable>,
) {
    match result {
        Ok(beam) => {
            columns.insert(format!("{prefix}ok"), whole(&[1]));
            for (name, value) in [
                (
                    "waist_radius_major_um",
                    beam.waist_radius_major_um().value(),
                ),
                (
                    "waist_radius_minor_um",
                    beam.waist_radius_minor_um().value(),
                ),
                ("rayleigh_major_mm", beam.rayleigh_major_mm().value()),
                ("rayleigh_minor_mm", beam.rayleigh_minor_mm().value()),
                ("focus_shift_mm", beam.focus_shift_mm()),
                ("major_axis_angle_deg", beam.axis_angle_deg()),
            ] {
                columns.insert(format!("{prefix}{name}"), bits(&[value]));
            }
            let notes = notes_of(beam);
            let notes: Vec<&str> = notes.iter().map(String::as_str).collect();
            columns.insert(format!("{prefix}notes"), words(&notes));
        }
        Err(error) => {
            columns.insert(format!("{prefix}ok"), whole(&[0]));
            columns.insert(format!("{prefix}error"), words(&[&error_text(error)]));
        }
    }
}

/// The twelve numbers of a train as the resolver gets them, as columns under `prefix`, and the thermal test as `-1` (not recorded), `0` and `1`.
pub fn put_input(columns: &mut Columns, prefix: &str, input: &BeamInput) {
    for (name, value) in [
        ("waist_major_um", input.waist_major_um),
        ("waist_minor_um", input.waist_minor_um),
        ("rayleigh_major_mm", input.rayleigh_major_mm),
        ("rayleigh_minor_mm", input.rayleigh_minor_mm),
        ("m2_major", input.m2_major),
        ("m2_minor", input.m2_minor),
        ("wavelength_nm", input.wavelength_nm),
        ("waist_offset_z_mm", input.waist_offset_z_mm),
        ("thermal_shift_mm", input.thermal_shift_mm),
        ("thermal_threshold_mm", input.thermal_threshold_mm),
        ("major_axis_angle_deg", input.major_axis_angle_deg),
        ("scanner_rotation_deg", input.scanner_rotation_deg),
    ] {
        columns.insert(format!("{prefix}input_{name}"), bits(&[value]));
    }
    let test = match input.thermal_test_passed {
        None => -1,
        Some(false) => 0,
        Some(true) => 1,
    };
    columns.insert(format!("{prefix}input_thermal_test_passed"), whole(&[test]));
}
