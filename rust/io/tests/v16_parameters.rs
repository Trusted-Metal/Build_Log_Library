//! The V16 adapter, part 3: the calibration (IP-12 phase 12E, task T12.5; decisions D12-6 and D12-7, Q12-2 and Q12-3, and IP-24 finding F2; the rule of D13-21).
//!
//! * **`Laser_Calibration_N` is laser N's own**: the snapshot has no optical train, 13 attributes have a CDM name, and the 12 others are **not named** and stay in the snapshot's `extra` as the file has
//!   them. The parameters of the V16 miniature conform to `v16.json` (`Part::Parameters`), and the untyped remainder is the list that the independent script wrote.
//! * **The three offsets carry the note that their sign is opposite to other versions' and are never flipped.** A number is a number whether the file stored it as an `int16`, an `int32` or a `float32`.
//! * **The mount's two bit resolutions are given to each laser's three galvos**, as whole numbers, with the origin that says the value was global in the file. One that is not a whole number is reported
//!   on the galvos' values and stays on the mount.
//! * **The formula images and the correction grid are kept where they are and are not read.**
//! * **A calibration that cannot be read is reported on its laser, kept whole, and the build opens** (D13-21).
//!
//! The small mounts are made at test time with the binding, and read with the adapter's own reader.

mod common;

use build_log_conformance::{check_parts, expectations_dir, Expectation, Part};
use build_log_io::adapters::v16::{layout, parameters::optional_whole_number};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildMeta, ErrorKind, LaserKey, LaserMeta, LaserParameters, NodeKind, NotReadCause, Origin,
    ParameterValue, UnitSource, BIT_RESOLUTION_GLOBAL, SCAN_HEAD_OFFSET_SIGN_OPPOSITE,
};
use build_log_testkit::{miniature, miniature_path};
use common::v16::{calibration, with_lasers};
use common::{put_array, put_scalar, put_text, temp_h5};

fn expectation() -> Expectation {
    Expectation::load(&expectations_dir().join("v16.json")).expect("the V16 expectation file")
}

fn build_of_the_miniature() -> BuildMeta {
    let opened =
        open(production(), miniature_path(miniature::V16)).expect("the V16 miniature opens");
    (*opened.source().build().expect("the metadata")).clone()
}

fn laser_of(build: &BuildMeta, number: u32) -> &LaserMeta {
    build
        .laser(&LaserKey::new(format!("Laser_{number}"), number))
        .expect("the laser")
}

fn parameters_of(build: &BuildMeta, number: u32) -> &LaserParameters {
    laser_of(build, number)
        .parameters
        .as_ref()
        .expect("the laser has a snapshot")
}

fn value(parameters: &LaserParameters, name: &str) -> Option<ParameterValue> {
    parameters
        .values()
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

fn number(parameters: &LaserParameters, name: &str) -> Option<f64> {
    match value(parameters, name)? {
        ParameterValue::Quantity(q) => Some(q.value),
        ParameterValue::Plain(v) => Some(v),
        other => panic!("`{name}` is {other:?}, not a number"),
    }
}

const BIT_NAMES: [&str; 6] = [
    "galvo_x.actual_bit_resolution",
    "galvo_x.commanded_bit_resolution",
    "galvo_y.actual_bit_resolution",
    "galvo_y.commanded_bit_resolution",
    "galvo_z.actual_bit_resolution",
    "galvo_z.commanded_bit_resolution",
];

// ---------------------------------------------------------------- the miniature

#[test]
fn the_v16_parameters_equal_the_expectation() {
    let build = build_of_the_miniature();
    let found = check_parts(&build, &expectation(), &[Part::Parameters]);
    assert!(
        found.is_empty(),
        "the parameters do not conform:\n  {}",
        found.join("\n  ")
    );
    for number in [1, 2] {
        let parameters = parameters_of(&build, number);
        // the calibration is the laser's own: no optical train, and no equation
        assert_eq!(parameters.optical_train_number, None);
        assert!(parameters.light_source.volts_to_watts.is_none());
        // thirteen values of the calibration, six bit resolutions, and a note for each of the three offsets and the six bit resolutions
        assert_eq!(parameters.values().len(), 19);
        assert_eq!(parameters.origins.iter().count(), 9);
    }
    // the calibration of each laser is its own: the two lasers have other values
    assert_eq!(
        number(parameters_of(&build, 1), "optics.major_axis_angle"),
        Some(0.0)
    );
    assert_eq!(
        number(parameters_of(&build, 2), "optics.major_axis_angle"),
        Some(180.0)
    );
}

#[test]
fn the_three_offsets_carry_the_sign_note_and_are_not_flipped() {
    let build = build_of_the_miniature();
    let file = hdf5::File::open(miniature_path(miniature::V16)).expect("the file");
    for number in [1, 2] {
        let parameters = parameters_of(&build, number);
        let group = file
            .group(&format!(
                "{}/{}/{}{number}",
                layout::MOUNT,
                layout::CALIBRATION_DATA,
                layout::CALIBRATION_PREFIX
            ))
            .expect("the calibration");
        for (attribute, name) in [
            (layout::SCANNER_OFFSET_X, "scanner.offset_x"),
            (layout::SCANNER_OFFSET_Y, "scanner.offset_y"),
            (layout::SCANNER_OFFSET_Z, "scanner.offset_z"),
        ] {
            // the value is what the file holds, read by the binding and not by the adapter: not flipped, not rounded
            let written: f64 = group
                .attr(attribute)
                .and_then(|a| a.read_scalar::<f64>())
                .expect("the offset is a number");
            assert_eq!(
                number_of(parameters, name),
                written,
                "{name} of laser {number}"
            );
            assert_eq!(
                parameters.origins.get(name),
                Some(&Origin::Other(SCAN_HEAD_OFFSET_SIGN_OPPOSITE.to_string())),
                "{name} of laser {number}"
            );
        }
        // and nothing else carries that note: the scaling and the field size are as they are
        for name in ["scanner.scaling_x", "scanner.field_size_x", "scanner.model"] {
            assert_eq!(parameters.origins.get(name), None, "{name}");
        }
    }
    // laser 1's offsets are the first two numbers of the file, with the signs that V17 and V18 have the other way round
    let first = parameters_of(&build, 1);
    assert_eq!(number(first, "scanner.offset_x"), Some(87.5));
    assert_eq!(number(first, "scanner.offset_y"), Some(-23.5));
}

fn number_of(parameters: &LaserParameters, name: &str) -> f64 {
    number(parameters, name).unwrap_or_else(|| panic!("`{name}` is not there"))
}

#[test]
fn the_untyped_remainder_of_each_laser_is_the_oracles() {
    // the independent script lists, for each laser, the attributes it did not name and the children it did not read: the snapshot's `extra` is that list, and nothing else
    let text = std::fs::read_to_string(expectations_dir().join("v16.json")).expect("the file");
    let json: serde_json::Value = serde_json::from_str(&text).expect("json");
    let lasers = json["lasers"].as_array().expect("lasers");
    assert_eq!(lasers.len(), 2);
    let build = build_of_the_miniature();
    for (at, entry) in lasers.iter().enumerate() {
        let untyped = &entry["parameters"]["untyped"];
        let mut attributes: Vec<String> = untyped["attributes"]
            .as_array()
            .expect("attributes")
            .iter()
            .map(|a| a.as_str().expect("a name").to_string())
            .collect();
        attributes.sort();
        let mut nodes: Vec<Vec<String>> = untyped["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .map(|n| {
                n.as_array()
                    .expect("a path")
                    .iter()
                    .map(|s| s.as_str().expect("a segment").to_string())
                    .collect()
            })
            .collect();
        nodes.sort();
        let extra = &parameters_of(&build, at as u32 + 1).extra;
        let kept_attributes: Vec<String> = extra.attributes().map(|(n, _)| n.to_string()).collect();
        let kept_nodes: Vec<Vec<String>> = extra.nodes().map(|n| n.path.clone()).collect();
        assert_eq!(kept_attributes.len(), 12, "twelve attributes are not named");
        assert_eq!(
            kept_attributes,
            attributes,
            "the attributes kept for laser {}",
            at + 1
        );
        assert_eq!(kept_nodes, nodes, "the nodes kept for laser {}", at + 1);
    }
}

#[test]
fn the_formula_images_are_kept_in_extra_and_the_correction_grid_is_typed_and_not_kept() {
    let build = build_of_the_miniature();
    let at = |name: &str| vec![layout::CALIBRATION_DATA.to_string(), name.to_string()];
    // the two formula images beside the calibrations: true-colour datasets of unsigned bytes, described by their extents and their four HDF5 image attributes, and not read
    for (name, extents) in [
        ("BitsToMillimetersFormula", [660u64, 692, 3]),
        ("VoltsToWattsFormula", [771, 1048, 3]),
    ] {
        let node = build
            .extra
            .node(&at(name))
            .unwrap_or_else(|| panic!("{name} is kept"));
        assert_eq!(node.kind, NodeKind::Dataset, "{name}");
        assert_eq!(node.dims.as_slice(), extents, "{name}");
        let attributes: Vec<&str> = node.attributes().map(|(n, _)| n).collect();
        assert_eq!(
            attributes,
            [
                "CLASS",
                "DISPLAY_ORIGIN",
                "IMAGE_SUBCLASS",
                "IMAGE_WHITE_IS_ZERO"
            ],
            "{name}"
        );
    }
    // the calibrations themselves became snapshots, so they are not kept as well
    for number in [1, 2] {
        assert!(build
            .extra
            .node(&at(&format!("{}{number}", layout::CALIBRATION_PREFIX)))
            .is_none());
    }
    // nothing else of the calibration group is kept: the two images, and no attribute of the group
    let kept: Vec<Vec<String>> = build.extra.nodes().map(|n| n.path.clone()).collect();
    assert_eq!(
        kept,
        [at("BitsToMillimetersFormula"), at("VoltsToWattsFormula")]
    );
    // the correction grid of each laser is typed since IP-24 phase 24B: the snapshot lists it (`v16_grids.rs`), and it is no longer an untyped node of the snapshot's `extra`
    for number in [1, 2] {
        let parameters = parameters_of(&build, number);
        assert_eq!(parameters.grids.len(), 1, "laser {number}");
        assert!(
            parameters
                .extra
                .node(&["CorrectionData".to_string()])
                .is_none(),
            "the grid of laser {number} is typed and kept as well"
        );
    }
}

// ---------------------------------------------------------------- the values

#[test]
fn a_number_stored_as_int16_int32_or_float32_is_a_number() {
    let build = with_lasers("v16-number-widths", &[1], |mount| {
        let group = calibration(mount, 1);
        put_scalar(&group, layout::SCANNER_FIELD_SIZE_X, 600i16);
        put_scalar(&group, layout::SCANNER_FIELD_SIZE_Y, 650i32);
        put_scalar(&group, layout::SCANNER_FIELD_SIZE_Z, 1.5f32);
        put_scalar(&group, layout::SCANNER_OFFSET_X, -3i16);
        put_array(&group, layout::SCANNER_OFFSET_Y, &[4i32]);
        put_scalar(&group, layout::SCALING_X, 1i32);
        put_scalar(&group, layout::SCALING_Y, 1.25f32);
        put_scalar(&group, layout::MAJOR_AXIS_ANGLE, 180u8);
        put_text(&group, layout::LASER_MODEL, "a laser");
    })
    .expect("the build is read");
    let parameters = parameters_of(&build, 1);
    for (name, expected) in [
        ("scanner.field_size_x", 600.0),
        ("scanner.field_size_y", 650.0),
        ("scanner.field_size_z", 1.5),
        ("scanner.offset_x", -3.0),
        ("scanner.offset_y", 4.0),
        ("scanner.scaling_x", 1.0),
        ("scanner.scaling_y", 1.25),
        ("optics.major_axis_angle", 180.0),
    ] {
        assert_eq!(number(parameters, name), Some(expected), "{name}");
    }
    // V16 states no unit: a quantity's unit is unknown, and a plain number has none
    match value(parameters, "scanner.field_size_x") {
        Some(ParameterValue::Quantity(q)) => assert_eq!(q.unit_source, UnitSource::Unknown),
        other => panic!("expected a quantity, got {other:?}"),
    }
    assert!(matches!(
        value(parameters, "scanner.scaling_x"),
        Some(ParameterValue::Plain(_))
    ));
    assert_eq!(
        value(parameters, "light_source.model"),
        Some(ParameterValue::Text("a laser".into()))
    );
    // only what the file holds is given: nothing is made up for an attribute that is not there
    assert!(value(parameters, "scanner.model").is_none());
    assert!(value(parameters, "optics.rayleigh_length_major").is_none());
}

#[test]
fn the_calibration_of_a_laser_is_found_by_its_number() {
    // laser 2 and laser 5 are paired with calibrations 2 and 5, in whatever order the file made them, and laser 3's calibration is not there
    let build = with_lasers("v16-pairing", &[2, 5, 3], |mount| {
        put_text(&calibration(mount, 5), layout::LASER_MODEL, "five");
        put_text(&calibration(mount, 2), layout::LASER_MODEL, "two");
    })
    .expect("the build is read");
    let model = |n: u32| value(parameters_of(&build, n), "light_source.model");
    assert_eq!(model(2), Some(ParameterValue::Text("two".into())));
    assert_eq!(model(5), Some(ParameterValue::Text("five".into())));
    // a laser the file has no calibration for has none, and says nothing of it
    let third = laser_of(&build, 3);
    assert!(third.parameters.is_none());
    assert!(third.origins.get("parameters").is_none());
    // and a calibration of a laser that the build does not have is kept where it is
    let build = with_lasers("v16-calibration-without-laser", &[1], |mount| {
        put_text(&calibration(mount, 1), layout::LASER_MODEL, "one");
        put_text(&calibration(mount, 7), layout::LASER_MODEL, "seven");
    })
    .expect("the build is read");
    let node = build
        .extra
        .node(&[
            layout::CALIBRATION_DATA.to_string(),
            format!("{}7", layout::CALIBRATION_PREFIX),
        ])
        .expect("the calibration of a laser that is not there is kept");
    assert!(node.attribute(layout::LASER_MODEL).is_some());
}

// ---------------------------------------------------------------- a calibration that cannot be read

#[test]
fn a_calibration_attribute_that_cannot_be_read_is_recorded_not_read() {
    // laser 1's field size is text where a number belongs: the laser has no snapshot, the reason is on the laser, the whole calibration is kept, and laser 2 is read
    let build = with_lasers("v16-calibration-damage", &[1, 2], |mount| {
        let first = calibration(mount, 1);
        put_text(&first, layout::SCANNER_FIELD_SIZE_X, "wide");
        put_text(&first, layout::LASER_MODEL, "one");
        put_text(&calibration(mount, 2), layout::LASER_MODEL, "two");
    })
    .expect("the build opens");
    let damaged = laser_of(&build, 1);
    assert!(damaged.parameters.is_none());
    match damaged.origins.get("parameters") {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, NotReadCause::Fault);
            assert!(reason.contains(layout::SCANNER_FIELD_SIZE_X), "{reason}");
        }
        other => panic!("expected a reason, got {other:?}"),
    }
    // nothing is dropped: the calibration that could not be read is kept whole, with its attributes as the file has them
    let node = build
        .extra
        .node(&[
            layout::CALIBRATION_DATA.to_string(),
            format!("{}1", layout::CALIBRATION_PREFIX),
        ])
        .expect("the calibration is kept");
    assert!(node.attribute(layout::LASER_MODEL).is_some());
    assert!(node.attribute(layout::SCANNER_FIELD_SIZE_X).is_some());
    // the other laser is not affected
    assert_eq!(
        value(parameters_of(&build, 2), "light_source.model"),
        Some(ParameterValue::Text("two".into()))
    );
}

#[test]
fn a_calibration_that_is_not_a_group_is_reported_on_each_laser_and_kept() {
    // `Laser_Calibration_1` is a dataset and the whole calibration group is a dataset in the second case: each is damage of the file, reported on the lasers that it concerns
    let build = with_lasers("v16-calibration-dataset", &[1, 2], |mount| {
        let data = mount
            .create_group(layout::CALIBRATION_DATA)
            .expect("the calibration group");
        data.new_dataset::<f32>()
            .shape(3)
            .create(format!("{}1", layout::CALIBRATION_PREFIX).as_str())
            .expect("a dataset where a calibration belongs");
        put_text(&calibration(mount, 2), layout::LASER_MODEL, "two");
    })
    .expect("the build opens");
    assert!(laser_of(&build, 1).parameters.is_none());
    assert!(matches!(
        laser_of(&build, 1).origins.get("parameters"),
        Some(Origin::NotRead { .. })
    ));
    assert!(parameters_of(&build, 2).values().len() == 1);
    assert!(build
        .extra
        .node(&[
            layout::CALIBRATION_DATA.to_string(),
            format!("{}1", layout::CALIBRATION_PREFIX)
        ])
        .is_some());

    let build = with_lasers("v16-calibration-group-dataset", &[1, 2], |mount| {
        mount
            .new_dataset::<f32>()
            .shape(2)
            .create(layout::CALIBRATION_DATA)
            .expect("a dataset where the calibration group belongs");
    })
    .expect("the build opens");
    for number in [1, 2] {
        let laser = laser_of(&build, number);
        assert!(laser.parameters.is_none());
        assert!(matches!(
            laser.origins.get("parameters"),
            Some(Origin::NotRead { .. })
        ));
    }
    assert!(build
        .extra
        .node(&[layout::CALIBRATION_DATA.to_string()])
        .is_some());
}

// ---------------------------------------------------------------- the mount's bit resolutions

#[test]
fn the_mounts_bit_resolutions_are_given_to_each_lasers_three_galvos_with_the_global_note() {
    let build = with_lasers("v16-bit-resolutions", &[1, 2], |mount| {
        put_text(mount, layout::ACTUAL_BIT_RESOLUTION, "20");
        put_text(mount, layout::COMMANDED_BIT_RESOLUTION, "24");
        for n in [1, 2] {
            put_text(&calibration(mount, n), layout::LASER_MODEL, "a laser");
        }
    })
    .expect("the build is read");
    for number in [1, 2] {
        let parameters = parameters_of(&build, number);
        for name in BIT_NAMES {
            let expected = if name.contains("commanded") { 24 } else { 20 };
            assert_eq!(
                value(parameters, name),
                Some(ParameterValue::Whole(expected)),
                "{name} of laser {number}"
            );
            assert_eq!(
                parameters.origins.get(name),
                Some(&Origin::Other(BIT_RESOLUTION_GLOBAL.to_string())),
                "{name} of laser {number}"
            );
        }
    }
    // the mount's attributes are taken: they are not kept as well
    assert!(build
        .extra
        .attribute(layout::ACTUAL_BIT_RESOLUTION)
        .is_none());
    assert!(build
        .extra
        .attribute(layout::COMMANDED_BIT_RESOLUTION)
        .is_none());
    // an integer is a whole number too, as the independent script has it
    let integer = with_lasers("v16-bit-integer", &[1], |mount| {
        put_scalar(mount, layout::ACTUAL_BIT_RESOLUTION, 20i32);
        put_array(mount, layout::COMMANDED_BIT_RESOLUTION, &[18u16]);
        put_text(&calibration(mount, 1), layout::LASER_MODEL, "a laser");
    })
    .expect("the build is read");
    let parameters = parameters_of(&integer, 1);
    assert_eq!(
        value(parameters, "galvo_y.actual_bit_resolution"),
        Some(ParameterValue::Whole(20))
    );
    assert_eq!(
        value(parameters, "galvo_z.commanded_bit_resolution"),
        Some(ParameterValue::Whole(18))
    );
}

#[test]
fn a_bit_resolution_that_is_not_a_whole_number_is_reported_and_kept() {
    // the independent script stops on each of these; the library reports it on the galvos' values, and the attribute stays on the mount
    for (n, text) in ["twenty", "20.0", "-1", " 20", ""].into_iter().enumerate() {
        let build = with_lasers(&format!("v16-bit-bad-{n}"), &[1], |mount| {
            put_text(mount, layout::ACTUAL_BIT_RESOLUTION, text);
            put_text(mount, layout::COMMANDED_BIT_RESOLUTION, "20");
            put_text(&calibration(mount, 1), layout::LASER_MODEL, "a laser");
        })
        .expect("the build opens");
        let parameters = parameters_of(&build, 1);
        for galvo in ["galvo_x", "galvo_y", "galvo_z"] {
            let name = format!("{galvo}.actual_bit_resolution");
            assert!(value(parameters, &name).is_none(), "{text:?}: {name}");
            match parameters.origins.get(&name) {
                Some(Origin::NotRead { cause, reason }) => {
                    assert_eq!(*cause, NotReadCause::Fault);
                    assert!(reason.contains(layout::ACTUAL_BIT_RESOLUTION), "{reason}");
                }
                other => panic!("{text:?}: expected a reason, got {other:?}"),
            }
        }
        // the other bit resolution is read, and the one that was not stays on the mount as the file wrote it
        assert_eq!(
            value(parameters, "galvo_x.commanded_bit_resolution"),
            Some(ParameterValue::Whole(20))
        );
        assert!(build
            .extra
            .attribute(layout::ACTUAL_BIT_RESOLUTION)
            .is_some());
        assert!(build
            .extra
            .attribute(layout::COMMANDED_BIT_RESOLUTION)
            .is_none());
    }
    // a float is not a whole number even where it is whole
    let build = with_lasers("v16-bit-float", &[1], |mount| {
        put_scalar(mount, layout::ACTUAL_BIT_RESOLUTION, 20.0f32);
        put_text(&calibration(mount, 1), layout::LASER_MODEL, "a laser");
    })
    .expect("the build opens");
    assert!(value(parameters_of(&build, 1), "galvo_x.actual_bit_resolution").is_none());
    assert!(build
        .extra
        .attribute(layout::ACTUAL_BIT_RESOLUTION)
        .is_some());
}

#[test]
fn bit_resolutions_that_no_laser_has_stay_on_the_mount() {
    // a build with no calibration has no snapshot to give them to, and a laser without a calibration is given none
    let build = with_lasers("v16-bit-no-calibration", &[1, 2], |mount| {
        put_text(mount, layout::ACTUAL_BIT_RESOLUTION, "20");
        put_text(mount, layout::COMMANDED_BIT_RESOLUTION, "20");
    })
    .expect("the build is read");
    assert!(build.lasers().iter().all(|l| l.parameters.is_none()));
    assert!(build
        .extra
        .attribute(layout::ACTUAL_BIT_RESOLUTION)
        .is_some());
    assert!(build
        .extra
        .attribute(layout::COMMANDED_BIT_RESOLUTION)
        .is_some());
    // one laser with a calibration and one without: the first has the values, the second has no snapshot and no note
    let build = with_lasers("v16-bit-one-calibration", &[1, 2], |mount| {
        put_text(mount, layout::ACTUAL_BIT_RESOLUTION, "20");
        put_text(&calibration(mount, 1), layout::LASER_MODEL, "a laser");
    })
    .expect("the build is read");
    assert_eq!(
        value(parameters_of(&build, 1), "galvo_x.actual_bit_resolution"),
        Some(ParameterValue::Whole(20))
    );
    assert!(laser_of(&build, 2).parameters.is_none());
    assert!(laser_of(&build, 2).origins.is_empty());
    // the attribute was given, so it is taken from the mount
    assert!(build
        .extra
        .attribute(layout::ACTUAL_BIT_RESOLUTION)
        .is_none());
}

// ---------------------------------------------------------------- V16's whole-number reader (it stays in V16 until a second version needs it: D12-12)

#[test]
fn a_whole_number_is_digits_or_an_integer_and_nothing_else() {
    let on_disk = temp_h5("v16-whole", |g| {
        put_text(g, "digits", "20");
        put_text(g, "zeros", "007");
        put_scalar(g, "int16", -5i16);
        put_array(g, "one_u64", &[9u64]);
        put_scalar(g, "float", 20.0f64);
        put_text(g, "sign", "-1");
        put_text(g, "plus", "+1");
        put_text(g, "space", "20 ");
        put_text(g, "point", "2.0");
        put_text(g, "empty", "");
        put_text(g, "fullwidth", "\u{FF12}\u{FF10}");
        put_text(g, "arabic", "\u{0662}\u{0660}");
        put_text(g, "huge", "99999999999999999999");
        put_array(g, "several", &[1i32, 2]);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let g = file.group("Mount").expect("the mount");
    let read = |name: &str| optional_whole_number(&g, name);
    assert_eq!(read("digits").unwrap(), Some(20));
    assert_eq!(read("zeros").unwrap(), Some(7));
    assert_eq!(read("int16").unwrap(), Some(-5));
    assert_eq!(read("one_u64").unwrap(), Some(9));
    assert_eq!(read("absent").unwrap(), None);
    for name in [
        "float",
        "sign",
        "plus",
        "space",
        "point",
        "empty",
        "fullwidth",
        "arabic",
        "several",
    ] {
        let error = read(name).expect_err(name);
        assert!(
            matches!(error.kind(), ErrorKind::Corrupt { .. }),
            "{name}: {:?}",
            error.kind()
        );
    }
    // the digits that are not a number the width can hold are `OutOfWidth`, never a wrapped number
    assert!(matches!(
        read("huge").expect_err("huge").kind(),
        ErrorKind::OutOfWidth { .. }
    ));
    // the error names the attribute that it refuses
    assert!(read("point").unwrap_err().to_string().contains("point"));
}
