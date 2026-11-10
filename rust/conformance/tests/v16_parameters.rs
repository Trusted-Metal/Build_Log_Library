//! The parameter snapshot of a V16 laser, held to the expectation of the V16 miniature and shown to fail when it is wrong (IP-12 phase 12B, task T12.2; decisions D12-6 and Q12-2; IP-24 finding F2).
//!
//! V16 keeps its calibration per laser (`Laser_Calibration_N`), so **a snapshot has no optical train**; thirteen attributes have a CDM name and the mount's two bit resolutions are given to each
//! laser's three galvos. **The offsets carry a note** that their sign is opposite to other versions' (inferred), **and the bit resolutions a note that the file gave one value for all**: the independent
//! script's own sentences, which the Model's two constants must equal.
//!
//! * `the_v16_synthetic_build_gives_the_parameters_the_expectation_states`: a build made from the expectation conforms, and holds the values and the origins the script wrote.
//! * `the_models_notes_are_the_oracles_words`: the Model's constants are the script's sentences.
//! * The rest hold the check: a flipped sign, a note that is lost, other or on the wrong value, a value that is missing, extra or another, and an optical train that the file does not have.

use build_log_conformance::{check_parts, load_all, synthesise, Expectation, Part};
use build_log_model::{
    BuildMeta, LaserMeta, Origin, ParameterValue, Quantity, BIT_RESOLUTION_GLOBAL,
    SCAN_HEAD_OFFSET_SIGN_OPPOSITE,
};

fn v16() -> Expectation {
    load_all()
        .expect("the expectation files can be read")
        .into_iter()
        .find(|e| e.miniature() == "v16.h5")
        .expect("an expectation for the V16 miniature")
}

/// What the parameters part says of the V16 build after `change` is made to laser `index`.
fn after(index: usize, change: impl FnOnce(&mut LaserMeta)) -> Vec<String> {
    let expectation = v16();
    let mut meta: BuildMeta = synthesise(&expectation).expect("a synthetic CDM");
    let mut laser = meta.lasers()[index].clone();
    change(&mut laser);
    meta.add_laser(laser);
    check_parts(&meta, &expectation, &[Part::Parameters])
}

const OFFSETS: [&str; 3] = ["scanner.offset_x", "scanner.offset_y", "scanner.offset_z"];

#[test]
fn the_v16_synthetic_build_gives_the_parameters_the_expectation_states() {
    let expectation = v16();
    let meta = synthesise(&expectation).expect("a synthetic CDM");
    assert_eq!(
        check_parts(&meta, &expectation, &[Part::Parameters]),
        Vec::<String>::new()
    );
    assert_eq!(meta.lasers().len(), 2);
    for laser in meta.lasers() {
        let parameters = laser
            .parameters
            .as_ref()
            .expect("a snapshot for each laser");
        // no optical train: the calibration is the laser's own
        assert_eq!(parameters.optical_train_number, None);
        // thirteen values of the calibration and six bit resolutions
        assert_eq!(parameters.values().len(), 19);
        for axis in ["galvo_x", "galvo_y", "galvo_z"] {
            for field in ["actual_bit_resolution", "commanded_bit_resolution"] {
                assert_eq!(
                    parameters.origins.get(&format!("{axis}.{field}")),
                    Some(&Origin::Other(BIT_RESOLUTION_GLOBAL.to_string()))
                );
            }
        }
        for name in OFFSETS {
            assert_eq!(
                parameters.origins.get(name),
                Some(&Origin::Other(SCAN_HEAD_OFFSET_SIGN_OPPOSITE.to_string())),
                "{name}"
            );
        }
        // nothing else has an origin: the note belongs to the offsets and the bit resolutions
        assert_eq!(parameters.origins.iter().count(), 9);
    }
    // the first laser's offsets are the file's own, as written: the signs that other versions have the other way round
    let first = meta.lasers()[0].parameters.as_ref().unwrap();
    let value = |name: &str| match first.values().into_iter().find(|(n, _)| n == name) {
        Some((_, ParameterValue::Quantity(q))) => q.value,
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(
        (value("scanner.offset_x"), value("scanner.offset_y")),
        (87.5, -23.5)
    );
}

#[test]
fn the_models_notes_are_the_oracles_words() {
    // the script's sentences are in the expectation file, and the Model's constants are held to them: a constant that changes breaks this, not only an adapter
    let expectation = v16();
    let meta = synthesise(&expectation).expect("a synthetic CDM");
    let parameters = meta.lasers()[0].parameters.as_ref().unwrap();
    for name in OFFSETS {
        assert_eq!(
            parameters.origins.get(name),
            Some(&Origin::Other(SCAN_HEAD_OFFSET_SIGN_OPPOSITE.into()))
        );
    }
    assert_eq!(
        parameters.origins.get("galvo_x.actual_bit_resolution"),
        Some(&Origin::Other(BIT_RESOLUTION_GLOBAL.into()))
    );
    // and the synthetic build takes the words from the file: a build that carries the Model's constants conforms, one that carries other words does not
    let found = after(0, |laser| {
        let p = laser.parameters.as_mut().unwrap();
        p.origins.record(
            "scanner.offset_x",
            Origin::Other(format!("{SCAN_HEAD_OFFSET_SIGN_OPPOSITE}!")),
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("scanner.offset_x") && found[0].contains("its origin"));
}

#[test]
fn a_flipped_sign_a_changed_missing_or_extra_value_is_reported_on_its_own_name() {
    // the offset's sign is flipped: the value is what changes, and it is named
    let found = after(0, |laser| {
        let q = laser
            .parameters
            .as_mut()
            .unwrap()
            .scanner
            .offset_x
            .as_mut()
            .unwrap();
        q.value = -q.value;
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("scanner.offset_x") && found[0].contains("87.5"),
        "{found:?}"
    );
    // a unit that the file does not state is guessed
    let found = after(0, |laser| {
        laser.parameters.as_mut().unwrap().scanner.field_size_x =
            Some(Quantity::from_file(600.0, Some("mm")));
    });
    assert!(
        found
            .iter()
            .any(|l| l.contains("field_size_x") && l.contains("unit")),
        "{found:?}"
    );
    // a value that is missing
    let found = after(1, |laser| {
        laser.parameters.as_mut().unwrap().scanner.model = None;
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("scanner.model") && found[0].contains("not there"));
    // a value that the file does not have: V16's calibration holds no power limit
    let found = after(0, |laser| {
        laser
            .parameters
            .as_mut()
            .unwrap()
            .light_source
            .power_max_actual = Some(Quantity::from_file(300.0, None));
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("power_max_actual") && found[0].contains("found, expected nothing"));
    // a bit resolution that is another number, and one that is not given to every galvo
    let found = after(1, |laser| {
        laser
            .parameters
            .as_mut()
            .unwrap()
            .galvo_y
            .actual_bit_resolution = Some(16);
    });
    assert!(
        found
            .iter()
            .any(|l| l.contains("galvo_y.actual_bit_resolution")),
        "{found:?}"
    );
    let found = after(0, |laser| {
        laser
            .parameters
            .as_mut()
            .unwrap()
            .galvo_z
            .commanded_bit_resolution = None;
    });
    assert!(
        found
            .iter()
            .any(|l| l.contains("galvo_z.commanded_bit_resolution") && l.contains("not there")),
        "{found:?}"
    );
}

#[test]
fn a_note_that_is_lost_other_or_on_the_wrong_value_is_reported() {
    // the sign note is lost: the origin of that offset is not there, and the other two are
    let found = after(0, |laser| {
        let p = laser.parameters.as_mut().unwrap();
        let kept: Vec<_> = p
            .origins
            .iter()
            .filter(|(name, _)| *name != "scanner.offset_y")
            .map(|(name, origin)| (name.to_string(), origin.clone()))
            .collect();
        p.origins = build_log_model::Origins::new();
        for (name, origin) in kept {
            p.origins.record(name, origin);
        }
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("scanner.offset_y") && found[0].contains("its origin"));
    // the sentence says something else
    let found = after(1, |laser| {
        laser.parameters.as_mut().unwrap().origins.record(
            "galvo_x.actual_bit_resolution",
            Origin::Other("per axis".into()),
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("galvo_x.actual_bit_resolution"));
    // a note on a value that has none is an origin that was not expected
    let found = after(0, |laser| {
        laser.parameters.as_mut().unwrap().origins.record(
            "scanner.scaling_x",
            Origin::Other(SCAN_HEAD_OFFSET_SIGN_OPPOSITE.into()),
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("scanner.scaling_x") && found[0].contains("expected nothing"));
    // an origin of another kind on the offset is not the note
    let found = after(0, |laser| {
        laser
            .parameters
            .as_mut()
            .unwrap()
            .origins
            .record("scanner.offset_x", Origin::DeclaredByAdapter);
    });
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn an_optical_train_that_the_file_does_not_have_is_reported() {
    let found = after(0, |laser| {
        laser.parameters.as_mut().unwrap().optical_train_number = Some(1);
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("the optical train"), "{found:?}");
}

#[test]
fn an_expectation_without_origins_lists_none_and_asks_nothing_of_them() {
    // the V17 files list no `origins`: a snapshot may say what it says (the volts-to-watts note, the pairing) and the check does not ask for them
    let all = load_all().expect("the expectation files can be read");
    let mut checked = 0;
    for expectation in all.iter().filter(|e| e.miniature() != "v16.h5") {
        let mut meta = synthesise(expectation).expect("a synthetic CDM");
        let Some(mut laser) = meta.lasers().first().cloned() else {
            continue;
        };
        let Some(parameters) = laser.parameters.as_mut() else {
            continue;
        };
        parameters
            .origins
            .record("scanner.offset_x", Origin::Other("anything".into()));
        meta.add_laser(laser);
        assert_eq!(
            check_parts(&meta, expectation, &[Part::Parameters]),
            Vec::<String>::new(),
            "{}",
            expectation.miniature()
        );
        checked += 1;
    }
    // the six V17 files have a snapshot for their first laser; V18 has none yet
    assert_eq!(checked, 6);
}
