//! Each guard of `support` is shown to catch a workspace made to break its rule (IP-01 task T01.5, decision D01-4). A rule that nothing can break
//! silently has to be tested against a case that breaks it, not only against the repository as it happens to be.

#[allow(dead_code)]
mod support;

use std::collections::{BTreeMap, BTreeSet};

use support::*;

fn dep(name: &str, kind: Kind, optional: bool) -> Dependency {
    Dependency {
        name: name.to_string(),
        kind,
        optional,
        origin: format!("../{name}"),
    }
}

fn package(name: &str, dir: &str, dependencies: Vec<Dependency>) -> Package {
    Package {
        name: name.to_string(),
        dir: dir.to_string(),
        publishable: false,
        lib_name: Some(name.replace('-', "_")),
        rust_version: Some("1.85.1".to_string()),
        dependencies,
    }
}

/// A workspace built the way the plan says it should be: the layers, and the edges Appendix C allows.
fn healthy() -> Workspace {
    let members = vec![
        package(MODEL, "rust/model", vec![]),
        package(
            IO,
            "rust/io",
            vec![
                dep(MODEL, Kind::Normal, false),
                dep(TESTKIT, Kind::Dev, false),
            ],
        ),
        package(
            PROCESSING,
            "rust/processing",
            vec![
                dep(MODEL, Kind::Normal, false),
                dep(TESTKIT, Kind::Dev, false),
            ],
        ),
        package(
            FACADE,
            "rust/build-log",
            vec![
                dep(MODEL, Kind::Normal, false),
                dep(IO, Kind::Normal, true),
                dep(PROCESSING, Kind::Normal, true),
                dep(TESTKIT, Kind::Dev, false),
            ],
        ),
        package(TESTKIT, "rust/testkit", vec![]),
    ];
    let mut members = members;
    members[3].lib_name = Some("build_log".to_string());
    let mut graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in &members {
        graph
            .entry(p.name.clone())
            .or_default()
            .extend(p.dependencies.iter().map(|d| d.name.clone()));
    }
    Workspace {
        members,
        graph,
        lock_packages: [MODEL, IO, PROCESSING, FACADE, TESTKIT, "serde_json"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        rust_versions: BTreeMap::new(),
        rust_dirs: ["build-log", "io", "model", "processing", "testkit"]
            .iter()
            .map(|d| (d.to_string(), true))
            .collect(),
        schema_top_level: true,
        schema_dirs_under_rust: vec![],
        text_files: vec![(
            "package.json".to_string(),
            "{\"name\": \"lbl-tooling\"}".to_string(),
        )],
    }
}

fn add_edge(ws: &mut Workspace, from: &str, to: &str) {
    ws.graph
        .entry(from.to_string())
        .or_default()
        .insert(to.to_string());
}

fn add_dependency(ws: &mut Workspace, from: &str, dependency: Dependency) {
    add_edge(ws, from, &dependency.name);
    ws.members
        .iter_mut()
        .find(|p| p.name == from)
        .expect("a member")
        .dependencies
        .push(dependency);
}

#[test]
fn the_healthy_workspace_passes_every_guard() {
    let ws = healthy();
    assert_eq!(
        layering_violations(&ws, &allowed_dependencies()),
        Vec::<String>::new()
    );
    assert!(model_hdf5_violations(&ws).is_empty() && processing_io_violations(&ws).is_empty());
    assert!(mcf_violations(&ws).is_empty() && spec_location_violations(&ws).is_empty());
    assert!(layer_violations(&ws).is_empty() && crate_directory_violations(&ws).is_empty());
    assert!(msrv_violations(&ws, "1.85.1").is_empty());
}

#[test]
fn each_guard_rejects_a_synthetic_violation() {
    type Guard = fn(&Workspace) -> Vec<String>;
    let allowed_layering: Guard = |ws| layering_violations(ws, &allowed_dependencies());
    let msrv_check: Guard = |ws| msrv_violations(ws, "1.85.1");
    let mut cases: Vec<(&str, Workspace, Guard, &str)> = Vec::new();

    // layering: every forbidden edge is named
    let mut ws = healthy();
    add_dependency(&mut ws, IO, dep(PROCESSING, Kind::Normal, false));
    cases.push((
        "IO depends on Processing",
        ws,
        allowed_layering,
        "build-log-io -> build-log-processing (normal)",
    ));
    let mut ws = healthy();
    add_dependency(&mut ws, PROCESSING, dep(IO, Kind::Normal, false));
    cases.push((
        "Processing depends on IO",
        ws,
        allowed_layering,
        "build-log-processing -> build-log-io (normal)",
    ));
    let mut ws = healthy();
    add_dependency(&mut ws, MODEL, dep(IO, Kind::Normal, false));
    cases.push((
        "the Model depends on IO",
        ws,
        allowed_layering,
        "build-log-model -> build-log-io (normal)",
    ));
    let mut ws = healthy();
    add_dependency(&mut ws, TESTKIT, dep(MODEL, Kind::Normal, false));
    cases.push((
        "the test kit depends on the Model",
        ws,
        allowed_layering,
        "build-log-testkit -> build-log-model (normal)",
    ));
    let mut ws = healthy();
    add_dependency(&mut ws, IO, dep(FACADE, Kind::Dev, false));
    cases.push((
        "IO depends on the facade",
        ws,
        allowed_layering,
        "nothing inside the workspace may depend on the facade",
    ));
    let mut ws = healthy();
    ws.members[3].dependencies[1].optional = false;
    cases.push((
        "the facade's IO dependency is not optional",
        ws,
        allowed_layering,
        "build-log -> build-log-io (normal): must be optional",
    ));
    let mut ws = healthy();
    ws.members[1].dependencies[1].kind = Kind::Normal;
    cases.push((
        "IO depends on the test kit outside tests",
        ws,
        allowed_layering,
        "build-log-io -> build-log-testkit (normal): allowed only as a dev dependency",
    ));

    // the Model reaches HDF5, however far down
    let mut ws = healthy();
    add_edge(&mut ws, MODEL, "some-helper");
    add_edge(&mut ws, "some-helper", "hdf5-metno");
    cases.push((
        "the Model reaches HDF5 through a helper",
        ws,
        model_hdf5_violations,
        "build-log-model -> some-helper -> hdf5-metno",
    ));

    // Processing reaches IO through another crate
    let mut ws = healthy();
    add_edge(&mut ws, PROCESSING, "some-helper");
    add_edge(&mut ws, "some-helper", IO);
    cases.push((
        "Processing reaches IO through a helper",
        ws,
        processing_io_violations,
        "build-log-processing -> some-helper -> build-log-io",
    ));

    // MCF, in each place it could come in
    let mut ws = healthy();
    add_edge(&mut ws, FACADE, "machine-config");
    cases.push((
        "an MCF package in the graph",
        ws,
        mcf_violations,
        "the dependency graph contains the MCF package `machine-config`",
    ));
    let mut ws = healthy();
    ws.lock_packages.insert("machine_config".to_string());
    cases.push((
        "an MCF package in Cargo.lock",
        ws,
        mcf_violations,
        "Cargo.lock contains the MCF package `machine_config`",
    ));
    let mut ws = healthy();
    add_dependency(
        &mut ws,
        IO,
        Dependency {
            name: "mcf-adapter".to_string(),
            kind: Kind::Normal,
            optional: false,
            origin: "../../Machine_Config_Library/rust".to_string(),
        },
    );
    cases.push((
        "a dependency from the MCF folder",
        ws,
        mcf_violations,
        "build-log-io depends on `mcf-adapter`",
    ));
    let mut ws = healthy();
    ws.text_files.push((
        "tools/fixtures/requirements.txt".to_string(),
        "h5py==3.16.0\nmachine-config-cli==1.0\n".to_string(),
    ));
    cases.push((
        "an MCF package in the Python requirements",
        ws,
        mcf_violations,
        "tools/fixtures/requirements.txt names an MCF package",
    ));

    // the specification folder
    let mut ws = healthy();
    ws.schema_top_level = false;
    cases.push((
        "no top-level schema folder",
        ws,
        spec_location_violations,
        "schema/ does not exist",
    ));
    let mut ws = healthy();
    ws.schema_dirs_under_rust = vec!["rust/model/schema".to_string()];
    cases.push((
        "a schema folder inside a language folder",
        ws,
        spec_location_violations,
        "rust/model/schema is a `schema` folder under rust/",
    ));

    // the minimum supported Rust version (phase 1D)
    let mut ws = healthy();
    ws.members[0].rust_version = None;
    cases.push((
        "a crate declares no rust-version",
        ws,
        msrv_check,
        "package `build-log-model` declares no rust-version",
    ));
    let mut ws = healthy();
    ws.members[1].rust_version = Some("1.80.0".to_string());
    cases.push((
        "a crate declares another rust-version",
        ws,
        msrv_check,
        "package `build-log-io` declares rust-version 1.80.0",
    ));
    let mut ws = healthy();
    ws.rust_versions
        .insert("hdf5-metno".to_string(), "1.90.0".to_string());
    cases.push((
        "a dependency needs a newer compiler",
        ws,
        msrv_check,
        "`hdf5-metno` requires Rust 1.90.0, newer than the recorded minimum 1.85.1",
    ));

    // the shape of the workspace (phase 1B)
    let mut ws = healthy();
    ws.members.retain(|p| p.name != TESTKIT);
    cases.push((
        "no test kit",
        ws,
        layer_violations,
        "the workspace has no package `build-log-testkit`",
    ));
    let mut ws = healthy();
    ws.members[0].publishable = true;
    cases.push((
        "a publishable crate",
        ws,
        layer_violations,
        "package `build-log-model` is publishable",
    ));
    let mut ws = healthy();
    ws.members[3].lib_name = Some("facade".to_string());
    cases.push((
        "the facade's library is misnamed",
        ws,
        layer_violations,
        "not `build_log`",
    ));
    let mut ws = healthy();
    ws.rust_dirs.push(("scratch".to_string(), false));
    cases.push((
        "a folder under rust/ with no manifest",
        ws,
        crate_directory_violations,
        "rust/scratch has no Cargo.toml",
    ));
    let mut ws = healthy();
    ws.rust_dirs.push(("extra".to_string(), true));
    cases.push((
        "a crate folder that is not a member",
        ws,
        crate_directory_violations,
        "rust/extra is not a member of the workspace",
    ));

    for (what, ws, guard, expected) in cases {
        let found = guard(&ws);
        assert!(
            !found.is_empty(),
            "the guard accepted a workspace where {what}"
        );
        assert!(
            found.iter().any(|f| f.contains(expected)),
            "where {what}, the guard did not name the offender ({expected:?}); it said: {found:?}"
        );
    }
}

#[test]
fn the_conformance_crate_of_a_later_plan_fits_the_matrix_without_changing_code() {
    // IP-02 adds `build-log-conformance`; the matrix is data, so its row is a line in tests/data/allowed_dependencies.json, not a change here
    let mut ws = healthy();
    ws.members.push(package(
        "build-log-conformance",
        "rust/conformance",
        vec![
            dep(MODEL, Kind::Normal, false),
            dep(TESTKIT, Kind::Dev, false),
        ],
    ));
    let found = layering_violations(&ws, &allowed_dependencies());
    assert_eq!(found, vec!["build-log-conformance -> build-log-testkit (dev): this dependency is not allowed (IP-01 Appendix C)".to_string()]);
}

#[test]
fn the_allowed_dependency_data_is_read_as_written() {
    let data = r#"{ "allowed": [ { "from": "a", "to": "b", "kinds": ["normal", "dev"], "optional": true } ] }"#;
    let allowed = allowed_from_json(data).expect("valid data");
    let rule = allowed.rule("a", "b").expect("the rule");
    assert_eq!(
        (rule.kinds.clone(), rule.optional),
        (vec![Kind::Normal, Kind::Dev], true)
    );
    assert!(allowed.rule("b", "a").is_none());
    assert!(allowed_from_json(
        "{ \"allowed\": [ { \"from\": \"a\", \"to\": \"b\", \"kinds\": [\"sideways\"] } ] }"
    )
    .is_err());
    assert!(allowed_from_json("not json").is_err());
}
