//! The marker reader (IP-04 phase 4B, task T04.2, decision D04-2): the version of a mount, in every form that the files hold.
//!
//! The miniatures give the real cases (a scalar in V16, V18 and March, an array of one in the four tauri files; Appendix A of the plan), and files made at test
//! time give the rest: text, other widths, and what cannot be a version.

mod common;

use build_log_io::dispatch::{peek_marker, VersionKey};
use build_log_model::ErrorKind;
use build_log_testkit::{miniature, miniature_path};
use common::{put_array, put_scalar, put_text, temp_h5};
use hdf5::types::FixedAscii;
use hdf5::File;

const MARKER: &[&str] = &["File_Version"];

fn key_of_miniature(name: &str, mount: &str) -> Option<String> {
    let file = File::open(miniature_path(name)).expect("a miniature opens");
    let group = file.group(mount).expect("the mount of the miniature");
    peek_marker(&group, MARKER)
        .expect("the marker is read")
        .map(|k| k.to_string())
}

fn key_of(file: &common::TempH5, names: &[&str]) -> Result<Option<String>, build_log_model::Error> {
    let file = File::open(&file.0).expect("the temporary file opens");
    let group = file.group("Mount").expect("the mount");
    peek_marker(&group, names).map(|k| k.map(|k| k.to_string()))
}

#[test]
fn a_marker_stored_as_a_scalar_or_an_array_of_one_gives_the_same_key() {
    // the miniatures, as Appendix A of the plan lists them
    assert_eq!(
        key_of_miniature(miniature::V16, "Built_Part").as_deref(),
        Some("16")
    );
    assert_eq!(
        key_of_miniature(miniature::V17_MARCH, "Built_Part").as_deref(),
        Some("17")
    );
    assert_eq!(
        key_of_miniature(miniature::V17_SPECIAL_LAYERS, "Built_Part").as_deref(),
        Some("17")
    );
    assert_eq!(
        key_of_miniature(miniature::V18, "Build").as_deref(),
        Some("18")
    );
    for tauri in [
        miniature::V17_MOCK,
        miniature::V17_OCT15,
        miniature::V17_TRIGGERED_2026_10_19,
        miniature::V17_2026_08_18_IMAGES,
    ] {
        assert_eq!(
            key_of_miniature(tauri, "Built_Part").as_deref(),
            Some("17"),
            "{tauri}"
        );
    }
    // and the same number in other forms gives the same key: a scalar, an array of one, other widths
    let forms = temp_h5("marker-forms", |g| {
        put_scalar(g, "scalar_i32", 17_i32);
        put_array(g, "array_i32", &[17_i32]);
        put_scalar(g, "scalar_i64", 17_i64);
        put_scalar(g, "scalar_u8", 17_u8);
        put_array(g, "array_u16", &[17_u16]);
    });
    for name in [
        "scalar_i32",
        "array_i32",
        "scalar_i64",
        "scalar_u8",
        "array_u16",
    ] {
        assert_eq!(
            key_of(&forms, &[name]).expect("a key").as_deref(),
            Some("17"),
            "{name}"
        );
    }
}

#[test]
fn a_marker_stored_as_text_gives_its_text() {
    let texts = temp_h5("marker-text", |g| {
        put_text(g, "mock", "9001");
        put_text(g, "padded", "  17 ");
        put_text(g, "dotted", "1.0");
        put_scalar(
            g,
            "fixed",
            FixedAscii::<8>::from_ascii("17").expect("ascii"),
        );
    });
    assert_eq!(
        key_of(&texts, &["mock"]).expect("a key").as_deref(),
        Some("9001")
    );
    assert_eq!(
        key_of(&texts, &["padded"]).expect("a key").as_deref(),
        Some("17"),
        "spaces are trimmed"
    );
    // "1.0" is a key like any other: it is for the registry to say that nobody registered it
    assert_eq!(
        key_of(&texts, &["dotted"]).expect("a key").as_deref(),
        Some("1.0")
    );
    // a fixed-length string is read as the text it holds, without its zero padding
    assert_eq!(
        key_of(&texts, &["fixed"]).expect("a key").as_deref(),
        Some("17")
    );
}

#[test]
fn a_marker_that_cannot_be_a_version_is_corrupt() {
    let bad = temp_h5("marker-bad", |g| {
        put_scalar(g, "float", 17.0_f64);
        put_array(g, "two", &[17_i32, 18]);
        put_text(g, "empty", "");
        put_text(g, "blank", "   ");
        put_text(g, "control", "17\n18");
        put_scalar(g, "a", 17_i32);
        put_scalar(g, "b", 18_i32);
    });
    for name in ["float", "two", "empty", "blank", "control"] {
        let err = key_of(&bad, &[name]).expect_err(name);
        assert!(
            matches!(err.kind(), ErrorKind::Corrupt { .. }),
            "{name}: {err}"
        );
    }
    // two markers that disagree are not resolved by choosing one
    let err = key_of(&bad, &["a", "b"]).expect_err("17 and 18 in one place");
    assert!(matches!(err.kind(), ErrorKind::Corrupt { .. }), "{err}");
    // two that agree are one marker
    let agree = temp_h5("marker-agree", |g| {
        put_scalar(g, "a", 17_i32);
        put_text(g, "b", "17");
    });
    assert_eq!(
        key_of(&agree, &["a", "b"]).expect("a key").as_deref(),
        Some("17")
    );
}

#[test]
fn a_file_without_the_marker_gives_none() {
    let none = temp_h5("marker-none", |g| put_text(g, "Other", "something else"));
    assert_eq!(key_of(&none, MARKER).expect("no error"), None);
    // a place that nobody registered finds nothing, and neither does an empty list
    assert_eq!(key_of(&none, &["Other_Name"]).expect("no error"), None);
    assert_eq!(key_of(&none, &[]).expect("no error"), None);
}

#[test]
fn versions_sort_as_numbers_and_compare_exactly() {
    let mut keys: Vec<VersionKey> = ["9001", "18", "100", "16", "17"]
        .iter()
        .map(|t| VersionKey::new(t).expect("a key"))
        .collect();
    keys.sort();
    let sorted: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();
    assert_eq!(sorted, ["16", "17", "18", "100", "9001"]);
    assert_ne!(
        VersionKey::new("17").unwrap(),
        VersionKey::new("17.0").unwrap(),
        "there is no range or rounding"
    );
    assert_eq!(VersionKey::new(" 17\0\0").unwrap().as_str(), "17");
}
