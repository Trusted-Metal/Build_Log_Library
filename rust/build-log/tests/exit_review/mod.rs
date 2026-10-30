//! What the exit reviews of the plans have in common (IP-06 phase 6H, IP-07 phase 7H, and the plans after them): a checklist is read off the code, and each criterion that names tests is held to them.
//!
//! * [`tests_that_are_gone`]: each criterion lists the tests that hold it, by name and by file, and a test that was renamed or deleted is a criterion that no longer holds.
//! * [`unsafe_in_the_workspace`] and [`hdf5_in_the_model`]: the two facts about the code that every plan's last criterion about the shape of the code asks for (no `unsafe`, and the Model has no HDF5).
//!
//! Each returns the problems it found, as sentences that name the criterion, so that a review gathers them and says them all at once.

use std::fs;
use std::path::{Path, PathBuf};

/// The criteria that name tests: the criterion, the file (below `rust/`) and the tests that hold it.
pub type Checklist<'a> = &'a [(&'a str, &'a str, &'a [&'a str])];

/// The `rust/` folder of the workspace.
pub fn rust() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the rust folder")
        .to_path_buf()
}

/// The `.rs` files below a folder.
pub fn sources(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(folder).expect("a folder").flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// Whether a line of code uses the keyword `unsafe` (a comment is not code, and `unsafe_code` in a lint is another word).
pub fn has_unsafe(line: &str) -> bool {
    if line.trim_start().starts_with("//") {
        return false;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices("unsafe").any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + "unsafe".len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// Each test of the checklist that is not in the file the checklist says it is in.
pub fn tests_that_are_gone(checklist: Checklist) -> Vec<String> {
    let rust = rust();
    let mut missing = Vec::new();
    for (criterion, file, tests) in checklist {
        let text = fs::read_to_string(rust.join(file))
            .unwrap_or_else(|e| panic!("{file} is not readable: {e}"));
        for test in *tests {
            if !text.contains(&format!("fn {test}(")) {
                missing.push(format!("criterion {criterion}: `{test}` is not in {file}"));
            }
        }
    }
    missing
}

/// Criterion 9: no `unsafe` in any crate of the workspace, and the workspace denies it.
pub fn unsafe_in_the_workspace() -> Vec<String> {
    let rust = rust();
    let mut missing = Vec::new();
    let manifest = fs::read_to_string(rust.parent().expect("the repository").join("Cargo.toml"))
        .expect("the workspace manifest");
    if !manifest.contains("unsafe_code = \"deny\"") {
        missing.push("criterion 9: the workspace no longer denies `unsafe_code`".to_string());
    }
    let mut files = Vec::new();
    for crate_dir in fs::read_dir(&rust).expect("the crates").flatten() {
        let src = crate_dir.path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    assert!(files.len() > 40, "the scan found {} sources", files.len());
    for file in &files {
        let text = fs::read_to_string(file).expect("a source");
        for (number, line) in text.lines().enumerate() {
            if has_unsafe(line) {
                missing.push(format!(
                    "criterion 9: `unsafe` at {}:{}",
                    file.strip_prefix(&rust).unwrap_or(file).display(),
                    number + 1
                ));
            }
        }
    }
    missing
}

/// Criterion 9: the Model still has no HDF5, not in its manifest and not in the code of its sources.
pub fn hdf5_in_the_model() -> Vec<String> {
    let rust = rust();
    let mut missing = Vec::new();
    let model_manifest =
        fs::read_to_string(rust.join("model/Cargo.toml")).expect("the Model's manifest");
    let mut in_dependencies = false;
    for line in model_manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line.contains("dependencies");
        } else if in_dependencies && line.to_lowercase().contains("hdf5") {
            missing.push(format!("criterion 9: the Model depends on HDF5: `{line}`"));
        }
    }
    let mut model_files = Vec::new();
    sources(&rust.join("model/src"), &mut model_files);
    for file in &model_files {
        let text = fs::read_to_string(file).expect("a source");
        for (number, line) in text.lines().enumerate() {
            let code = !line.trim_start().starts_with("//");
            if code && (line.contains("use hdf5") || line.contains("hdf5::")) {
                missing.push(format!(
                    "criterion 9: the Model uses HDF5 at {}:{}",
                    file.strip_prefix(&rust).unwrap_or(file).display(),
                    number + 1
                ));
            }
        }
    }
    missing
}
