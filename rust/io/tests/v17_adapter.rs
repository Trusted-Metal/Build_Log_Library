//! The V17 adapter, the build level (IP-13 phase 13B, tasks T13.2 and T13.3; decisions D13-2, D13-8 and D13-15).
//!
//! * The adapter is registered under `"17"`, with the place of its marker, no capability beyond the metadata, a reader, and a writer and patcher declared
//!   unsupported with their reasons.
//! * A value is read as one value in whichever form V17 stored it (a scalar or an array of one, any width, variable-length or fixed-length text), an absent
//!   attribute is `None`, and an attribute with several values where one belongs is `Corrupt`. The small files are made at test time with the binding.
//! * The build level of the gate file conforms to its expectation file (`Part::Build`), and nothing on the mount is dropped: every attribute of it is typed
//!   or in `extra`.
//! * A request for samples answers with a typed error that is not `NotPresent`, until IP-06.

mod common;

use std::collections::BTreeSet;

use build_log_conformance::{check_parts, expectations_dir, Expectation, Part};
use build_log_io::adapters::v17::{self, layout, values};
use build_log_io::adapters::MarkerPlace;
use build_log_io::dispatch::{open, production};
use build_log_model::{
    AttrShape, AttrValue, Capabilities, ChannelKey, ColumnRequest, ErrorKind, LaserKey, LayerId,
    ReadContext, StoredClass,
};
use build_log_testkit::{miniature, miniature_path};
use common::{put_array, put_scalar, put_text, temp_h5, TempH5};
use hdf5::types::{FixedAscii, FixedUnicode, VarLenUnicode};

/// A file with one group, `Mount`, that `write` fills, opened again; the closure reads from the group.
fn with_mount<T>(
    label: &str,
    write: impl FnOnce(&hdf5::Group),
    read: impl FnOnce(&hdf5::Group) -> T,
) -> T {
    let file_on_disk: TempH5 = temp_h5(label, write);
    let file = hdf5::File::open(&file_on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    read(&mount)
}

fn kind(error: &build_log_model::Error) -> String {
    format!("{:?}", error.kind())
}

// ---------------------------------------------------------------- registration

#[test]
fn the_v17_adapter_is_registered_under_17() {
    assert_eq!(production().supported_versions(), ["17"]);
    let set = production().sets().next().expect("one adapter set");
    assert_eq!(set.key().as_str(), "17");
    assert_eq!(set.name(), "v17");
    assert_eq!(
        set.marker(),
        &MarkerPlace::new(layout::MOUNT, layout::MARKER)
    );
    // nothing is declared that the reader does not return yet (decision D13-7)
    assert_eq!(set.capabilities(), &Capabilities::new());
    assert!(set.reader().is_provided());
    let writer = set.writer().unsupported_reason().expect("a reason");
    assert!(writer.contains("IP-08"), "{writer}");
    let patcher = set.patcher().unsupported_reason().expect("a reason");
    assert!(patcher.contains("IP-11"), "{patcher}");
    // the registration unit is the one the folder hands out
    assert_eq!(v17::adapter_set().key(), set.key());
}

// ---------------------------------------------------------------- the values, in every form V17 stores them

#[test]
fn a_number_stored_as_a_scalar_or_an_array_of_one_reads_the_same() {
    with_mount(
        "v17-numbers",
        |g| {
            put_scalar(g, "i32_scalar", 7i32);
            put_array(g, "i32_one", &[7i32]);
            put_scalar(g, "i64_scalar", 7i64);
            put_array(g, "i64_one", &[7i64]);
            put_scalar(g, "u8_scalar", 7u8);
            put_scalar(g, "u16_scalar", 7u16);
            put_scalar(g, "f32_scalar", 0.03f32);
            put_array(g, "f32_one", &[0.03f32]);
            put_scalar(g, "f64_scalar", 0.03f64);
            put_array(g, "f64_one", &[0.03f64]);
            put_scalar(g, "too_big", u64::MAX);
        },
        |g| {
            for name in [
                "i32_scalar",
                "i32_one",
                "i64_scalar",
                "i64_one",
                "u8_scalar",
                "u16_scalar",
            ] {
                assert_eq!(
                    values::optional_integer(g, name).unwrap(),
                    Some(7),
                    "{name}"
                );
            }
            // a float32 is widened exactly, so the value is the one the file holds and not a rounded one
            for name in ["f32_scalar", "f32_one"] {
                assert_eq!(
                    values::optional_float(g, name).unwrap(),
                    Some(f64::from(0.03f32)),
                    "{name}"
                );
            }
            for name in ["f64_scalar", "f64_one"] {
                assert_eq!(
                    values::optional_float(g, name).unwrap(),
                    Some(0.03),
                    "{name}"
                );
            }
            // a number that does not fit is an error that says so, never a wrong value
            let error = values::optional_integer(g, "too_big").unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::OutOfWidth { .. }),
                "{}",
                kind(&error)
            );
        },
    );
}

#[test]
fn text_stored_as_variable_or_fixed_length_reads_the_same() {
    with_mount(
        "v17-text",
        |g| {
            put_text(g, "variable", "IN718");
            put_array(
                g,
                "variable_one",
                &["IN718".parse::<VarLenUnicode>().unwrap()],
            );
            put_scalar(
                g,
                "fixed_ascii",
                FixedAscii::<9>::from_ascii(b"IN718").unwrap(),
            );
            put_scalar(
                g,
                "fixed_unicode",
                "IN718".parse::<FixedUnicode<9>>().unwrap(),
            );
            put_text(g, "empty", "");
        },
        |g| {
            for name in ["variable", "variable_one", "fixed_ascii", "fixed_unicode"] {
                assert_eq!(
                    values::optional_text(g, name).unwrap().as_deref(),
                    Some("IN718"),
                    "{name}"
                );
            }
            // an empty string is a value (the alloy of the current output is one), and is not an absent attribute
            assert_eq!(
                values::optional_text(g, "empty").unwrap().as_deref(),
                Some("")
            );
            assert_eq!(values::required_text(g, "variable").unwrap(), "IN718");
        },
    );
}

#[test]
fn an_absent_optional_attribute_is_none_and_not_a_default() {
    with_mount(
        "v17-absent",
        |g| {
            // an attribute of no elements holds no value, which is the same as not having one
            g.new_attr::<i32>()
                .shape(0)
                .create("no_elements")
                .expect("an empty attribute");
        },
        |g| {
            assert_eq!(values::optional_integer(g, "missing").unwrap(), None);
            assert_eq!(values::optional_float(g, "missing").unwrap(), None);
            assert_eq!(values::optional_text(g, "missing").unwrap(), None);
            assert_eq!(values::optional_integer(g, "no_elements").unwrap(), None);
            // a text that must be there is an error that names it
            let error = values::required_text(g, "missing").unwrap_err();
            match error.kind() {
                ErrorKind::Corrupt { detail } => assert!(detail.contains("missing"), "{detail}"),
                other => panic!("expected Corrupt, got {other:?}"),
            }
        },
    );
}

#[test]
fn an_attribute_with_several_values_where_one_is_expected_is_not_read_as_its_first() {
    with_mount(
        "v17-several",
        |g| {
            put_array(g, "two_ints", &[1i32, 2]);
            put_array(g, "two_floats", &[1.0f32, 2.0]);
            put_array(
                g,
                "two_texts",
                &[
                    "a".parse::<VarLenUnicode>().unwrap(),
                    "b".parse::<VarLenUnicode>().unwrap(),
                ],
            );
            put_text(g, "text", "not a number");
            put_scalar(g, "number", 7i32);
        },
        |g| {
            for error in [
                values::optional_integer(g, "two_ints").unwrap_err(),
                values::optional_float(g, "two_floats").unwrap_err(),
                values::optional_text(g, "two_texts").unwrap_err(),
            ] {
                match error.kind() {
                    ErrorKind::Corrupt { detail } => {
                        assert!(detail.contains("2 values where one belongs"), "{detail}");
                        assert!(detail.contains("first is not taken"), "{detail}");
                    }
                    other => panic!("expected Corrupt, got {other:?}"),
                }
            }
            // and a value of the wrong kind is just as plain: text where a number belongs, a number where text belongs
            assert!(matches!(
                values::optional_integer(g, "text").unwrap_err().kind(),
                ErrorKind::Corrupt { .. }
            ));
            assert!(matches!(
                values::optional_float(g, "number").unwrap_err().kind(),
                ErrorKind::Corrupt { .. }
            ));
            assert!(matches!(
                values::optional_text(g, "number").unwrap_err().kind(),
                ErrorKind::Corrupt { .. }
            ));
        },
    );
}

// ---------------------------------------------------------------- the gate file

fn gate_path() -> std::path::PathBuf {
    miniature_path(miniature::V17_TRIGGERED_2026_10_19)
}

#[test]
fn the_build_level_of_the_gate_file_conforms() {
    let opened = open(production(), gate_path()).expect("the gate file opens through the registry");
    assert_eq!(opened.key().as_str(), "17");
    assert_eq!(opened.adapter(), "v17");
    let build = opened.source().build().expect("the metadata");
    let expectation = Expectation::load(&expectations_dir().join("v17_triggered_2026_10_19.json"))
        .expect("the expectation file");
    let found = check_parts(&build, &expectation, &[Part::Build]);
    assert!(
        found.is_empty(),
        "the build level does not conform:\n  {}",
        found.join("\n  ")
    );
    // declared and present are counted separately: the file declares one layer, and holds one (the layers have their own tests, `v17_layers.rs`)
    assert_eq!(build.layers_declared, Some(1));
    assert_eq!(build.layers_present(), 1);
}

#[test]
fn nothing_on_the_mount_is_dropped_by_the_build_level() {
    let opened = open(production(), gate_path()).expect("the gate file opens");
    let build = opened.source().build().expect("the metadata");
    // an independent look at the mount: every attribute it has
    let file = hdf5::File::open(gate_path()).expect("the file");
    let mount = file.group(layout::MOUNT).expect("the mount");
    let on_the_mount: BTreeSet<String> = mount.attr_names().unwrap().into_iter().collect();
    let typed: BTreeSet<String> = layout::TYPED_ON_THE_MOUNT
        .iter()
        .map(|s| s.to_string())
        .collect();
    let kept: BTreeSet<String> = build
        .extra
        .attributes()
        .map(|(name, _)| name.to_string())
        .collect();
    assert!(
        on_the_mount.len() > typed.len(),
        "the mount has more attributes than the build level types"
    );
    // an attribute is typed or kept, and never both; the marker is the version itself and is typed
    assert!(
        kept.is_disjoint(&typed),
        "kept twice: {:?}",
        kept.intersection(&typed).collect::<Vec<_>>()
    );
    let accounted: BTreeSet<String> = typed
        .intersection(&on_the_mount)
        .cloned()
        .chain(kept.iter().cloned())
        .collect();
    assert_eq!(
        accounted, on_the_mount,
        "an attribute of the mount is neither typed nor kept"
    );
    // the marker is the version itself: it chose this adapter and is not kept as data that a conversion would carry to another version
    assert!(on_the_mount.contains(layout::MARKER));
    assert!(build.extra.attribute(layout::MARKER).is_none());
    // a kept attribute keeps the form it was stored in: a number that is an array of one is not turned into a scalar
    let height = build
        .extra
        .attribute("Starting_Height")
        .expect("an attribute that the build level does not type");
    assert!(matches!(height.value, AttrValue::Float(ref v) if v.len() == 1));
    assert_eq!(height.stored_as.class, StoredClass::Float);
    assert_eq!(height.stored_as.bits, Some(32));
    assert_eq!(height.stored_as.shape, AttrShape::Array { elements: 1 });
    assert!(height.issue.is_none());
}

// ---------------------------------------------------------------- samples, until IP-06

#[test]
fn a_sample_request_answers_unsupported_until_ip06() {
    let opened = open(production(), gate_path()).expect("the gate file opens");
    let request = ColumnRequest::new(
        LayerId(1),
        Some(LaserKey::new("Laser_1", 1)),
        ChannelKey::new(["Laser_Power"]),
    );
    let error = opened
        .source()
        .read(&request, &ReadContext::new())
        .expect_err("samples are not read yet");
    // the data is in the file, so it is not `NotPresent`: the answer says what is missing and why
    match error.kind() {
        ErrorKind::UnsupportedInVersion { version, feature } => {
            assert_eq!(version, "17");
            assert!(feature.contains("IP-06"), "{feature}");
        }
        other => panic!("expected UnsupportedInVersion, got {other:?}"),
    }
    // and the metadata is still there
    assert!(opened.source().build().is_ok());
}
