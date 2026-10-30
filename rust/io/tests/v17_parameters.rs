//! The parameters of a V17 laser (IP-13 phase 13H, task T13.10; decisions D13-16, D13-18, D13-21 and D13-25).
//!
//! * A laser's parameters are the calibration of its optical train, and laser N uses train N. Each value is read as the file gave it, with the unit only where the file
//!   has one; the volts-to-watts equation is named constants; a power limit that is text is not a number; a train that cannot be read is reported on its laser.
//! * Whatever is not typed is kept, below the train: the correction grids stay untyped (the question of their domain is open), and the formula images are kept opaque.
//!
//! The conformance of the six files, parameters included, is held by `v17_gate.rs`, and nothing is dropped there too. The small files here are made at test time.

mod common;

use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    AttrValue, BuildMeta, EquationKind, GalvoProtocol, LaserKey, LaserParameters, NodeKind, Origin,
    ParameterValue, Quantity, StoredClass, UnitSource,
};
use build_log_testkit::{miniature, miniature_path};
use common::{laser, layer, monitoring, put_array, put_scalar, put_text, temp_h5, TempH5};
use hdf5::Group;

fn build_of(file: &str) -> BuildMeta {
    let opened = open(production(), miniature_path(file)).expect("the file opens");
    (*opened.source().build().expect("the metadata")).clone()
}

fn parameters_of(build: &BuildMeta, number: u32) -> LaserParameters {
    build
        .laser(&LaserKey::new(format!("Laser_{number}"), number))
        .expect("the laser")
        .parameters
        .clone()
        .expect("its parameters")
}

fn value(parameters: &LaserParameters, name: &str) -> ParameterValue {
    parameters
        .values()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("`{name}` is not set"))
        .1
}

fn quantity(parameters: &LaserParameters, name: &str) -> Quantity {
    match value(parameters, name) {
        ParameterValue::Quantity(q) => q,
        other => panic!("`{name}` is {other:?}, not a quantity"),
    }
}

/// A file whose mount has a build id and one layer with lasers 1 and 2; `write` adds the calibration (and anything else).
fn synthetic(label: &str, write: impl FnOnce(&Group, &Group)) -> BuildMeta {
    let on_disk: TempH5 = temp_h5(label, |mount| {
        put_text(mount, layout::BUILD_ID, "a build");
        let layers = monitoring(mount);
        let first = layer(&layers, 1);
        laser(&first, 1, 5, None);
        laser(&first, 2, 5, None);
        write(mount, &first);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    v17::read::read_build(&mount).expect("the build reads")
}

/// The group of optical train `number`, made if it is not there.
fn train(mount: &Group, number: u32) -> Group {
    let calibration = mount
        .group(layout::CALIBRATION_DATA)
        .or_else(|_| mount.create_group(layout::CALIBRATION_DATA))
        .expect("the calibration group");
    calibration
        .create_group(&format!("{}{number}", layout::OPTICAL_TRAIN_PREFIX))
        .expect("an optical train")
}

fn light_source(train: &Group, algorithm: &str, constants: &str) -> Group {
    let light = train
        .create_group(layout::LIGHT_SOURCE)
        .expect("a light source");
    put_text(&light, layout::VOLTS_TO_WATTS_ALGORITHM, algorithm);
    put_text(&light, layout::VOLTS_TO_WATTS_PARAMS, constants);
    light
}

// ---------------------------------------------------------------- per laser, by name, with the unit only where the file has one

#[test]
fn the_scanner_and_light_source_attributes_become_parameters_per_laser() {
    let build = build_of(miniature::V17_TRIGGERED_2026_10_19);
    let (one, two) = (parameters_of(&build, 1), parameters_of(&build, 2));
    // each laser has its own train's values, paired by number, and the pairing is said to be the adapter's
    assert_eq!(
        (one.optical_train_number, two.optical_train_number),
        (Some(1), Some(2))
    );
    assert_eq!(
        one.origins.get("optical_train_number"),
        Some(&Origin::DeclaredByAdapter)
    );
    assert_eq!(
        (
            one.optics.thermal_lensing_test_passed,
            two.optics.thermal_lensing_test_passed
        ),
        (Some(false), Some(true))
    );
    // the scanner: no unit attribute, so no unit; the wavelength has one, and it is the file's
    let field = quantity(&one, "scanner.field_size_x");
    assert_eq!(field.value, 600.0);
    assert_eq!(field.unit_source, UnitSource::Unknown);
    let wavelength = quantity(&one, "light_source.wavelength");
    assert_eq!(wavelength.unit.id.as_deref(), Some("nm"));
    assert_eq!(wavelength.unit_source, UnitSource::FromFile);
    // text, whole numbers, a plain number and a protocol keep their own kinds
    assert!(one.scanner.model.as_deref().is_some_and(|m| !m.is_empty()));
    assert_eq!(one.galvo_x.actual_bit_resolution, Some(20));
    assert_eq!(one.scanner.scaling_x, Some(1.0));
    assert_eq!(one.galvo_y.protocol, Some(GalvoProtocol::Sl2100));
    assert_eq!(
        one.scanner_card.sample_period.as_ref().map(|q| q.value),
        Some(10.0)
    );
    // what the parameters typed is not also kept, and what they did not type is kept below the train
    assert!(one.extra.attribute(layout::SCAN_HEAD_MODEL).is_none());
    assert!(one.extra.node(&["Scanner".to_string()]).is_none());
    let grid = one
        .extra
        .node(&["Correction_Data".to_string()])
        .expect("the correction grid is kept untyped");
    assert_eq!(grid.dims.as_slice(), [257, 257, 2]);
    assert_eq!(grid.kind, NodeKind::Dataset);
    // and the calibration is no longer in the build's own extra, except what no laser typed
    assert!(build.extra.nodes().all(|n| n
        .path
        .get(1)
        .is_none_or(|p| !p.starts_with(layout::OPTICAL_TRAIN_PREFIX))));
}

#[test]
fn a_number_stored_as_an_integer_in_one_file_reads_as_a_number() {
    // the 2026-10-15 file has `int32` where the current output has `float32` for the same beam values
    let build = build_of(miniature::V17_OCT15);
    let waist = quantity(&parameters_of(&build, 1), "optics.waist_major");
    assert_eq!(waist.value, 66.0);
    assert_eq!(waist.unit.id.as_deref(), Some("um"));
}

#[test]
fn a_laser_with_no_optical_train_has_no_parameters_and_a_train_no_laser_uses_is_kept() {
    let build = synthetic("params-trains", |mount, _| {
        let one = train(mount, 1);
        put_text(&one, layout::METADATA, "train one");
        let three = train(mount, 3);
        put_text(&three, layout::METADATA, "train three");
    });
    let lasers = build.lasers();
    assert!(lasers[0].parameters.is_some());
    // laser 2 has no train 2: absent is absent, and nothing is said about it
    assert!(lasers[1].parameters.is_none() && lasers[1].origins.get("parameters").is_none());
    // train 3 belongs to no laser, so it is kept whole in the build's extra; train 1 is the parameters of laser 1
    let kept = build
        .extra
        .node(&["Calibration_Data".into(), "Optical_Train_3".into()])
        .expect("the train that no laser uses is kept");
    assert!(kept.attribute(layout::METADATA).is_some());
    assert!(build
        .extra
        .node(&["Calibration_Data".into(), "Optical_Train_1".into()])
        .is_none());
}

// ---------------------------------------------------------------- the equation

#[test]
fn volts_to_watts_becomes_named_constants_in_both_flavors() {
    // the current output (variable-length text): a line on the first laser and a polynomial on the second
    let gate = build_of(miniature::V17_TRIGGERED_2026_10_19);
    let line = parameters_of(&gate, 1).light_source.volts_to_watts.unwrap();
    assert_eq!(line.kind, EquationKind::Linear);
    assert_eq!(
        (line.constant("b"), line.constant("a")),
        (Some(50.5), Some(107.5))
    );
    let polynomial = parameters_of(&gate, 2).light_source.volts_to_watts.unwrap();
    assert_eq!(polynomial.kind, EquationKind::Polynomial);
    assert_eq!(
        (
            polynomial.constant("c0"),
            polynomial.constant("c1"),
            polynomial.constant("c2")
        ),
        (Some(10.0), Some(5.0), Some(0.5))
    );
    // the March files (fixed-length text) give the same names, and the names are the adapter's, which the origin says
    let march = build_of(miniature::V17_MARCH);
    let second = parameters_of(&march, 2);
    let march_line = second.light_source.volts_to_watts.as_ref().unwrap();
    assert_eq!(
        (march_line.constant("b"), march_line.constant("a")),
        (Some(48.5), Some(105.0))
    );
    assert_eq!(
        second.origins.get("light_source.volts_to_watts"),
        Some(&Origin::DeclaredByAdapter)
    );
    // the text is typed, so it is not kept as well
    assert!(second
        .extra
        .node(&["Light_Source".to_string()])
        .is_none_or(|n| n.attribute(layout::VOLTS_TO_WATTS_PARAMS).is_none()));
}

#[test]
fn an_unknown_algorithm_stays_other() {
    let build = synthetic("params-equation", |mount, _| {
        light_source(&train(mount, 1), "EXPONENTIAL", "1,2");
        // matched exactly: the same word in another case has no name either
        light_source(&train(mount, 2), "Linear", "1,2");
    });
    let first = parameters_of(&build, 1);
    let equation = first.light_source.volts_to_watts.as_ref().unwrap();
    assert_eq!(equation.kind, EquationKind::Other("EXPONENTIAL".into()));
    assert!(equation.constants.is_empty());
    // its constants are positional text that nobody can name, so they stay as they are, in the extra of the snapshot
    let kept = first.extra.node(&["Light_Source".to_string()]).unwrap();
    assert_eq!(
        kept.attribute(layout::VOLTS_TO_WATTS_PARAMS).unwrap().value,
        AttrValue::Text(vec!["1,2".into()])
    );
    assert_eq!(
        parameters_of(&build, 2)
            .light_source
            .volts_to_watts
            .unwrap()
            .kind,
        EquationKind::Other("Linear".into())
    );
}

#[test]
fn constants_that_do_not_fit_the_algorithm_are_a_fault_in_that_train() {
    let build = synthetic("params-constants", |mount, _| {
        light_source(&train(mount, 1), layout::ALGORITHM_LINEAR, "1,2,3");
        light_source(&train(mount, 2), layout::ALGORITHM_POLYNOMIAL, "1, 2.5, 4");
    });
    // a line takes two constants: laser 1 is reported on itself and its train is kept whole
    let one = build.laser(&LaserKey::new("Laser_1", 1)).unwrap();
    assert!(one.parameters.is_none());
    assert!(
        matches!(one.origins.get("parameters"), Some(Origin::NotRead { reason: why, .. }) if why.contains("constants"))
    );
    assert!(build
        .extra
        .node(&["Calibration_Data".into(), "Optical_Train_1".into()])
        .is_some());
    // a polynomial takes as many as it is given, named by their power, and spaces around a number are not part of it
    let polynomial = parameters_of(&build, 2)
        .light_source
        .volts_to_watts
        .unwrap();
    assert_eq!(polynomial.kind, EquationKind::Polynomial);
    assert_eq!(
        (
            polynomial.constant("c0"),
            polynomial.constant("c1"),
            polynomial.constant("c2")
        ),
        (Some(1.0), Some(2.5), Some(4.0))
    );
}

// ---------------------------------------------------------------- opaque things and things that are not numbers

#[test]
fn the_formula_images_are_kept_opaque() {
    for file in [miniature::V17_TRIGGERED_2026_10_19, miniature::V17_MARCH] {
        let build = build_of(file);
        for (name, extents) in [
            ("BitsToMillimetersFormula", [660, 692, 3]),
            ("VoltsToWattsFormula", [771, 1048, 3]),
        ] {
            let image = build
                .extra
                .node(&["Calibration_Data".into(), name.into()])
                .unwrap_or_else(|| panic!("{file}: {name} is kept"));
            // a dataset kept by its extents and its element type, with the attributes of the image; its samples are not held
            assert_eq!(image.kind, NodeKind::Dataset);
            assert_eq!(image.dims.as_slice(), extents, "{file}: {name}");
            assert_eq!(
                image
                    .element
                    .as_ref()
                    .map(|e| (e.class.clone(), e.bits, e.signed)),
                Some((StoredClass::Integer, Some(8), Some(false)))
            );
            let attributes: Vec<&str> = image.attributes().map(|(n, _)| n).collect();
            assert_eq!(
                attributes,
                [
                    "CLASS",
                    "DISPLAY_ORIGIN",
                    "IMAGE_SUBCLASS",
                    "IMAGE_WHITE_IS_ZERO"
                ]
            );
        }
    }
}

#[test]
fn both_sample_period_forms_become_one_quantity() {
    let build = synthetic("params-periods", |mount, first| {
        // a number with no unit, as the scanner card of every file we hold has it; and text that has its unit in it
        for (number, written) in [(1, None), (2, Some("10us"))] {
            let card = train(mount, number)
                .create_group(layout::SCANNER_CARD)
                .unwrap();
            match written {
                None => put_array(&card, layout::SAMPLE_PERIOD, &[10.0f32]),
                Some(text) => put_text(&card, layout::SAMPLE_PERIOD, text),
            }
        }
        // the laser's time axis takes its period from the same card, so that the two cannot disagree
        let _ = first;
    });
    let at = |number: u32| {
        let laser = build
            .laser(&LaserKey::new(format!("Laser_{number}"), number))
            .unwrap();
        let axis = laser
            .layers
            .get(build_log_model::LayerId(1))
            .unwrap()
            .time_axis
            .period
            .clone();
        (
            parameters_of(&build, number).scanner_card.sample_period,
            axis,
        )
    };
    let (card, axis) = at(1);
    assert_eq!(card, axis);
    let number = card.expect("a period");
    assert_eq!(
        (number.value, number.unit_source),
        (10.0, UnitSource::Unknown)
    );
    let (card, axis) = at(2);
    assert_eq!(card, axis);
    let text = card.expect("a period");
    assert_eq!(
        (text.value, text.unit.id.as_deref(), text.unit_source),
        (10.0, Some("us"), UnitSource::FromFile)
    );
}

#[test]
fn a_power_limit_stored_as_text_is_not_recorded_as_a_number() {
    // the current output has four limits as numbers
    let current = parameters_of(&build_of(miniature::V17_MOCK), 1);
    assert!(current.origins.get("light_source.power_limits").is_none());
    let minimum = quantity(&current, "light_source.power_min_actual");
    assert_eq!(
        (minimum.value, minimum.unit_source),
        (101.69999694824219, UnitSource::Unknown)
    );
    // the March files hold two attributes of text in which the old writer put the bytes of a number: no limit is recorded, the origin says why, and the text is kept
    for file in [miniature::V17_MARCH, miniature::V17_SPECIAL_LAYERS] {
        for number in [1, 2] {
            let march = parameters_of(&build_of(file), number);
            for name in [
                "light_source.power_min_actual",
                "light_source.power_min_nominal",
                "light_source.power_max_actual",
                "light_source.power_max_nominal",
            ] {
                assert!(
                    march.values().iter().all(|(n, _)| n.as_str() != name),
                    "{file}, laser {number}: {name} is not recorded"
                );
            }
            match march.origins.get("light_source.power_limits") {
                Some(Origin::Other(why)) => assert!(why.contains("text"), "{why}"),
                other => panic!("{file}, laser {number}: expected the reason, got {other:?}"),
            }
            let kept = march
                .extra
                .node(&["Light_Source".to_string()])
                .expect("the light source is kept");
            assert!(kept.attribute(layout::POWER_MIN).is_some());
            assert!(kept.attribute(layout::POWER_MAX).is_some());
        }
    }
}

// ---------------------------------------------------------------- words that are named, and faults

#[test]
fn the_galvo_protocol_names_the_words_in_the_files_and_keeps_any_other() {
    let build = synthetic("params-protocols", |mount, _| {
        let scanner = train(mount, 1).create_group(layout::SCANNER).unwrap();
        for (galvo, text) in [
            (layout::GALVO_X, "XY2-100"),
            (layout::GALVO_Y, "SL2-100"),
            (layout::GALVO_Z, "xy2-100"),
        ] {
            let group = scanner.create_group(galvo).unwrap();
            put_text(&group, layout::COMMUNICATION_PROTOCOL, text);
        }
    });
    let parameters = parameters_of(&build, 1);
    assert_eq!(parameters.galvo_x.protocol, Some(GalvoProtocol::Xy2100));
    assert_eq!(parameters.galvo_y.protocol, Some(GalvoProtocol::Sl2100));
    // matched exactly: the same word in another case has no name
    assert_eq!(
        parameters.galvo_z.protocol,
        Some(GalvoProtocol::Other("xy2-100".into()))
    );
}

#[test]
fn a_train_that_cannot_be_read_is_reported_on_its_laser_and_kept_whole() {
    let build = synthetic("params-faults", |mount, _| {
        // a flag that is 7, and a galvo with two names for its tuning type: each is a fault in its own train
        let one = train(mount, 1);
        put_scalar(&one, layout::OPTICAL_TRAIN_MOVEABLE, 7u8);
        let two = train(mount, 2);
        let galvo = two
            .create_group(layout::SCANNER)
            .unwrap()
            .create_group(layout::GALVO_X)
            .unwrap();
        put_text(&galvo, layout::TUNING_TYPE, "PID");
        put_text(&galvo, layout::GALVO_TUNING_TYPE, "PID");
    });
    for (number, why) in [(1, "only 0 and 1"), (2, "two attributes")] {
        let laser = build
            .laser(&LaserKey::new(format!("Laser_{number}"), number))
            .unwrap();
        assert!(laser.parameters.is_none(), "laser {number}");
        match laser.origins.get("parameters") {
            Some(Origin::NotRead { cause, reason }) => {
                assert_eq!(
                    *cause,
                    build_log_model::NotReadCause::Fault,
                    "laser {number}"
                );
                assert!(reason.contains(why), "{reason}");
            }
            other => panic!("laser {number}: expected the reason, got {other:?}"),
        }
        // the train is kept whole, so nothing it holds is lost, and the build opened
        assert!(
            build
                .extra
                .node(&["Calibration_Data".into(), format!("Optical_Train_{number}")])
                .is_some(),
            "train {number}"
        );
    }
}
