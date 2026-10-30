//! The other V17 files (IP-13 phase 13E, tasks T13.7 and T13.9; decisions D13-16 and D13-22): the mock capture, the 2026-10-15 machine build, the March build and
//! the special-layer file.
//!
//! One tolerant reader reads all of them (D13-2), and each conforms to its whole expectation file. The gate (`v17_gate.rs`) holds the files to that; this file
//! holds what is special about each one: what the reader keeps beside what it types, and what a file says that another does not.

use build_log_conformance::{check, expectations_dir, Expectation};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    AttrValue, BuildMeta, ExtraAttribute, LaserKey, LaserMeta, LayerId, Presence, StoredClass,
};
use build_log_testkit::{miniature, miniature_path};
use hdf5::types::TypeDescriptor;

fn build_of(file: &str) -> BuildMeta {
    let opened = open(production(), miniature_path(file)).expect("the file opens");
    (*opened.source().build().expect("the metadata")).clone()
}

fn conforms(file: &str, json: &str) -> BuildMeta {
    let build = build_of(file);
    let expectation =
        Expectation::load(&expectations_dir().join(json)).expect("an expectation file");
    let found = check(&build, &expectation);
    assert!(
        found.is_empty(),
        "{file} does not conform:\n  {}",
        found.join("\n  ")
    );
    build
}

fn laser(build: &BuildMeta, number: u32) -> &LaserMeta {
    build
        .laser(&LaserKey::new(format!("Laser_{number}"), number))
        .expect("the laser")
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

fn attribute<'a>(build: &'a BuildMeta, name: &str) -> Option<&'a ExtraAttribute> {
    build.extra.attribute(name)
}

// ---------------------------------------------------------------- the current flavor: the mock capture and the 2026-10-15 build

#[test]
fn the_mock_capture_conforms() {
    let build = conforms(miniature::V17_MOCK, "v17_mock.json");
    // its `Configuration` group is empty and is kept, beside the mount; the capture holds no OPC UA groups
    let configuration = build
        .extra
        .node(&path(&["..", "Configuration"]))
        .expect("the empty configuration group is kept");
    assert_eq!(configuration.attributes().count(), 0);
    assert!(
        build
            .extra
            .nodes()
            .all(|n| n.path.get(1).map(String::as_str) != Some("Global_Sensors")),
        "the mock capture has no OPC UA groups"
    );
}

#[test]
fn the_2026_10_15_file_conforms_with_its_second_laser_recorded_empty() {
    let build = conforms(miniature::V17_OCT15, "v17_oct15.json");
    let first = laser(&build, 1).layers.get(LayerId(1)).expect("laser 1");
    let second = laser(&build, 2).layers.get(LayerId(1)).expect("laser 2");
    assert!(matches!(first.presence, Presence::Recorded { .. }));
    // recorded and empty is not "not recorded": the group is there, with its own times, and holds no samples
    assert_eq!(second.presence, Presence::RecordedEmpty);
    assert!(second.time_axis.window.is_some());
}

#[test]
fn a_different_plate_vocabulary_is_kept_in_extra() {
    // the older vocabulary: one dimension and a shape; the newer one: three dimensions, a corner radius and a unit beside each. Neither is typed (it is not in
    // the slice); each file keeps its own, as written, and not the other's
    let older = build_of(miniature::V17_OCT15);
    assert_eq!(
        attribute(&older, "Build_Plate_Dimensions").map(|a| &a.value),
        Some(&AttrValue::Float(vec![280.0]))
    );
    assert_eq!(
        attribute(&older, "Build_Plate_Shape").map(|a| &a.value),
        Some(&AttrValue::Text(vec!["Square".into()]))
    );
    assert!(attribute(&older, "Build_Plate_X_Dimension").is_none());

    let newer = build_of(miniature::V17_MOCK);
    for (name, value) in [
        ("Build_Plate_X_Dimension", 250.0),
        ("Build_Plate_Y_Dimension", 250.0),
        ("Build_Plate_Z_Dimension", 20.0),
        ("Build_Plate_Corner_Radius", 125.0),
    ] {
        assert_eq!(
            attribute(&newer, name).map(|a| &a.value),
            Some(&AttrValue::Float(vec![value])),
            "{name}"
        );
    }
    assert!(attribute(&newer, "Build_Plate_Dimensions").is_none());
}

// ---------------------------------------------------------------- the March flavor

#[test]
fn fixed_length_text_reads_as_text_without_its_padding() {
    // the March build stores its text in fixed-length strings: prove that this file does, and that the value comes out as the text alone
    let file = hdf5::File::open(miniature_path(miniature::V17_MARCH)).expect("the file");
    let stored = file
        .group("Built_Part")
        .expect("the mount")
        .attr("Alloys")
        .expect("the alloys")
        .dtype()
        .expect("its type")
        .to_descriptor()
        .expect("a descriptor");
    let size = match stored {
        TypeDescriptor::FixedAscii(size) | TypeDescriptor::FixedUnicode(size) => size,
        other => panic!(
            "the March file was expected to store the alloys as fixed-length text, not {other:?}"
        ),
    };
    assert!(size > "IN718".len(), "the string is padded: {size} bytes");

    let build = build_of(miniature::V17_MARCH);
    let alloy = build.identity.alloy.as_deref().expect("the alloys");
    assert_eq!(alloy, "IN718");
    for text in [
        Some(alloy),
        build.identity.username.as_deref(),
        build.identity.date.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        assert!(!text.contains('\0') && text == text.trim_end(), "{text:?}");
    }
}

#[test]
fn the_march_file_conforms() {
    let build = conforms(miniature::V17_MARCH, "v17_march.json");
    // an `i64` count, and layer times in microseconds that are kept as the integers they are, with no unit
    assert_eq!(build.layers_declared, Some(2));
    assert!(build
        .layers
        .iter()
        .all(|(_, l)| l.start.as_ref().is_some_and(|t| t.unit.id.is_none())));
    // the March build records no laser times, so no window is claimed
    for number in [1, 2] {
        assert!(laser(&build, number)
            .layers
            .iter()
            .all(|(_, entry)| entry.time_axis.window.is_none()));
    }
}

#[test]
fn the_special_layers_conform_with_laser_2_recorded_empty_on_layers_2007_and_2008() {
    let build = conforms(miniature::V17_SPECIAL_LAYERS, "special_layers_v17.json");
    let second = laser(&build, 2);
    for id in [2007, 2008] {
        assert_eq!(
            second.layers.get(LayerId(id)).expect("the layer").presence,
            Presence::RecordedEmpty,
            "layer {id}"
        );
    }
    for id in [1964, 1965, 1966, 1967, 2006] {
        assert!(
            matches!(
                second.layers.get(LayerId(id)).unwrap().presence,
                Presence::Recorded { .. }
            ),
            "layer {id}"
        );
    }
    assert!(laser(&build, 1)
        .layers
        .iter()
        .all(|(_, entry)| matches!(entry.presence, Presence::Recorded { .. })));
}

#[test]
fn the_declared_count_stays_2110_with_seven_layers_present() {
    let build = build_of(miniature::V17_SPECIAL_LAYERS);
    assert_eq!(build.layers_declared, Some(2110));
    assert_eq!(build.layers.len(), 7);
}

#[test]
fn march_power_min_and_max_are_kept_in_extra_as_recorded_with_their_stored_size() {
    // The old capture software wrote each limit as a string: `Power_Min` as the eight bytes of a number and a terminator (nine bytes), `Power_Max` as one
    // byte, the terminator alone, so the file holds nothing for it. The library keeps both as the file has them and decodes neither (decision D13-16).
    for file in [miniature::V17_MARCH, miniature::V17_SPECIAL_LAYERS] {
        let build = build_of(file);
        for (train, minimum) in [
            (
                1,
                AttrValue::Bytes(vec![vec![0xCD, 0xCC, 0xCC, 0xCC, 0xCC, 0x6C, 0x59, 0x40]]),
            ),
            // these eight bytes happen to be valid text, so they are kept as the text they spell
            (2, AttrValue::Text(vec!["fffff&Y@".into()])),
        ] {
            // laser N uses train N: the limits are kept in the laser's parameters, by their path below the train
            let node = laser(&build, train)
                .parameters
                .as_ref()
                .expect("the parameters of the laser")
                .extra
                .node(&path(&["Light_Source"]))
                .unwrap_or_else(|| panic!("{file}: the light source of train {train} is kept"));
            let min = node.attribute("Power_Min").expect("Power_Min is kept");
            assert_eq!(min.value, minimum, "{file}, train {train}");
            assert_eq!(min.stored_as.fixed_bytes, Some(9), "{file}, train {train}");
            let max = node.attribute("Power_Max").expect("Power_Max is kept");
            assert_eq!(max.value, AttrValue::Text(vec![String::new()]));
            assert_eq!(max.stored_as.fixed_bytes, Some(1), "{file}, train {train}");
            assert_eq!(max.stored_as.class, StoredClass::Text);
            assert!(min.issue.is_none() && max.issue.is_none());
        }
    }
}
