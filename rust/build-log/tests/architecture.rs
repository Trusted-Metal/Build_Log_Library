//! The architecture of the workspace, checked on the real thing (IP-01 tasks T01.3 and T01.5; requirements BL-AR-01, BL-AR-02, BL-AR-06).
//!
//! The guards themselves are in `support`; `guards_reject_violations.rs` shows each of them catching a workspace made to break its rule.

#[allow(dead_code)]
mod support;

use std::fs;
use std::sync::OnceLock;

use support::*;

fn workspace() -> &'static Workspace {
    static WORKSPACE: OnceLock<Workspace> = OnceLock::new();
    WORKSPACE
        .get_or_init(|| load_workspace().expect("the workspace can be read with `cargo metadata`"))
}

// ---------------------------------------------------------------- phase 1B: the shape of the workspace

#[test]
fn workspace_has_the_four_layers_and_the_testkit() {
    assert_clean("the layers of the workspace", layer_violations(workspace()));
}

#[test]
fn every_crate_directory_is_a_workspace_member() {
    assert_clean(
        "the crate folders under rust/",
        crate_directory_violations(workspace()),
    );
}

#[test]
fn every_crate_opts_into_the_workspace_lints() {
    let root = repo_root();
    for (name, dir) in LAYERS {
        let manifest = fs::read_to_string(root.join(dir).join("Cargo.toml")).expect("a manifest");
        let lints = manifest.replace("\r\n", "\n");
        assert!(lints.contains("[lints]\nworkspace = true"), "{name} ({dir}) does not say `[lints] workspace = true`, so it escapes the workspace lints (D01-7)");
    }
}

#[test]
fn unsafe_code_is_forbidden_where_the_decision_says() {
    // D01-7: forbid in the Model, Processing and the facade; the IO layer stays at the workspace's `deny` (it may need `unsafe` for the HDF5 binding).
    let root = repo_root();
    for dir in ["rust/model", "rust/processing", "rust/build-log"] {
        let source =
            fs::read_to_string(root.join(dir).join("src").join("lib.rs")).expect("a lib.rs");
        assert!(
            source.contains("#![forbid(unsafe_code)]"),
            "{dir}/src/lib.rs does not forbid unsafe code (D01-7)"
        );
    }
}

#[test]
fn every_lib_rs_states_its_layer_and_what_it_may_depend_on() {
    let root = repo_root();
    for (name, dir) in LAYERS {
        let source = fs::read_to_string(root.join(dir).join("src").join("lib.rs"))
            .expect("a lib.rs")
            .replace("\r\n", "\n");
        assert!(
            source.lines().next().is_some_and(|l| l.starts_with("//!")),
            "{name}: lib.rs must start with a `//!` description"
        );
        assert!(
            source.contains("**Layer:**") && source.contains("**May depend on:**"),
            "{name}: its `//!` text must state its layer and what it may depend on"
        );
    }
}

#[test]
fn the_facade_reexports_the_model_at_its_root() {
    // the same type, not a copy: a value built through the facade is accepted where the Model's own type is expected
    assert_eq!(
        std::any::TypeId::of::<build_log::Build>(),
        std::any::TypeId::of::<build_log_model::Build>()
    );
    assert_eq!(
        std::any::TypeId::of::<build_log::Laser>(),
        std::any::TypeId::of::<build_log_model::Laser>()
    );
}

// ---------------------------------------------------------------- phase 1D: the minimum supported Rust version

#[test]
fn msrv_recorded_and_set() {
    // recorded in the README (measured: the oldest toolchain that builds and tests the workspace) and set in every crate (`rust-version.workspace = true`)
    let readme = fs::read_to_string(repo_root().join("README.md")).expect("the README");
    let recorded =
        recorded_msrv(&readme).expect("the README records the minimum supported Rust version");
    assert_clean(
        "the minimum supported Rust version",
        msrv_violations(workspace(), &recorded),
    );
}

#[test]
fn the_recorded_minimum_is_read_from_the_words_that_state_it() {
    assert_eq!(
        recorded_msrv("Minimum supported Rust version (measured): **1.85.1**.").as_deref(),
        Some("1.85.1")
    );
    assert_eq!(recorded_msrv("nothing here"), None);
    assert_eq!(parse_rust_version("1.85.1"), Some((1, 85, 1)));
    assert_eq!(parse_rust_version("1.85"), Some((1, 85, 0)));
    assert_eq!(parse_rust_version("one"), None);
    assert!(
        parse_rust_version("1.10.0") > parse_rust_version("1.9.9"),
        "versions compare as numbers, not text"
    );
}

// ---------------------------------------------------------------- phase 1C: the guards, on the real workspace

#[test]
fn layering_is_one_way() {
    assert_clean(
        "a dependency between workspace crates that Appendix C does not allow",
        layering_violations(workspace(), &allowed_dependencies()),
    );
}

#[test]
fn model_has_no_hdf5_dependency() {
    assert_clean("the Model reaches HDF5", model_hdf5_violations(workspace()));
}

#[test]
fn processing_does_not_depend_on_io() {
    assert_clean(
        "Processing reaches the IO layer",
        processing_io_violations(workspace()),
    );
}

#[test]
fn no_mcf_dependency() {
    assert_clean("the library depends on MCF", mcf_violations(workspace()));
}

#[test]
fn spec_directory_is_outside_language_directories() {
    assert_clean(
        "the specification folder",
        spec_location_violations(workspace()),
    );
}

#[test]
fn the_allowed_dependency_data_names_only_crates_of_the_workspace_or_of_a_later_plan() {
    // a typo in the data would silently allow nothing or everything; the one crate that does not exist yet is the conformance crate of IP-02
    let known: Vec<&str> = LAYERS
        .iter()
        .map(|(name, _)| *name)
        .chain(["build-log-conformance"])
        .collect();
    for rule in &allowed_dependencies().rules {
        assert!(
            known.contains(&rule.from.as_str()) && known.contains(&rule.to.as_str()),
            "the rule {} -> {} names an unknown crate",
            rule.from,
            rule.to
        );
        assert_ne!(
            rule.to, FACADE,
            "no rule may allow a dependency on the facade"
        );
        assert_ne!(
            rule.from, TESTKIT,
            "the test kit depends on no workspace crate"
        );
    }
}
