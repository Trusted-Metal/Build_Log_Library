//! The facade's feature matrix (IP-01 task T01.4, requirement BL-AR-07): each combination of features that CI needs builds, and
//! `--no-default-features` leaves the Model only. `static-hdf5` is left out: it compiles libhdf5 from source and is built by its own, advisory, CI job.
//!
//! The checks run `cargo` in a separate target folder (`target/feature-matrix`), so they neither wait for nor disturb the build that is running them.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("two folders below the repository root")
        .to_path_buf()
}

fn cargo(args: &[&str]) -> (bool, String, String) {
    let program = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let root = repo_root();
    let output = Command::new(program)
        .args(args)
        .arg("--locked")
        .current_dir(&root)
        .env(
            "CARGO_TARGET_DIR",
            root.join("target").join("feature-matrix"),
        )
        .output()
        .expect("cargo can be run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// The feature sets of the matrix, as the arguments that choose them.
const MATRIX: [(&str, &[&str]); 4] = [
    ("the default features", &[]),
    (
        "no default features (the Model only)",
        &["--no-default-features"],
    ),
    (
        "the Processing layer without the IO layer",
        &["--no-default-features", "--features", "processing"],
    ),
    (
        "every feature except static-hdf5",
        &["--features", "processing,jobs,specta"],
    ),
];

#[test]
fn every_feature_combination_in_the_matrix_builds() {
    for (what, features) in MATRIX {
        let mut args = vec!["check", "-p", "build-log"];
        args.extend_from_slice(features);
        let (ok, _out, err) = cargo(&args);
        assert!(
            ok,
            "the facade does not build with {what} (cargo check {features:?}):\n{err}"
        );
    }
}

/// The workspace packages the facade pulls in for a feature set, from `cargo tree`.
fn layers_in(features: &[&str]) -> Vec<String> {
    let mut args = vec![
        "tree",
        "-p",
        "build-log",
        "-e",
        "normal",
        "--prefix",
        "none",
    ];
    args.extend_from_slice(features);
    let (ok, out, err) = cargo(&args);
    assert!(ok, "cargo tree {features:?} failed:\n{err}");
    let mut found: Vec<String> = out
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| n.starts_with("build-log"))
        .map(str::to_string)
        .collect();
    found.sort();
    found.dedup();
    found
}

#[test]
fn no_default_features_leaves_the_model_only() {
    assert_eq!(
        layers_in(&["--no-default-features"]),
        vec!["build-log", "build-log-model"]
    );
}

#[test]
fn the_default_features_add_the_io_layer_and_the_processing_feature_adds_processing() {
    assert_eq!(
        layers_in(&[]),
        vec!["build-log", "build-log-io", "build-log-model"]
    );
    assert_eq!(
        layers_in(&["--no-default-features", "--features", "processing"]),
        vec!["build-log", "build-log-model", "build-log-processing"]
    );
    assert_eq!(
        layers_in(&["--features", "processing"]),
        vec![
            "build-log",
            "build-log-io",
            "build-log-model",
            "build-log-processing"
        ]
    );
}
