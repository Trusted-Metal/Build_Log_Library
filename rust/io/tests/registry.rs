//! The adapter set, the registry and resolution (IP-04 phase 4C, tasks T04.3 to T04.5; requirements BL-VA-03, BL-VA-04, BL-VA-10).
//!
//! The registry is data, so every test builds its own with fake versions and touches no global state. (The production registry is read only, to say what
//! it holds.)

use build_log_io::adapters::{AdapterSet, MarkerPlace, Surface};
use build_log_io::dispatch::{production, resolve, resolve_for_create, Registry, VersionKey};
use build_log_model::{
    BuildSource, Capabilities, Direction, Error, ErrorClass, ErrorKind, Feature, InMemorySource,
};

fn fake_reader(_mount: &hdf5::Group) -> Result<Box<dyn BuildSource>, Error> {
    Ok(Box::new(InMemorySource::new()))
}

/// A complete fake version: a reader, and no writer and no patcher, each with its reason.
fn fake(key: &str) -> AdapterSet {
    AdapterSet::builder(
        key,
        format!("fake-{key}"),
        MarkerPlace::new("Fake_Mount", "Fake_Marker"),
    )
    .capabilities(Capabilities::new().with(Direction::Read, Feature::Events))
    .reader(fake_reader)
    .without_writer("the fake version has no writer")
    .without_patcher("the fake version has no patcher")
    .build()
    .expect("a complete set builds")
}

fn key(text: &str) -> VersionKey {
    VersionKey::new(text).expect("a key")
}

fn registry_of(keys: &[&str]) -> Registry {
    let mut registry = Registry::new();
    for k in keys {
        registry.register(fake(k)).expect("a new key");
    }
    registry
}

#[test]
fn open_registry_rejects_unregistered_version() {
    let registry = registry_of(&["17", "18"]);
    for unknown in ["19", "16", "1.0", "9001"] {
        let err = resolve(&registry, &key(unknown)).expect_err(unknown);
        match err.kind() {
            ErrorKind::UnsupportedVersion { found, supported } => {
                assert_eq!(found, unknown);
                assert_eq!(
                    supported,
                    &["17".to_string(), "18".to_string()],
                    "the supported list is the registry's keys"
                );
            }
            other => panic!("expected UnsupportedVersion for {unknown}, found {other:?}"),
        }
        assert_eq!(err.class(), ErrorClass::Caller);
        assert!(!err.is_retryable());
    }
    // a registered version resolves to its own set, and to no other
    assert_eq!(
        resolve(&registry, &key("18")).expect("registered").name(),
        "fake-18"
    );
    assert_eq!(
        resolve(&registry, &key("17")).expect("registered").name(),
        "fake-17"
    );
}

#[test]
fn create_registry_rejects_unregistered_version() {
    let registry = registry_of(&["17"]);
    let err = resolve_for_create(&registry, &key("19")).expect_err("not registered");
    assert!(
        matches!(err.kind(), ErrorKind::UnsupportedVersion { found, .. } if found == "19"),
        "{err}"
    );
}

#[test]
fn a_version_without_a_writer_answers_create_with_unsupported_in_version() {
    // a reader-only version: it opens, and it cannot create (the contract of the registry, whatever the adapter does)
    let registry = registry_of(&["9001"]);
    assert!(resolve(&registry, &key("9001")).is_ok());
    let err = resolve_for_create(&registry, &key("9001")).expect_err("no writer");
    match err.kind() {
        ErrorKind::UnsupportedInVersion { version, feature } => {
            assert_eq!(version, "9001");
            assert!(
                feature.contains("creating") && feature.contains("the fake version has no writer"),
                "{feature}"
            );
        }
        other => panic!("expected UnsupportedInVersion, found {other:?}"),
    }
    assert_eq!(err.class(), ErrorClass::Caller);
}

#[test]
fn supported_versions_reflects_production_registry() {
    // derived from the keys, never from a list of its own: the same keys, in the order of the versions
    let production = production();
    let from_sets: Vec<String> = production.sets().map(|s| s.key().to_string()).collect();
    assert_eq!(production.supported_versions(), from_sets);
    assert_eq!(production.len(), from_sets.len());
    // and in an injected registry the order is the order of the numbers, whatever order they were registered in
    let registry = registry_of(&["9001", "18", "100", "16", "17"]);
    assert_eq!(
        registry.supported_versions(),
        ["16", "17", "18", "100", "9001"]
    );
}

#[test]
fn the_production_registry_holds_no_mock() {
    // 9000 to 9999 is reserved for test-only versions (D00-4), registered by injection and never in production
    for version in production().supported_versions() {
        let in_mock_range = version
            .parse::<u32>()
            .map(|n| (9000..=9999).contains(&n))
            .unwrap_or(false);
        assert!(
            !in_mock_range,
            "{version} is a mock version and is registered in production"
        );
    }
}

#[test]
fn a_duplicate_key_is_refused() {
    let mut registry = registry_of(&["17"]);
    let err = registry
        .register(fake("17"))
        .expect_err("17 is already there");
    assert!(
        matches!(err.kind(), ErrorKind::ValidationError { violations } if violations.len() == 1),
        "{err}"
    );
    assert_eq!(registry.len(), 1, "the first registration is untouched");
    assert!(Registry::new()
        .with(fake("17"))
        .and_then(|r| r.with(fake("17")))
        .is_err());
}

#[test]
fn every_registered_version_supplies_every_surface_or_declares_it_unsupported() {
    // for the production registry (empty today, so each real adapter is held to this as it arrives) and for an injected one
    let mut all: Vec<AdapterSet> = production().sets().cloned().collect();
    all.extend(registry_of(&["17", "9001"]).sets().cloned());
    for set in &all {
        let reader_ok = set.reader().is_provided()
            || set
                .reader()
                .unsupported_reason()
                .is_some_and(|r| !r.trim().is_empty());
        assert!(
            reader_ok,
            "{}: the reader is neither provided nor declared unsupported",
            set.key()
        );
        for (name, reason) in [
            ("writer", set.writer().unsupported_reason()),
            ("patcher", set.patcher().unsupported_reason()),
        ] {
            assert!(
                reason.is_some_and(|r| !r.trim().is_empty()),
                "{}: the {name} is neither provided nor declared unsupported with a reason",
                set.key()
            );
        }
    }
}

#[test]
fn a_surface_cannot_be_left_undeclared() {
    let place = || MarkerPlace::new("Mount", "Marker");
    // nothing declared: the reader, the writer and the patcher are each reported
    let err = AdapterSet::builder("17", "v17", place())
        .build()
        .expect_err("nothing declared");
    match err.kind() {
        ErrorKind::ValidationError { violations } => {
            assert_eq!(violations.len(), 3, "{violations:?}");
            let text = format!("{violations:?}");
            for what in ["reader", "writer", "patcher"] {
                assert!(text.contains(what), "{what} is named: {text}");
            }
        }
        other => panic!("expected ValidationError, found {other:?}"),
    }
    // an unsupported surface needs a reason that says something
    let err = AdapterSet::builder("17", "v17", place())
        .reader(fake_reader)
        .without_writer("  ")
        .without_patcher("")
        .build()
        .expect_err("empty reasons");
    assert!(
        matches!(err.kind(), ErrorKind::ValidationError { violations } if violations.len() == 2),
        "{err}"
    );
    // a key that is not a version, a nameless adapter and an incomplete place are reported with the rest, all at once
    let err = AdapterSet::builder("", " ", MarkerPlace::new("", ""))
        .reader(fake_reader)
        .without_writer("none")
        .without_patcher("none")
        .build()
        .expect_err("invalid pieces");
    assert!(
        matches!(err.kind(), ErrorKind::ValidationError { violations } if violations.len() == 3),
        "{err}"
    );
    // and a complete set builds, with what it was given
    let set = fake("17");
    assert_eq!(set.key().as_str(), "17");
    assert!(
        set.reader().is_provided() && !set.writer().is_provided() && !set.patcher().is_provided()
    );
    assert!(matches!(set.writer(), Surface::Unsupported { .. }));
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Events));
    assert!(!set
        .capabilities()
        .supports(Direction::Write, Feature::Events));
}

#[test]
fn the_marker_places_are_the_distinct_places_of_the_registered_adapters() {
    let mut registry = Registry::new();
    for (key, mount) in [("17", "Built_Part"), ("16", "Built_Part"), ("18", "Build")] {
        let set = AdapterSet::builder(
            key,
            format!("v{key}"),
            MarkerPlace::new(mount, "File_Version"),
        )
        .reader(fake_reader)
        .without_writer("test")
        .without_patcher("test")
        .build()
        .expect("a complete set");
        registry.register(set).expect("a new key");
    }
    // the dispatcher knows these places and no others, each once
    assert_eq!(
        registry.marker_places(),
        [
            MarkerPlace::new("Build", "File_Version"),
            MarkerPlace::new("Built_Part", "File_Version")
        ]
    );
}
