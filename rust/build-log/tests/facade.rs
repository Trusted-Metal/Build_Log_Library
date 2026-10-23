//! The facade is the surface of a consumer, and nothing else is needed (IP-04 phase 4F, tasks T04.11 to T04.13; requirements BL-AR-03, BL-AR-08, BL-VA-06,
//! BL-VA-12 and BL-TV-02).
//!
//! These tests use the facade and the Model, as a consumer does. The mock version stands in for a real file until the first adapter lands.

#![cfg(feature = "hdf5")]

use std::fs;
use std::path::{Path, PathBuf};

use build_log::testing::{mock_registry, mount_for, write_mock_file, write_mock_group};
use build_log::{
    hdf5, library_versions, open, open_at, open_at_with, open_with, supported_versions,
    Capabilities, ErrorKind, CDM_VERSION, PACKAGE_VERSION,
};
use build_log_testkit::scan::code_of;

// The consumer example is a module of this test, so that it runs here for the mock version and for a second one: it is written against the facade only.
#[allow(dead_code)]
#[path = "../examples/mock_consumer.rs"]
mod mock_consumer;

/// A temporary file, removed when the value is dropped. The label must be different in each test, because tests run side by side.
struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn temp(label: &str) -> Temp {
    Temp(std::env::temp_dir().join(format!(
        "build-log-facade-{}-{label}.h5",
        std::process::id()
    )))
}

#[test]
fn the_facade_opens_the_mock_version_through_an_injected_registry() {
    let file = temp("opens");
    write_mock_file(&file.0, "9001", "a build", 4).expect("a mock file");
    let registry = mock_registry(&["9001"]).expect("the mock registers");

    let opened = open_with(&registry, &file.0).expect("the mock version opens");
    assert_eq!(opened.key().as_str(), "9001");
    assert_eq!(opened.adapter(), "mock");
    assert_eq!(*opened.capabilities(), Capabilities::new());
    let build = opened.source().build().expect("the metadata");
    assert_eq!(build.identity.build_id, "a build");
    assert_eq!(build.layers_declared, Some(4));
    assert_eq!(opened.source().layers().expect("the layers").len(), 4);
    assert_eq!(opened.into_source().build().expect("the metadata"), build);

    // the same build from a group of a file the caller holds, at the root of it or nested at any depth
    let held = hdf5::File::open(&file.0).expect("the file opens");
    let mount = held.group(&mount_for("9001")).expect("the mount");
    let by_group = open_at_with(&registry, &mount).expect("a mount opens");
    assert_eq!(by_group.source().build().expect("the metadata"), build);
    let nested = temp("opens-nested");
    {
        let file = hdf5::File::create(&nested.0).expect("a new file");
        let deep = file
            .create_group("one")
            .and_then(|g| g.create_group("two"))
            .expect("nested groups");
        write_mock_group(&deep, "9001", "a build", 4).expect("a mock build");
    }
    let held = hdf5::File::open(&nested.0).expect("the file opens");
    let deep = held.group("one/two").expect("the nested mount");
    let by_depth = open_at_with(&registry, &deep).expect("a nested mount opens");
    assert_eq!(by_depth.source().build().expect("the metadata"), build);

    // the production registry holds no real version yet, and the mock is never in it: opening says what it can do instead
    assert!(supported_versions().is_empty());
    for result in [open(&file.0), open_at(&mount)] {
        match result.expect_err("nothing is registered").kind() {
            ErrorKind::UnsupportedVersion { supported, .. } => assert!(supported.is_empty()),
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }
}

#[test]
fn one_call_reports_the_three_versions_and_the_adapter() {
    let file = temp("reports");
    write_mock_file(&file.0, "9001", "a build", 1).expect("a mock file");
    let registry = mock_registry(&["9001"]).expect("the mock registers");
    let report = open_with(&registry, &file.0).expect("opens").report();
    assert_eq!(report.schema_version, "9001");
    assert_eq!(report.cdm_version, CDM_VERSION);
    assert_eq!(report.package_version, PACKAGE_VERSION);
    assert_eq!(report.adapter, "mock");
    let text = report.to_string();
    for part in ["9001", CDM_VERSION, PACKAGE_VERSION, "mock"] {
        assert!(text.contains(part), "the report {text:?} lacks {part:?}");
    }
    // the package version is this package's, as the workspace sets it
    assert_eq!(PACKAGE_VERSION, env!("CARGO_PKG_VERSION"));

    // and without a file: the library's own two numbers and what it can open
    let library = library_versions();
    assert_eq!(library.cdm_version, CDM_VERSION);
    assert_eq!(library.package_version, PACKAGE_VERSION);
    assert_eq!(library.supported_versions, supported_versions());
}

// ---------------------------------------------------------------- the facade is the only surface

/// What in a consumer's code reaches past the facade: a path through the IO crate, an adapter, the dispatch core or an adapter's own types. Comments and
/// strings are not code.
fn internals_touched(file: &str, text: &str) -> Vec<String> {
    const INTERNALS: [&str; 5] = [
        "build_log_io",
        "adapters",
        "dispatch",
        "AdapterSet",
        "MarkerPlace",
    ];
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut found = Vec::new();
    for (n, line) in code_of(text, false).lines().enumerate() {
        for name in INTERNALS {
            let touched = line.match_indices(name).any(|(at, _)| {
                !line[..at].chars().next_back().is_some_and(is_ident)
                    && !line[at + name.len()..].chars().next().is_some_and(is_ident)
            });
            if touched {
                found.push(format!("{file}:{}: reaches `{name}`", n + 1));
            }
        }
    }
    found
}

/// What in the facade's own source re-exports more than the contract: an adapter, a whole crate of the IO layer, or more of the dispatch core than the two
/// types that a consumer names.
fn reexports_too_much(file: &str, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for statement in code_of(text, false).split(';') {
        if !statement.contains("pub use") {
            continue;
        }
        let statement = statement.split_whitespace().collect::<Vec<_>>().join(" ");
        if statement.contains("adapters") {
            found.push(format!("{file}: re-exports an adapter: `{statement}`"));
        }
        if statement.contains("build_log_io as") || statement.contains("build_log_io::*") {
            found.push(format!(
                "{file}: re-exports the whole IO crate: `{statement}`"
            ));
        }
        if let Some((_, after)) = statement.split_once("dispatch::") {
            let names: Vec<&str> = after
                .split(|c: char| c.is_whitespace() || matches!(c, '{' | '}' | ','))
                .filter(|n| !n.is_empty())
                .collect();
            if names
                .iter()
                .any(|n| !["Registry", "VersionKey"].contains(n))
            {
                found.push(format!(
                    "{file}: re-exports more of the dispatch core than Registry and VersionKey: `{statement}`"
                ));
            }
        }
    }
    found
}

fn sources_in(dir: &Path) -> Vec<(String, String)> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{} is not readable: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            (name, fs::read_to_string(&p).expect("a readable source"))
        })
        .collect()
}

#[test]
fn public_api_never_touches_adapter_internals() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // the code of a consumer: the example, and the tests that stand in for one
    let mut consumer = sources_in(&root.join("examples"));
    consumer.push((
        "tests/facade.rs".to_string(),
        fs::read_to_string(root.join("tests").join("facade.rs")).expect("this file"),
    ));
    assert!(
        consumer.iter().any(|(name, _)| name == "mock_consumer.rs"),
        "the guard did not find the consumer example"
    );
    let mut found = Vec::new();
    for (name, text) in &consumer {
        found.extend(internals_touched(name, text));
    }
    // and the facade's own source re-exports nothing of the adapters
    let own = sources_in(&root.join("src"));
    for expected in ["lib.rs", "open.rs", "testing.rs"] {
        assert!(
            own.iter().any(|(n, _)| n == expected),
            "the guard did not find src/{expected}"
        );
    }
    for (name, text) in &own {
        found.extend(reexports_too_much(name, text));
    }
    assert!(
        found.is_empty(),
        "the facade is not the only surface:\n{}",
        found.join("\n")
    );

    // the guards fail on code made to break them
    let breaking = [
        "use build_log_io::dispatch::open;",
        "let set = build_log::io::adapters::mock::adapter_set();",
        "fn f(set: AdapterSet) {}",
        "let place = MarkerPlace::new(\"a\", \"b\");",
    ];
    for source in breaking {
        assert!(
            !internals_touched("made_to_break.rs", source).is_empty(),
            "not rejected: {source:?}"
        );
    }
    let clean = [
        "// a consumer does not use build_log_io::dispatch or adapters\nfn f() {}",
        "let name = \"adapters\"; let dispatched = 1; let my_adapters = 2;",
    ];
    for source in clean {
        assert_eq!(
            internals_touched("clean.rs", source),
            Vec::<String>::new(),
            "rejected: {source:?}"
        );
    }
    let breaking = [
        "pub use build_log_io::adapters::mock;",
        "pub use build_log_io::adapters::{mock, AdapterSet};",
        "pub use build_log_io as io;",
        "pub use build_log_io::dispatch::{open, Registry};",
        "pub use build_log_io::dispatch::*;",
    ];
    for source in breaking {
        assert!(
            !reexports_too_much("made_to_break.rs", source).is_empty(),
            "not rejected: {source:?}"
        );
    }
    let clean = [
        "pub use build_log_io::dispatch::{Registry, VersionKey};",
        "pub use build_log_io::binding as hdf5;",
        "pub use build_log_model::*;",
        "pub mod io {\n    pub use build_log_io::{check_libhdf5, LIBHDF5_MINIMUM};\n}",
        "use build_log_io::adapters::mock;",
    ];
    for source in clean {
        assert_eq!(
            reexports_too_much("clean.rs", source),
            Vec::<String>::new(),
            "rejected: {source:?}"
        );
    }
}

#[test]
fn a_consumer_needs_no_change_for_a_new_version() {
    // the same two functions of the example, for a version that exists and for one that does not exist until the test injects it
    let registry = mock_registry(&["9001", "9002"]).expect("two mock versions register");
    let mut described = Vec::new();
    for key in ["9001", "9002"] {
        let file = temp(&format!("consumer-{key}"));
        write_mock_file(&file.0, key, "a build", 3).expect("a mock file");
        described.push(mock_consumer::run(&registry, &file.0).expect("the consumer describes it"));
    }
    assert!(
        described[0].contains("schema 9001") && described[0].contains("adapter mock"),
        "{}",
        described[0]
    );
    assert!(
        described[1].contains("schema 9002") && described[1].contains("adapter mock-9002"),
        "{}",
        described[1]
    );
    for text in &described {
        assert!(text.contains("build a build with 3 layers"), "{text}");
    }

    // and nothing in the consumer's code before `main` names a version: the version is data in the report
    let example = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("mock_consumer.rs"),
    )
    .expect("the example");
    let consumer = example
        .split("fn main")
        .next()
        .expect("the part before main");
    let code = code_of(consumer, true);
    assert!(
        !code.contains("9001") && !code.contains("9002"),
        "the consumer's code names a version:\n{code}"
    );

    // a version the registry does not know is refused, whatever the consumer is
    let file = temp("consumer-unknown");
    write_mock_file(&file.0, "9003", "a build", 1).expect("a mock file");
    let error = mock_consumer::run(&registry, &file.0).expect_err("9003 is not registered");
    assert!(
        matches!(
            error.kind(),
            ErrorKind::Unversioned { .. } | ErrorKind::UnsupportedVersion { .. }
        ),
        "{error:?}"
    );
}

// ---------------------------------------------------------------- the exit checklist (plan IP-04, section 7)

/// Each criterion of the exit checklist, with the tests that show it and where they are (from the `rust` folder). A test that was deleted or renamed makes the
/// checklist fail here, so a criterion cannot go unproven without anyone noticing. Whether they pass is the workspace's `cargo test` (criterion 1, with clippy,
/// the format check and the `wasm32` build of the Model, which CI runs).
const CHECKLIST: [(&str, &str, &[&str]); 8] = [
    (
        "2: the marker reads as a scalar and as an array of one, and one that cannot be a version is corrupt",
        "io/tests/marker.rs",
        &["a_marker_stored_as_a_scalar_or_an_array_of_one_gives_the_same_key", "a_marker_that_cannot_be_a_version_is_corrupt"],
    ),
    (
        "3: the registry rejects, dispatches, lists, refuses a duplicate and requires every surface",
        "io/tests/registry.rs",
        &[
            "open_registry_rejects_unregistered_version",
            "supported_versions_reflects_production_registry",
            "a_duplicate_key_is_refused",
            "every_registered_version_supplies_every_surface_or_declares_it_unsupported",
            "a_surface_cannot_be_left_undeclared",
        ],
    ),
    (
        "3: it dispatches through an injected adapter",
        "io/tests/dispatch.rs",
        &["open_registry_dispatches_via_injected_test_file"],
    ),
    (
        "4: no marker is Unversioned, a version without a writer refuses create, open preserves the version, two markers are refused",
        "io/tests/dispatch.rs",
        &["unversioned_file_returns_typed_outcome", "open_preserves_the_on_disk_version", "two_markers_on_two_mounts_are_refused"],
    ),
    (
        "4: a version without a writer answers create with UnsupportedInVersion",
        "io/tests/registry.rs",
        &["a_version_without_a_writer_answers_create_with_unsupported_in_version"],
    ),
    (
        "5: a mount nested at any depth gives the same build as a standalone file",
        "io/tests/dispatch.rs",
        &["group_attached_mode_matches_standalone_file_mode"],
    ),
    (
        "6: the isolation guards, and a second fake version touches nothing but its own code",
        "io/tests/adapter_isolation.rs",
        &[
            "adapter_does_not_reference_other_versions",
            "the_guard_rejects_an_adapter_made_to_break_it",
            "every_adapter_folder_is_registered_exactly_once",
        ],
    ),
    (
        "6: adding a version, and the dispatcher knows no layout name",
        "io/tests/adding_a_version.rs",
        &["adding_a_version_touches_only_its_own_adapter"],
    ),
];

#[test]
fn ip04_exit_checklist_holds() {
    let rust = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the rust folder");
    let mut missing = Vec::new();
    for (criterion, file, tests) in CHECKLIST {
        let text = fs::read_to_string(rust.join(file))
            .unwrap_or_else(|e| panic!("{file} is not readable: {e}"));
        for test in tests {
            if !text.contains(&format!("fn {test}()")) {
                missing.push(format!("criterion {criterion}: `{test}` is not in {file}"));
            }
        }
    }
    // the dispatcher's layout-name guard lives with the primitives' (one file, two guards)
    let guard = fs::read_to_string(rust.join("io/tests/primitives_layout_guard.rs"))
        .expect("the layout guard");
    if !guard.contains("fn the_dispatcher_holds_no_layout_names()") {
        missing.push("criterion 6: `the_dispatcher_holds_no_layout_names` is not in io/tests/primitives_layout_guard.rs".to_string());
    }
    // criterion 7 is the tests of this file; criterion 8 is a fact that can be read off the facade: no real version yet, and no mock in production
    for test in [
        "the_facade_opens_the_mock_version_through_an_injected_registry",
        "one_call_reports_the_three_versions_and_the_adapter",
        "public_api_never_touches_adapter_internals",
        "a_consumer_needs_no_change_for_a_new_version",
    ] {
        let this = fs::read_to_string(rust.join("build-log/tests/facade.rs")).expect("this file");
        if !this.contains(&format!("fn {test}()")) {
            missing.push(format!(
                "criterion 7: `{test}` is not in build-log/tests/facade.rs"
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "the exit checklist of IP-04 does not hold:\n{}",
        missing.join("\n")
    );

    // 8: the production registry is empty (until IP-13) and the mock is registered only by injection
    assert!(
        supported_versions().iter().all(|v| !v.starts_with("90")),
        "the mock is in the production registry: {:?}",
        supported_versions()
    );
    let registry = mock_registry(&["9001"]).expect("the mock registers by injection");
    assert_eq!(registry.supported_versions(), ["9001"]);
}
