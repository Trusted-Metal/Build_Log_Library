//! The build's context, held to the expectation of a miniature, and shown to fail when it is wrong (IP-02 phase 2V, task T02.30; decisions D02-20 and D02-21).
//!
//! * `the_synthetic_context_of_every_expectation_conforms`: a build made from each expectation passes the part, and holds the values the independent script wrote.
//! * The rest hold the check itself: a value that is changed, missing or extra, a unit that is another, a placeholder that was recorded or whose raw spelling changed, the sentence of the
//!   plate's origin, an origin that was not expected, and an expectation that lists no context.

use build_log_conformance::{check_parts, load_all, synthesise, Expectation, Part};
use build_log_model::{BuildContext, BuildIdentity, BuildMeta, Origin, ParameterValue, Quantity};

fn expectation(file: &str) -> Expectation {
    load_all()
        .expect("the expectation files can be read")
        .into_iter()
        .find(|e| e.miniature() == file)
        .unwrap_or_else(|| panic!("an expectation for {file}"))
}

/// What the context part says of the build made from a miniature's expectation after `change`.
fn after(file: &str, change: impl FnOnce(&mut BuildMeta)) -> Vec<String> {
    let expectation = expectation(file);
    let mut meta = synthesise(&expectation).expect("a synthetic CDM");
    change(&mut meta);
    check_parts(&meta, &expectation, &[Part::Context])
}

fn millimetres(value: f64) -> ParameterValue {
    ParameterValue::Quantity(Quantity::from_file(value, Some("mm")))
}

#[test]
fn the_synthetic_context_of_every_expectation_conforms() {
    let mut listed = 0;
    for expectation in load_all().expect("the expectation files can be read") {
        let meta = synthesise(&expectation).expect("a synthetic CDM");
        assert_eq!(
            check_parts(&meta, &expectation, &[Part::Context]),
            Vec::<String>::new(),
            "{}",
            expectation.miniature()
        );
        listed += meta.context.values().len();
    }
    // the eight files list 11 + 11 + 7 + 7 + 8 + 8 + 4 + 0 values; the ones that are not recorded are listed apart
    assert_eq!(listed, 56);
    // V18 carries none, and says so; V16's three placeholders are not values
    let v18 = synthesise(&expectation("v18.h5")).expect("a synthetic CDM");
    assert!(v18.context.is_empty() && v18.origins.is_empty());
    let v16 = synthesise(&expectation("v16.h5")).expect("a synthetic CDM");
    assert_eq!(v16.context.values().len(), 4);
    assert_eq!(
        v16.origins.get("context.starting_height"),
        Some(&Origin::PlaceholderDropped { raw: "1.0".into() })
    );
}

#[test]
fn a_changed_missing_or_extra_value_is_reported_on_its_own_name() {
    // a text that changes
    let found = after("v17_mock.h5", |m| {
        m.context.recoat_direction = Some("X-".into());
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("recoat_direction") && found[0].contains("\"X-\""));
    // a number that changes, and a unit that changes: each is its own line
    let found = after("v17_mock.h5", |m| {
        m.context.plate.x = Some(Quantity::from_file(251.0, Some("mm")));
        m.context.plate.y = Some(Quantity::from_file(250.0, Some("cm")));
    });
    assert!(found
        .iter()
        .any(|l| l.contains("plate.x") && l.contains("251")));
    assert!(found
        .iter()
        .any(|l| l.contains("plate.y") && l.contains("unit")));
    // a value that is missing
    let found = after("v17_mock.h5", |m| m.context.scanner_id = None);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("scanner_id") && found[0].contains("not there"));
    // a value that the file does not have
    let found = after("v17_march.h5", |m| {
        m.context.gas_flow_direction = Some("Y+".into());
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("gas_flow_direction") && found[0].contains("expected nothing"));
    // a quantity that is gone and a text that the file does not have
    let found = after("v17_mock.h5", |m| {
        m.context.plate.z = None;
        m.context
            .set("plate.shape", ParameterValue::Text("Square".into()))
            .unwrap();
    });
    assert!(found.iter().any(|l| l.contains("plate.z")));
    assert!(found.iter().any(|l| l.contains("plate.shape")));
}

#[test]
fn a_placeholder_that_was_recorded_or_whose_raw_spelling_changed_is_reported() {
    // the mock has no plate thickness, because 0 is what tauri writes when it does not know: recording it is a finding on that value
    let found = after("v17_mock.h5", |m| {
        m.context.set("plate.thickness", millimetres(0.0)).unwrap();
    });
    assert!(
        found
            .iter()
            .any(|l| l.contains("plate.thickness") && l.contains("expected nothing")),
        "{found:?}"
    );
    // the raw spelling is the file's: "0" is not "0.0"
    let found = after("v17_mock.h5", |m| {
        m.origins.record(
            BuildContext::origin_name("plate.weight"),
            Origin::PlaceholderDropped { raw: "0".into() },
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("plate.weight") && found[0].contains("its origin"));
    // and the origin cannot say something else
    let found = after("v17_march.h5", |m| {
        m.origins.record(
            BuildContext::origin_name("plate.dimension"),
            Origin::Defaulted,
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("plate.dimension"));
}

#[test]
fn the_vocabulary_of_the_plate_is_held_to_the_sentence_that_the_script_wrote() {
    // the sentence of the other vocabulary, a sentence of another making, and an origin where the file has no plate are each reported
    let found = after("v17_mock.h5", |m| {
        m.origins.record(
            BuildContext::origin_name("plate"),
            Origin::Other(build_log_model::PLATE_AS_DIMENSION.into()),
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("vocabulary of the plate"));
    let found = after("v17_oct15.h5", |m| {
        m.origins.record(
            BuildContext::origin_name("plate"),
            Origin::Other("a plate".into()),
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    let found = after("v18.h5", |m| {
        m.origins.record(
            BuildContext::origin_name("plate"),
            Origin::Other(build_log_model::PLATE_AS_AXES.into()),
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    // the library's two constants are the script's own words, as the expectations carry them
    for (file, constant) in [
        ("v17_mock.h5", build_log_model::PLATE_AS_AXES),
        ("v17_oct15.h5", build_log_model::PLATE_AS_DIMENSION),
    ] {
        let meta = synthesise(&expectation(file)).expect("a synthetic CDM");
        assert_eq!(
            meta.origins.get("context.plate"),
            Some(&Origin::Other(constant.to_string())),
            "{file}"
        );
    }
}

#[test]
fn an_origin_about_the_context_that_was_not_expected_is_reported() {
    let found = after("v17_mock.h5", |m| {
        m.origins.record(
            BuildContext::origin_name("plate.x"),
            Origin::PlaceholderDropped {
                raw: "250.0".into(),
            },
        );
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("context.plate.x") && found[0].contains("expected nothing"));
    // an origin that is not about the context is not the context's concern
    let found = after("v17_mock.h5", |m| {
        m.origins.record("date", Origin::Defaulted);
    });
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_expectation_that_lists_no_context_expects_an_empty_one() {
    let bare = Expectation::parse(r#"{"expectations_version": 1, "build": {}}"#).expect("parses");
    let empty = BuildMeta::new(BuildIdentity {
        build_id: Some("a build".into()),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    });
    assert!(check_parts(&empty, &bare, &[Part::Context]).is_empty());
    let mut set = empty.clone();
    set.context.machine_serial = Some("a serial".into());
    set.origins
        .record(BuildContext::origin_name("plate"), Origin::Defaulted);
    let found = check_parts(&set, &bare, &[Part::Context]);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().any(|l| l.contains("whether it has any value")));
    assert!(found.iter().any(|l| l.contains("context.plate")));
}
