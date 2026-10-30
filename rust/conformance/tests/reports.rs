//! The findings and the availability of a source, held to the expectation of a miniature, and shown to fail when they are wrong (IP-07 phases 7C and 7D, tasks T07.3 and T07.4).
//!
//! * `the_synthetic_builds_of_the_eight_expectations_give_the_findings_and_the_availability_they_state`: a build made from each expectation, in a source that only discovers, gives the
//!   findings and the availability that the independent script wrote (the findings of eight files, and the layer, laser and channel counts of each).
//! * The rest hold the checks themselves: a finding that is missing, one that is not expected, one of another severity or in another place, a count that is wrong, an exception that is
//!   missing or invented, a state that is another, and an expectation that states neither.

use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;

use build_log_conformance::{
    check_availability, check_validation, expectations_dir, load_all, synthesise, Expectation,
};
use build_log_model::{
    availability_of, BuildMeta, BuildSource, ColumnData, ColumnRequest, ColumnSource, Error,
    ErrorKind, Finding, FindingCode, InMemorySource, LaserKey, LayerId, Location, Origin, Presence,
    ReadContext, Revision, ValidationReport,
};
use serde_json::Value;

/// A source of a build, with the reports that it was told to give (a source that gives another report than its build's is the thing the checks are for).
struct Told {
    build: Arc<BuildMeta>,
    validation: Option<Arc<ValidationReport>>,
    changed_build_for_availability: Option<Arc<BuildMeta>>,
}

impl ColumnSource for Told {
    fn read(&self, _: &ColumnRequest, _: &ReadContext) -> Result<ColumnData, Error> {
        panic!("a report must not read a column");
    }
}

impl BuildSource for Told {
    fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        Ok(Arc::clone(&self.build))
    }
    fn revision(&self) -> Result<Revision, Error> {
        Ok(Revision::default())
    }
    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        match &self.validation {
            Some(report) => Ok(Arc::clone(report)),
            None => Ok(Arc::new(build_log_model::validate_build(&self.build))),
        }
    }
    fn availability(&self) -> Result<Arc<build_log_model::AvailabilityReport>, Error> {
        let build = self
            .changed_build_for_availability
            .as_ref()
            .unwrap_or(&self.build);
        let validation = build_log_model::validate_build(build);
        Ok(Arc::new(availability_of(build, &validation, &|_| None)))
    }
}

fn told(build: BuildMeta) -> Told {
    Told {
        build: Arc::new(build),
        validation: None,
        changed_build_for_availability: None,
    }
}

/// The expectation of a miniature, changed by `change`, parsed.
fn expectation(name: &str, change: impl FnOnce(&mut Value)) -> Expectation {
    let text = fs::read_to_string(expectations_dir().join(name)).expect("the file");
    let mut root: Value = serde_json::from_str(&text).expect("JSON");
    change(&mut root);
    Expectation::parse(&root.to_string()).expect("an expectation")
}

fn lines(found: &[String]) -> String {
    found.join("\n  ")
}

#[test]
fn the_synthetic_builds_of_the_eight_expectations_give_the_findings_and_the_availability_they_state(
) {
    let all = load_all().expect("the expectation files");
    assert_eq!(all.len(), 8);
    let mut with_findings = BTreeSet::new();
    for expectation in &all {
        let build = synthesise(expectation).expect("a build");
        let mut source = InMemorySource::new();
        source.set_build(build);
        let found = check_validation(&source, expectation);
        assert!(
            found.is_empty(),
            "{}:\n  {}",
            expectation.miniature(),
            lines(&found)
        );
        let found = check_availability(&source, expectation);
        assert!(
            found.is_empty(),
            "{}:\n  {}",
            expectation.miniature(),
            lines(&found)
        );
        let report = source.validate().expect("a report");
        if !report.is_clean() {
            with_findings.insert(expectation.miniature().to_string());
        }
    }
    // the one file with a finding is the one that declares more layers than it holds
    assert_eq!(
        with_findings.into_iter().collect::<Vec<_>>(),
        ["special_layers_v17.h5"]
    );
}

#[test]
fn a_finding_that_is_missing_extra_of_another_severity_or_in_another_place_is_reported() {
    let special = expectation("special_layers_v17.json", |_| {});
    let build = synthesise(&special).expect("a build");
    let good = build_log_model::validate_build(&build);
    assert_eq!(good.findings.len(), 1);
    let finding = good.findings[0].clone();

    let cases: Vec<(&str, Vec<Finding>, &str)> = vec![
        ("a finding missing", vec![], "missing: LayerCountDiffers"),
        (
            "a finding that is not expected",
            vec![
                finding.clone(),
                Finding::new(
                    FindingCode::LayersOverlap,
                    Location::new().with_layer(LayerId(3)),
                    "an overlap",
                ),
            ],
            "not expected: LayersOverlap",
        ),
        (
            "a finding twice",
            vec![finding.clone(), finding.clone()],
            "not expected: LayerCountDiffers",
        ),
        (
            "a finding in another place",
            vec![Finding::new(
                FindingCode::LayerCountDiffers,
                Location::new().with_layer(LayerId(1)),
                "x",
            )],
            "missing: LayerCountDiffers (Note) at the build",
        ),
        (
            "a finding of another code",
            vec![Finding::new(
                FindingCode::LayerWithoutLasers,
                Location::new(),
                "x",
            )],
            "not expected: LayerWithoutLasers",
        ),
    ];
    for (what, findings, said) in cases {
        let mut source = told(build.clone());
        source.validation = Some(Arc::new(ValidationReport::new(findings)));
        let found = check_validation(&source, &special);
        assert!(!found.is_empty(), "{what}");
        assert!(found.iter().any(|l| l.contains(said)), "{what}: {found:?}");
    }
    // the severity is held too: a report whose finding has another severity says another thing, and its counts are wrong as well
    let mut wrong = finding.clone();
    wrong.severity = build_log_model::Severity::Warning;
    let mut report = ValidationReport::new(vec![finding.clone()]);
    report.findings = vec![wrong];
    let mut source = told(build.clone());
    source.validation = Some(Arc::new(report));
    let found = check_validation(&source, &special);
    assert!(found.iter().any(|l| l.contains("(Warning)")), "{found:?}");
    // the counts of a report that does not match its findings are reported on their own
    let mut report = ValidationReport::new(vec![finding]);
    report.counts.faults = 4;
    let mut source = told(build);
    source.validation = Some(Arc::new(report));
    let found = check_validation(&source, &special);
    assert_eq!(found, ["the faults counted: 4, expected 0"]);
}

#[test]
fn an_availability_that_is_changed_is_reported_by_its_count_and_its_item() {
    let mock = expectation("v17_mock.json", |_| {});
    let build = synthesise(&mock).expect("a build");
    let source = told(build.clone());
    assert!(check_availability(&source, &mock).is_empty());

    // a laser that is not recorded in a layer where it was: the count of the lasers and of the channels changes
    let mut changed = build.clone();
    let laser = changed
        .laser_mut(&LaserKey::new("Laser_1", 1))
        .expect("the laser");
    laser.layers.insert(
        LayerId(1),
        build_log_model::LaserLayerMeta::new(Presence::NotRecorded),
    );
    let mut source = told(build.clone());
    source.changed_build_for_availability = Some(Arc::new(changed));
    let found = check_availability(&source, &mock);
    assert!(
        found.iter().any(|l| l.starts_with("the present lasers:")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|l| l.starts_with("the not_recorded lasers:")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|l| l.starts_with("the present channels:")),
        "{found:?}"
    );

    // a channel that is not read: unreadable, which the expectation does not state: its count and its item are reported
    let mut changed = build.clone();
    let error = Error::new(ErrorKind::Corrupt {
        detail: "damage".into(),
    });
    changed
        .laser_mut(&LaserKey::new("Laser_1", 1))
        .expect("the laser")
        .layers
        .get_mut(LayerId(1))
        .expect("the entry")
        .origins
        .record("channel Laser_Power", Origin::not_read(&error));
    let mut source = told(build);
    source.changed_build_for_availability = Some(Arc::new(changed));
    let found = check_availability(&source, &mock);
    assert!(
        found
            .iter()
            .any(|l| l.starts_with("the unreadable channels: 1, expected 0")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|l| l.contains(
            "not expected: channel unreadable at layer 1, laser Laser_1, channel Laser_Power"
        )),
        "{found:?}"
    );
}

#[test]
fn an_exception_that_is_missing_or_of_another_state_is_reported() {
    // an expectation that states a degraded channel, which a build that is clean does not give
    let clean = expectation("v17_mock.json", |_| {});
    let build = synthesise(&clean).expect("a build");
    let stated = expectation("v17_mock.json", |root| {
        let a = &mut root["availability"];
        a["exceptions"] = serde_json::json!([
            {"item": "channel", "layer": 1, "laser": "Laser_1", "channel": "Laser_Power", "state": "degraded"}
        ]);
        a["channels"]["degraded"] = Value::from(1);
        a["channels"]["present"] = Value::from(a["channels"]["present"].as_i64().unwrap() - 1);
    });
    let found = check_availability(&told(build.clone()), &stated);
    assert!(
        found.iter().any(|l| l
            .contains("missing: channel degraded at layer 1, laser Laser_1, channel Laser_Power")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|l| l.starts_with("the degraded channels: 0, expected 1")),
        "{found:?}"
    );
    // the same exception in another state is not the same exception
    let other = expectation("v17_mock.json", |root| {
        root["availability"]["exceptions"] = serde_json::json!([
            {"item": "channel", "layer": 1, "laser": "Laser_1", "channel": "Laser_Power", "state": "unsupported"}
        ]);
    });
    let found = check_availability(&told(build), &other);
    assert!(
        found
            .iter()
            .any(|l| l.contains("missing: channel unsupported")),
        "{found:?}"
    );
}

#[test]
fn a_degraded_channel_that_the_expectation_states_is_held_to_it() {
    // the mock build with the power of its first laser holding 40 samples in the first layer: a channel shorter than the stream, which is a warning and makes the channel degraded
    let plain = expectation("v17_mock.json", |_| {});
    let mut changed = synthesise(&plain).expect("a build");
    changed
        .laser_mut(&LaserKey::new("Laser_1", 1))
        .expect("the laser")
        .channels
        .iter_mut()
        .find(|c| c.key == build_log_model::ChannelKey::new(["Laser_Power"]))
        .expect("the power")
        .presence
        .insert(LayerId(1), Presence::from_count(40));
    let stated = expectation("v17_mock.json", |root| {
        root["validation"]["findings"] = serde_json::json!([
            {"code": "ChannelShorterThanStream", "severity": "Warning", "layer": 1, "laser": "Laser_1", "channel": "Laser_Power"}
        ]);
        root["validation"]["counts"]["warnings"] = Value::from(1);
        let a = &mut root["availability"];
        a["exceptions"] = serde_json::json!([
            {"item": "channel", "layer": 1, "laser": "Laser_1", "channel": "Laser_Power", "state": "degraded"}
        ]);
        a["channels"]["degraded"] = Value::from(1);
        a["channels"]["present"] = Value::from(a["channels"]["present"].as_i64().unwrap() - 1);
    });
    let source = told(changed.clone());
    // the build gives exactly what the expectation states: a finding, a degraded channel, one fewer present
    assert_eq!(check_validation(&source, &stated), Vec::<String>::new());
    assert_eq!(check_availability(&source, &stated), Vec::<String>::new());
    // and the expectation that says nothing is not met by it, in both
    assert!(!check_validation(&source, &plain).is_empty());
    assert!(!check_availability(&source, &plain).is_empty());
}

#[test]
fn an_expectation_that_states_neither_is_an_error_and_not_a_pass() {
    let bare = expectation("v17_mock.json", |root| {
        let map = root.as_object_mut().expect("an object");
        map.remove("validation");
        map.remove("availability");
    });
    let source = told(synthesise(&bare).expect("a build"));
    let found = check_validation(&source, &bare);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].contains("the expectation of the findings") && found[0].contains("validation"),
        "{found:?}"
    );
    let found = check_availability(&source, &bare);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].contains("the expectation of the availability")
            && found[0].contains("availability"),
        "{found:?}"
    );
}
