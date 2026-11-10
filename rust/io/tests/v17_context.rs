//! The build's context in the V17 files (IP-02 phase 2V, task T02.30; decisions D02-20 and D02-21).
//!
//! * The machine, the material, the starting height and the build plate are typed: the seven fields that every flavor carries, the plate in the vocabulary of its file (three dimensions and a
//!   corner radius with their units, or one dimension and a shape), and **neither vocabulary is converted into the other**.
//! * A value is as written: a text is text (an empty text is a recorded empty text), a number is a number whether the file stored it as a scalar, an array of one, a float or an integer, and
//!   a quantity has its companion attribute's unit or an unknown one.
//! * **A placeholder is not a value** (J4): `0` for a plate's thickness, weight and older dimension is not recorded and says what the file wrote; a starting height of `0` and a dimension of `280` are
//!   recorded as written.
//! * **An attribute that cannot be read is reported on itself, the build opens, and the attribute stays in `extra`.**
//!
//! The conformance of every file (`Part::Context`) is held by the tests of `v17_gate.rs` and `v17_files.rs`, and that nothing is dropped by the walk of `v17_gate.rs`.

mod common;

use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    validate_build, BuildContext, BuildMeta, FindingCode, NotReadCause, Origin, UnitSource,
    PLATE_AS_AXES, PLATE_AS_DIMENSION,
};
use build_log_testkit::{miniature, miniature_path};
use common::{monitoring, put_array, put_scalar, put_text, temp_h5, TempH5};

/// What a miniature's build says, through the registry.
fn build_of(file: &str) -> BuildMeta {
    let opened = open(production(), miniature_path(file)).expect("the file opens");
    (*opened.source().build().expect("the metadata")).clone()
}

/// A V17 mount with only what the build level requires, that `write` adds to, read by the adapter.
fn made(label: &str, write: impl FnOnce(&hdf5::Group)) -> BuildMeta {
    let on_disk: TempH5 = temp_h5(label, |mount| {
        put_array(mount, layout::MARKER, &[17i32]);
        put_text(mount, layout::BUILD_ID, "a build");
        monitoring(mount);
        write(mount);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    v17::read::read_build(&mount).expect("the build level reads")
}

fn value(build: &BuildMeta, name: &str) -> Option<build_log_model::ParameterValue> {
    build
        .context
        .values()
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

fn number(build: &BuildMeta, name: &str) -> Option<f64> {
    match value(build, name)? {
        build_log_model::ParameterValue::Quantity(q) => Some(q.value),
        other => panic!("`{name}` is {other:?}, not a quantity"),
    }
}

fn text(build: &BuildMeta, name: &str) -> Option<String> {
    match value(build, name)? {
        build_log_model::ParameterValue::Text(t) => Some(t),
        other => panic!("`{name}` is {other:?}, not text"),
    }
}

fn origin(build: &BuildMeta, name: &str) -> Option<Origin> {
    build.origins.get(&BuildContext::origin_name(name)).cloned()
}

fn raw(text: &str) -> Option<Origin> {
    Some(Origin::PlaceholderDropped { raw: text.into() })
}

// ---------------------------------------------------------------- the files

#[test]
fn the_seven_fields_of_every_flavor_become_context_fields() {
    // every V17 miniature carries the machine, the scanner, the material's parameter ID, the blade and the starting height; the plate's thickness and weight are real in the March build
    let tauri = [
        miniature::V17_MOCK,
        miniature::V17_TRIGGERED_2026_10_19,
        miniature::V17_2026_08_18_IMAGES,
        miniature::V17_OCT15,
    ];
    let march = [miniature::V17_MARCH, miniature::V17_SPECIAL_LAYERS];
    for file in tauri.into_iter().chain(march) {
        let build = build_of(file);
        for name in [
            "machine_serial",
            "scanner_id",
            "material_parameter_id",
            "recoater_blade",
        ] {
            assert!(text(&build, name).is_some(), "{file}: {name}");
        }
        assert!(number(&build, "starting_height").is_some(), "{file}");
    }
    // the words are the file's own: tauri's blade and the March build's, and the parameter ID that tauri leaves empty and that March writes as `na`
    assert_eq!(
        text(&build_of(miniature::V17_MOCK), "recoater_blade").as_deref(),
        Some("Standard")
    );
    let march_build = build_of(miniature::V17_MARCH);
    assert_eq!(
        text(&march_build, "recoater_blade").as_deref(),
        Some("Soft")
    );
    assert_eq!(
        text(&march_build, "material_parameter_id").as_deref(),
        Some("na")
    );
    // the March build's thickness and weight are real
    assert_eq!(number(&march_build, "plate.thickness"), Some(19.0));
    assert_eq!(number(&march_build, "plate.weight"), Some(10.0));
}

#[test]
fn an_empty_text_is_a_recorded_empty_text() {
    // tauri writes an empty parameter ID: it is a recorded empty text, and not a placeholder
    let build = build_of(miniature::V17_MOCK);
    assert_eq!(text(&build, "material_parameter_id").as_deref(), Some(""));
    assert_eq!(origin(&build, "material_parameter_id"), None);
}

#[test]
fn the_current_plate_vocabulary_becomes_axes_with_their_units() {
    let build = build_of(miniature::V17_MOCK);
    let plate = &build.context.plate;
    assert_eq!(
        [&plate.x, &plate.y, &plate.z, &plate.corner_radius].map(|q| q.as_ref().map(|q| q.value)),
        [Some(250.0), Some(250.0), Some(20.0), Some(125.0)]
    );
    // each has the unit that its companion attribute spells
    for quantity in [&plate.x, &plate.y, &plate.z, &plate.corner_radius] {
        let quantity = quantity.as_ref().unwrap();
        assert_eq!(quantity.unit_source, UnitSource::FromFile);
        assert_eq!(quantity.unit.id.as_deref(), Some("mm"));
    }
    // the other vocabulary is not there, and the origin says which one the file used
    assert!(plate.dimension.is_none() && plate.shape.is_none());
    assert_eq!(
        origin(&build, "plate"),
        Some(Origin::Other(PLATE_AS_AXES.to_string()))
    );
    // the directions are text as written
    assert_eq!(text(&build, "gas_flow_direction").as_deref(), Some("Y+"));
    assert_eq!(text(&build, "recoat_direction").as_deref(), Some("X+"));
}

#[test]
fn the_older_plate_vocabulary_becomes_a_dimension_and_a_shape() {
    let build = build_of(miniature::V17_OCT15);
    let plate = &build.context.plate;
    assert_eq!(plate.shape.as_deref(), Some("Square"));
    assert_eq!(plate.dimension.as_ref().map(|q| q.value), Some(280.0));
    // no file states the unit of the dimension, and nothing is guessed
    assert_eq!(
        plate.dimension.as_ref().map(|q| q.unit_source),
        Some(UnitSource::Unknown)
    );
    // it is never split into an X and a Y
    assert!(
        plate.x.is_none()
            && plate.y.is_none()
            && plate.z.is_none()
            && plate.corner_radius.is_none()
    );
    assert_eq!(
        origin(&build, "plate"),
        Some(Origin::Other(PLATE_AS_DIMENSION.to_string()))
    );
    // the files of one vocabulary have none of the other's values, and the March build's older vocabulary has a dimension that is not recorded
    assert_eq!(
        origin(&build_of(miniature::V17_MARCH), "plate"),
        Some(Origin::Other(PLATE_AS_DIMENSION.to_string()))
    );
}

#[test]
fn a_dimension_of_280_is_recorded_as_written() {
    // tauri's older output writes 280 with the shape `Square`: nothing in the file tells it from a real plate, so it is recorded and not dropped
    for file in [miniature::V17_OCT15, miniature::V17_2026_08_18_IMAGES] {
        let build = build_of(file);
        assert_eq!(number(&build, "plate.dimension"), Some(280.0), "{file}");
        assert_eq!(origin(&build, "plate.dimension"), None, "{file}");
    }
}

#[test]
fn a_zero_thickness_weight_or_dimension_is_not_recorded_and_keeps_its_raw_value() {
    // tauri writes 0 for the plate's thickness and weight when it does not know them
    let build = build_of(miniature::V17_MOCK);
    assert!(build.context.plate.thickness.is_none() && build.context.plate.weight.is_none());
    assert_eq!(origin(&build, "plate.thickness"), raw("0.0"));
    assert_eq!(origin(&build, "plate.weight"), raw("0.0"));
    // the March build writes 0.0 for the dimension, and real values for the thickness and the weight
    let march = build_of(miniature::V17_MARCH);
    assert!(march.context.plate.dimension.is_none());
    assert_eq!(origin(&march, "plate.dimension"), raw("0.0"));
    assert_eq!(origin(&march, "plate.thickness"), None);
    // a placeholder is not a placeholder where the file has a real value (checked on a made file: a thickness of 1.0 is possible in V17, where the 1.0 is V16's placeholder)
    let made = made("one", |mount| {
        put_scalar(mount, layout::BUILD_PLATE_THICKNESS, 1.0f64);
        put_array(mount, layout::BUILD_PLATE_WEIGHT, &[0i32]);
    });
    assert_eq!(number(&made, "plate.thickness"), Some(1.0));
    // and a 0 stored as an integer array of one is the placeholder, with the same raw spelling
    assert!(made.context.plate.weight.is_none());
    assert_eq!(origin(&made, "plate.weight"), raw("0.0"));
}

#[test]
fn a_starting_height_of_zero_is_recorded() {
    // 0 is a possible starting height, so it is a value, in every file that writes it
    for file in [miniature::V17_MOCK, miniature::V17_MARCH] {
        let build = build_of(file);
        assert_eq!(number(&build, "starting_height"), Some(0.0), "{file}");
        assert_eq!(origin(&build, "starting_height"), None, "{file}");
    }
}

#[test]
fn a_unit_attribute_gives_the_quantity_its_unit_and_none_leaves_it_unknown() {
    let build = build_of(miniature::V17_MOCK);
    let x = build.context.plate.x.as_ref().unwrap();
    assert_eq!(
        (x.unit_source, x.unit.spelling.as_deref()),
        (UnitSource::FromFile, Some("mm"))
    );
    // no file states the unit of the starting height
    let height = build.context.starting_height.as_ref().unwrap();
    assert_eq!(height.unit_source, UnitSource::Unknown);
    assert!(height.unit.spelling.is_none());
}

#[test]
fn a_number_stored_as_a_scalar_or_an_array_of_one_reads_the_same_for_the_context() {
    // a float or an integer, a scalar or an array of one element: one number
    let build = made("forms", |mount| {
        put_scalar(mount, layout::BUILD_PLATE_THICKNESS, 19.0f64);
        put_array(mount, layout::BUILD_PLATE_WEIGHT, &[10.0f32]);
        put_array(mount, layout::STARTING_HEIGHT, &[3i32]);
        put_scalar(mount, layout::BUILD_PLATE_DIMENSIONS, 280u16);
    });
    assert_eq!(number(&build, "plate.thickness"), Some(19.0));
    assert_eq!(number(&build, "plate.weight"), Some(10.0));
    assert_eq!(number(&build, "starting_height"), Some(3.0));
    assert_eq!(number(&build, "plate.dimension"), Some(280.0));
}

#[test]
fn a_file_without_any_context_has_an_empty_one_and_no_origin_about_it() {
    let build = made("none", |_| {});
    assert!(build.context.is_empty());
    assert!(build
        .origins
        .iter()
        .all(|(name, _)| !name.starts_with("context.")));
}

#[test]
fn a_file_that_gives_both_plate_vocabularies_reads_both_and_says_both() {
    let build = made("both", |mount| {
        put_scalar(mount, layout::BUILD_PLATE_DIMENSIONS, 280.0f32);
        put_text(mount, layout::BUILD_PLATE_SHAPE, "Square");
        put_scalar(mount, layout::BUILD_PLATE_X_DIMENSION, 250.0f64);
        put_text(mount, layout::BUILD_PLATE_X_DIMENSION_UNIT, "mm");
    });
    assert_eq!(number(&build, "plate.dimension"), Some(280.0));
    assert_eq!(number(&build, "plate.x"), Some(250.0));
    assert_eq!(
        origin(&build, "plate"),
        Some(Origin::Other(format!(
            "{PLATE_AS_DIMENSION}; {PLATE_AS_AXES}"
        )))
    );
}

// ---------------------------------------------------------------- what cannot be read

#[test]
fn a_context_attribute_with_several_values_is_reported_on_itself_and_kept() {
    // D13-15: the first of several values is never taken. The value is absent, the origin says why, the build opens, and the attribute stays in `extra`
    let build = made("several", |mount| {
        put_array(mount, layout::STARTING_HEIGHT, &[1.0f32, 2.0]);
        put_scalar(mount, layout::BUILD_PLATE_THICKNESS, 19.0f64);
    });
    assert!(build.context.starting_height.is_none());
    assert!(matches!(
        origin(&build, "starting_height"),
        Some(Origin::NotRead {
            cause: NotReadCause::Fault,
            ..
        })
    ));
    assert!(build.extra.attribute(layout::STARTING_HEIGHT).is_some());
    // the other values are read, and the report of the build says the one that was not as a fault
    assert_eq!(number(&build, "plate.thickness"), Some(19.0));
    assert!(build
        .extra
        .attribute(layout::BUILD_PLATE_THICKNESS)
        .is_none());
    let report = validate_build(&build);
    assert_eq!(report.count_of(FindingCode::NotRead), 1);
}

#[test]
fn a_text_where_a_number_belongs_and_a_number_where_a_text_belongs_are_reported_on_themselves() {
    let build = made("wrong-kind", |mount| {
        put_text(mount, layout::BUILD_PLATE_WEIGHT, "heavy");
        put_scalar(mount, layout::MACHINE_SERIAL, 3.0f32);
        put_text(mount, layout::SCANNER_ID, "a scanner");
    });
    assert!(build.context.plate.weight.is_none() && build.context.machine_serial.is_none());
    for name in ["plate.weight", "machine_serial"] {
        assert!(
            matches!(origin(&build, name), Some(Origin::NotRead { .. })),
            "{name}"
        );
    }
    // both attributes are kept, as they were stored, and the one that was read is not
    assert!(build.extra.attribute(layout::BUILD_PLATE_WEIGHT).is_some());
    assert!(build.extra.attribute(layout::MACHINE_SERIAL).is_some());
    assert!(build.extra.attribute(layout::SCANNER_ID).is_none());
    assert_eq!(text(&build, "scanner_id").as_deref(), Some("a scanner"));
}

#[test]
fn a_unit_that_cannot_be_read_leaves_the_value_unread_and_both_attributes_kept() {
    let build = made("bad-unit", |mount| {
        put_scalar(mount, layout::BUILD_PLATE_X_DIMENSION, 250.0f64);
        put_scalar(mount, layout::BUILD_PLATE_X_DIMENSION_UNIT, 7i32);
    });
    assert!(build.context.plate.x.is_none());
    assert!(matches!(
        origin(&build, "plate.x"),
        Some(Origin::NotRead { .. })
    ));
    assert!(build
        .extra
        .attribute(layout::BUILD_PLATE_X_DIMENSION)
        .is_some());
    assert!(build
        .extra
        .attribute(layout::BUILD_PLATE_X_DIMENSION_UNIT)
        .is_some());
    // the file did use the current vocabulary, whether or not its value could be read
    assert_eq!(
        origin(&build, "plate"),
        Some(Origin::Other(PLATE_AS_AXES.to_string()))
    );
}

#[test]
fn what_is_typed_leaves_extra() {
    // the nineteen attributes that the context types (the fifteen values and the four units) are not also kept: the mock capture's mount keeps none of them
    let build = build_of(miniature::V17_MOCK);
    for attribute in layout::context_attributes() {
        assert!(build.extra.attribute(attribute).is_none(), "{attribute}");
    }
    assert_eq!(layout::context_attributes().len(), 19);
}

#[test]
fn each_dimension_takes_the_unit_of_its_own_companion_attribute() {
    // the miniatures spell every unit `mm`, so only a file made for it can tell one dimension's unit from another's
    let build = made("units", |mount| {
        put_scalar(mount, layout::BUILD_PLATE_X_DIMENSION, 250.0f64);
        put_text(mount, layout::BUILD_PLATE_X_DIMENSION_UNIT, "mm");
        put_scalar(mount, layout::BUILD_PLATE_Y_DIMENSION, 300.0f64);
        put_text(mount, layout::BUILD_PLATE_Y_DIMENSION_UNIT, "um");
        put_scalar(mount, layout::BUILD_PLATE_Z_DIMENSION, 20.0f64);
        put_scalar(mount, layout::BUILD_PLATE_CORNER_RADIUS, 125.0f64);
        put_text(mount, layout::BUILD_PLATE_CORNER_RADIUS_UNIT, "deg");
    });
    let plate = &build.context.plate;
    let unit = |q: &Option<build_log_model::Quantity>| q.as_ref().map(|q| q.unit.spelling.clone());
    assert_eq!(unit(&plate.x), Some(Some("mm".to_string())));
    assert_eq!(unit(&plate.y), Some(Some("um".to_string())));
    assert_eq!(unit(&plate.corner_radius), Some(Some("deg".to_string())));
    // a dimension with no unit attribute has an unknown unit, and the others' units are not lent to it
    assert_eq!(
        plate.z.as_ref().map(|q| q.unit_source),
        Some(UnitSource::Unknown)
    );
}
