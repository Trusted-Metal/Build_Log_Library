//! Opening a file through the registry (IP-04 phase 4D, tasks T04.6 to T04.8; requirements BL-VA-01, BL-VA-08, BL-VA-10, BL-AR-09).
//!
//! The mock version (key `9001`) is the only adapter, and it is registered by injection; its files are made at test time. A second fake version (`9002`) and
//! two more for the probe tests are defined in this file, to show that the registry, and not the dispatcher, decides what exists.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use build_log_io::adapters::{mock, AdapterSet, MarkerPlace};
use build_log_io::dispatch::{open, open_at, open_probing, Registry};
use build_log_model::{
    BuildIdentity, BuildMeta, BuildSource, Capabilities, Direction, Error, ErrorClass, ErrorKind,
    Feature, InMemorySource, LayerId,
};
use common::{put_scalar, put_text, temp_file, TempH5};
use hdf5::Group;

// ---------------------------------------------------------------- the fakes

fn identity(id: &str) -> BuildIdentity {
    BuildIdentity {
        build_id: id.to_string(),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    }
}

/// A reader that ignores the file and says where it came from, so that a test can tell which adapter was chosen.
fn fake_reader(_mount: &Group) -> Result<Box<dyn BuildSource>, Error> {
    let mut source = InMemorySource::new();
    source.set_build(BuildMeta::new(identity("from-the-fake")));
    Ok(Box::new(source))
}

fn fake(key: &str, mount: &str, attribute: &str) -> AdapterSet {
    AdapterSet::builder(
        key,
        format!("fake-{key}"),
        MarkerPlace::new(mount, attribute),
    )
    .capabilities(Capabilities::new().with(Direction::Read, Feature::Events))
    .reader(fake_reader)
    .without_writer("the fake has no writer")
    .without_patcher("the fake has no patcher")
    .build()
    .expect("a complete set")
}

fn mock_registry() -> Registry {
    Registry::new()
        .with(mock::adapter_set())
        .expect("the mock registers")
}

fn mock_file(label: &str, build_id: &str, layers: i32) -> TempH5 {
    temp_file(label, |file| {
        let mount = file.create_group(mock::MOUNT).expect("the mount");
        mock::write_mock_file(&mount, build_id, layers).expect("a mock file");
    })
}

fn kind_name(error: &Error) -> &'static str {
    match error.kind() {
        ErrorKind::Unversioned { .. } => "Unversioned",
        ErrorKind::UnsupportedVersion { .. } => "UnsupportedVersion",
        ErrorKind::UnsupportedInVersion { .. } => "UnsupportedInVersion",
        ErrorKind::Corrupt { .. } => "Corrupt",
        _ => "another kind",
    }
}

// ---------------------------------------------------------------- the mock and the dispatch

#[test]
fn the_mock_version_reads_into_the_cdm() {
    let file = mock_file("mock-reads", "mock-build-1", 3);
    let opened = open(&mock_registry(), &file.0).expect("the mock file opens");
    let build = opened.source().build().expect("the metadata");
    assert_eq!(build.identity.build_id, "mock-build-1");
    assert_eq!(build.layers_declared, Some(3));
    assert_eq!(
        opened.source().layers().expect("the layers"),
        vec![LayerId(1), LayerId(2), LayerId(3)]
    );
    // which version it is, which adapter read it, and what that version can do
    assert_eq!(opened.key().as_str(), "9001");
    assert_eq!(opened.adapter(), "mock");
    assert_eq!(opened.capabilities(), &Capabilities::new());
    let report = opened.report("1.2.3");
    assert_eq!(
        (
            report.schema_version.as_str(),
            report.adapter.as_str(),
            report.package_version.as_str()
        ),
        ("9001", "mock", "1.2.3")
    );
}

#[test]
fn open_registry_dispatches_via_injected_test_file() {
    let file = mock_file("dispatch-injected", "mock-build-2", 1);
    // with the mock in the registry the mock adapter reads the file
    let from_mock = open(&mock_registry(), &file.0).expect("opens");
    assert_eq!(
        from_mock.source().build().unwrap().identity.build_id,
        "mock-build-2"
    );
    // with another adapter registered for the same version and place, that adapter reads it: the registry is consulted, nothing is hard-coded
    let injected = Registry::new()
        .with(fake("9001", mock::MOUNT, "File_Version"))
        .unwrap();
    let from_fake = open(&injected, &file.0).expect("opens");
    assert_eq!(
        from_fake.source().build().unwrap().identity.build_id,
        "from-the-fake"
    );
    assert_eq!(from_fake.adapter(), "fake-9001");
    assert_eq!(
        from_fake.capabilities(),
        &Capabilities::new().with(Direction::Read, Feature::Events)
    );
    // with nothing registered, nothing can be opened, and the answer says so
    let err = open(&Registry::new(), &file.0).expect_err("nothing is registered");
    assert!(
        matches!(err.kind(), ErrorKind::UnsupportedVersion { supported, .. } if supported.is_empty()),
        "{err}"
    );
    // a version that is not registered is `UnsupportedVersion`, with what was found and what is supported
    let other = temp_file("dispatch-9002", |f| {
        let g = f.create_group(mock::MOUNT).expect("the mount");
        put_scalar(&g, "File_Version", 9002_i32);
    });
    let err = open(&mock_registry(), &other.0).expect_err("9002 is not registered");
    match err.kind() {
        ErrorKind::UnsupportedVersion { found, supported } => {
            assert_eq!(found, "9002");
            assert_eq!(supported, &["9001".to_string()]);
        }
        other => panic!("expected UnsupportedVersion, found {other:?}"),
    }
}

#[test]
fn unversioned_file_returns_typed_outcome() {
    let registry = mock_registry();
    // a mount without the marker, and a file without the mount: both are `Unversioned`, a kind of its own
    let no_marker = temp_file("unversioned-mount", |f| {
        let g = f.create_group(mock::MOUNT).expect("the mount");
        put_text(&g, "Build_ID", "no marker here");
    });
    let no_mount = temp_file("unversioned-empty", |f| {
        f.create_group("Something_Else").expect("a group");
    });
    for (label, path) in [
        ("a mount without the marker", &no_marker.0),
        ("a file without the mount", &no_mount.0),
    ] {
        let err = open(&registry, path).expect_err(label);
        assert!(
            matches!(err.kind(), ErrorKind::Unversioned { .. }),
            "{label}: {err}"
        );
        assert_eq!(err.class(), ErrorClass::Validation, "{label}");
        assert!(!err.is_retryable(), "{label}");
    }
    // the same at a mount that is handed in
    let file = hdf5::File::open(&no_marker.0).expect("opens");
    let group = file.group(mock::MOUNT).expect("the mount");
    let err = open_at(&registry, &group).expect_err("no marker");
    assert!(matches!(err.kind(), ErrorKind::Unversioned { .. }), "{err}");
    // and it is not the outcome of a version nobody registered
    assert_ne!(kind_name(&err), "UnsupportedVersion");
}

#[test]
fn group_attached_mode_matches_standalone_file_mode() {
    let registry = mock_registry();
    let standalone = mock_file("mount-standalone", "same-data", 4);
    // the same data, nested three groups deep and under another name
    let nested = temp_file("mount-nested", |f| {
        let deepest = f
            .create_group("outer")
            .and_then(|g| g.create_group("inner"))
            .and_then(|g| g.create_group("any_name_at_all"))
            .expect("nested groups");
        mock::write_mock_file(&deepest, "same-data", 4).expect("a mock file");
    });
    let by_path = open(&registry, &standalone.0).expect("standalone opens");
    let file = hdf5::File::open(&nested.0).expect("opens");
    let deepest = file
        .group("outer/inner/any_name_at_all")
        .expect("the nested mount");
    let attached = open_at(&registry, &deepest).expect("a nested mount opens");
    assert_eq!(
        *attached.source().build().unwrap(),
        *by_path.source().build().unwrap()
    );
    assert_eq!(attached.key(), by_path.key());
    // opening the standalone mount by hand is the same as opening it by path
    let standalone_file = hdf5::File::open(&standalone.0).expect("opens");
    let mount = standalone_file.group(mock::MOUNT).expect("the mount");
    let by_hand = open_at(&registry, &mount).expect("opens at the mount");
    assert_eq!(
        *by_hand.source().build().unwrap(),
        *by_path.source().build().unwrap()
    );
    // the path wrapper does not find a nested mount: it looks for the registered places among the root's children, and the mount at a mount is the way in
    let err = open(&registry, &nested.0).expect_err("nothing is at the root");
    assert!(matches!(err.kind(), ErrorKind::Unversioned { .. }), "{err}");
}

#[test]
fn open_preserves_the_on_disk_version() {
    let registry = Registry::new()
        .with(mock::adapter_set())
        .unwrap()
        .with(fake("9002", "Fake_Mount", "File_Version"))
        .unwrap();
    let mock_one = mock_file("preserve-9001", "preserved", 2);
    let fake_one = temp_file("preserve-9002", |f| {
        let g = f.create_group("Fake_Mount").expect("the mount");
        put_scalar(&g, "File_Version", 9002_i32);
    });
    // each file is read by the adapter of its own version, whatever else is registered, and nothing is upgraded
    assert_eq!(open(&registry, &mock_one.0).unwrap().key().as_str(), "9001");
    assert_eq!(open(&registry, &fake_one.0).unwrap().key().as_str(), "9002");
    // and the file is opened for reading only: its bytes are the same after
    let before = std::fs::read(&mock_one.0).expect("reads");
    drop(open(&registry, &mock_one.0).expect("opens"));
    assert_eq!(std::fs::read(&mock_one.0).expect("reads"), before);
}

#[test]
fn a_marker_that_is_not_where_its_version_keeps_it_is_corrupt() {
    // two versions share a mount and keep their marker in different attributes; a file whose attribute of one version names the other is not either
    let registry = Registry::new()
        .with(fake("9101", "Shared_Mount", "Marker_P"))
        .unwrap()
        .with(fake("9102", "Shared_Mount", "Marker_Q"))
        .unwrap();
    let crossed = temp_file("crossed-marker", |f| {
        let g = f.create_group("Shared_Mount").expect("the mount");
        put_scalar(&g, "Marker_P", 9102_i32);
    });
    let err = open(&registry, &crossed.0).expect_err("the marker names the other version");
    assert!(matches!(err.kind(), ErrorKind::Corrupt { .. }), "{err}");
    // each version in its own attribute opens as itself
    let own = temp_file("own-marker", |f| {
        let g = f.create_group("Shared_Mount").expect("the mount");
        put_scalar(&g, "Marker_Q", 9102_i32);
    });
    assert_eq!(open(&registry, &own.0).unwrap().key().as_str(), "9102");
}

#[test]
fn two_markers_on_two_mounts_are_refused() {
    let registry = Registry::new()
        .with(mock::adapter_set())
        .unwrap()
        .with(fake("9002", "Fake_Mount", "File_Version"))
        .unwrap();
    let both = temp_file("two-mounts", |f| {
        let a = f.create_group(mock::MOUNT).expect("the mock mount");
        mock::write_mock_file(&a, "first", 1).expect("a mock file");
        let b = f.create_group("Fake_Mount").expect("the fake mount");
        put_scalar(&b, "File_Version", 9002_i32);
    });
    // the dispatcher does not choose between them
    let err = open(&registry, &both.0).expect_err("two builds in one file");
    match err.kind() {
        ErrorKind::Corrupt { detail } => assert!(
            detail.contains(mock::MOUNT) && detail.contains("Fake_Mount"),
            "{detail}"
        ),
        other => panic!("expected Corrupt, found {other:?}"),
    }
    // the caller who knows which one it wants opens that mount
    let file = hdf5::File::open(&both.0).expect("opens");
    let first =
        open_at(&registry, &file.group(mock::MOUNT).unwrap()).expect("the mock mount opens");
    assert_eq!(first.key().as_str(), "9001");
    let second =
        open_at(&registry, &file.group("Fake_Mount").unwrap()).expect("the fake mount opens");
    assert_eq!(second.key().as_str(), "9002");
}

// ---------------------------------------------------------------- the explicit probe

static ASKED_ONCE: AtomicUsize = AtomicUsize::new(0);
static ASKED_NEVER: AtomicUsize = AtomicUsize::new(0);

fn looks_like_it(group: &Group) -> bool {
    group
        .attr_names()
        .map(|names| names.iter().any(|n| n == "Shape_Hint"))
        .unwrap_or(false)
}

fn probe_counted_once(group: &Group) -> bool {
    ASKED_ONCE.fetch_add(1, Ordering::SeqCst);
    looks_like_it(group)
}

fn probe_never_asked(group: &Group) -> bool {
    ASKED_NEVER.fetch_add(1, Ordering::SeqCst);
    looks_like_it(group)
}

fn with_probe(key: &str, mount: &str, probe: fn(&Group) -> bool) -> AdapterSet {
    AdapterSet::builder(
        key,
        format!("probing-{key}"),
        MarkerPlace::new(mount, "File_Version"),
    )
    .reader(fake_reader)
    .without_writer("the fake has no writer")
    .without_patcher("the fake has no patcher")
    .probe(probe)
    .build()
    .expect("a complete set")
}

#[test]
fn a_probe_runs_only_when_the_caller_asks() {
    let registry = Registry::new()
        .with(with_probe("9201", "Probe_Mount", probe_counted_once))
        .unwrap();
    // a file with no marker whose structure the adapter would recognise
    let markerless = temp_file("probe-markerless", |f| {
        let g = f.create_group("Probe_Mount").expect("the mount");
        put_text(&g, "Shape_Hint", "it looks like mine");
    });
    // `open` never probes: the file is unversioned, and the probe was not called
    let err = open(&registry, &markerless.0).expect_err("no marker");
    assert!(matches!(err.kind(), ErrorKind::Unversioned { .. }), "{err}");
    assert_eq!(ASKED_ONCE.load(Ordering::SeqCst), 0);
    // when the caller asks, the probe is called once and the version is the probe's
    let opened = open_probing(&registry, &markerless.0).expect("the probe recognises it");
    assert_eq!(opened.key().as_str(), "9201");
    assert_eq!(ASKED_ONCE.load(Ordering::SeqCst), 1);
}

#[test]
fn a_probe_that_matches_selects_its_version() {
    // a probe never overrules a marker: a file that has one is not probed, even when the caller asks
    let registry = Registry::new()
        .with(with_probe("9301", "Probe_Mount", probe_never_asked))
        .unwrap()
        .with(mock::adapter_set())
        .unwrap();
    let marked = mock_file("probe-marked", "has a marker", 1);
    assert_eq!(
        open_probing(&registry, &marked.0).unwrap().key().as_str(),
        "9001"
    );
    assert_eq!(
        ASKED_NEVER.load(Ordering::SeqCst),
        0,
        "a file with a marker is not probed"
    );
    // a probe that does not match leaves the file unversioned
    let other = temp_file("probe-nomatch", |f| {
        let g = f.create_group("Probe_Mount").expect("the mount");
        put_text(&g, "Something", "unrelated");
    });
    let err = open_probing(&registry, &other.0).expect_err("nothing matches");
    assert!(matches!(err.kind(), ErrorKind::Unversioned { .. }), "{err}");
    // two probes that both match are refused, not chosen between
    let two = Registry::new()
        .with(with_probe("9401", "Probe_A", looks_like_it))
        .unwrap()
        .with(with_probe("9402", "Probe_B", looks_like_it))
        .unwrap();
    let both = temp_file("probe-both", |f| {
        for name in ["Probe_A", "Probe_B"] {
            let g = f.create_group(name).expect("a mount");
            put_text(&g, "Shape_Hint", "me too");
        }
    });
    let err = open_probing(&two, &both.0).expect_err("two probes match");
    assert!(matches!(err.kind(), ErrorKind::Corrupt { .. }), "{err}");
    // with one mount present, the probe of that mount's version selects it
    let one = temp_file("probe-one", |f| {
        let g = f.create_group("Probe_B").expect("a mount");
        put_text(&g, "Shape_Hint", "only me");
    });
    assert_eq!(open_probing(&two, &one.0).unwrap().key().as_str(), "9402");
}
