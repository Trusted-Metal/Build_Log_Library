//! The HDF5 primitives are layout-free (IP-03 phase 3C, task T03.1, decision D03-11, requirement BL-ST-13).
//!
//! `no_layout_name_appears_in_the_primitives` reads every source file of `src/hdf5/` and fails if the code of one holds the name of a group, dataset or
//! attribute of any version, a numbered or generic group name (`Layer_12`, `Laser_N`), or a string that looks like an HDF5 path. A name is the business of
//! an adapter, which knows its version's layout; a primitive reads and writes a value with its encoding and knows nothing else.
//! `the_guard_rejects_a_source_made_to_break_it` shows that the guard fails on a source written to break each of its rules, and passes a source that only
//! mentions a name in a comment. A comment is documentation, not code.
//!
//! It is a text scanner, in the pattern of the Model's `policy_guards.rs`, so it needs no dependency and no HDF5 file.

use std::fs;
use std::path::{Path, PathBuf};

use build_log_testkit::scan::code_of;

/// Names of groups, datasets and attributes of the committed miniatures that mean something in a version's layout (read from them on 2026-10-22). A name
/// that is also an everyday word (`Value`, `State`, `Date`) is left out on purpose: the types of the primitives use such words. A new version that
/// brings names of this kind adds them here.
const LAYOUT_NAMES: [&str; 54] = [
    // groups and datasets
    "Built_Part",
    "Calibration_Data",
    "Monitoring_Data",
    "Correction_Data",
    "Correction_File",
    "Inverse_Correction_Data",
    "Derivation_Equation_Constants",
    "Global_Sensors",
    "Laser_Calibration_Data",
    "Laser_Data",
    "Laser_Power",
    "Laser_State",
    "Light_Source",
    "Power_Calibration",
    "Scanner_Card",
    "Synchronous_Analog_Sensors",
    "Synchronous_Digital_Sensors",
    "BitsToMillimetersFormula",
    "VoltsToWattsFormula",
    "CorrectionData",
    "Pre_Exposure",
    "Post_Exposure",
    "run_events",
    "sensor_events",
    "task_events",
    "RunState",
    "Source_ts",
    "Source_Ts",
    "Galvo_X",
    "Galvo_Y",
    "Galvo_Z",
    "Actual_X",
    "Actual_Y",
    "Actual_Z",
    "Commanded_X",
    "Commanded_Y",
    "Commanded_Z",
    "Start_ns",
    "Stop_ns",
    // attributes
    "File_Version",
    "Layer_Quantity",
    "Layer_Thickness",
    "Start_Layer_Time",
    "End_Layer_Time",
    "Start_Laser_Time",
    "End_Laser_Time",
    "Power_Min",
    "Power_Max",
    "Sample_Period",
    "Number_Of_Parts",
    "Number_Of_Sensors",
    "Build_ID",
    "Machine_Serial",
    "Scanner_ID",
];

/// A numbered or generic group (`Layer_12`, `Laser_N`, `Optical_Train_2`, `Layer_{n}`).
const LAYOUT_PREFIXES: [&str; 4] = ["Layer_", "Laser_", "Optical_Train_", "Laser_Calibration_"];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The contents of the string literals of one line of code.
fn string_literals(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current: Option<String> = None;
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        match (&mut current, ch) {
            (None, '"') => current = Some(String::new()),
            (Some(_), '"') => found.extend(current.take()),
            (Some(s), '\\') => {
                s.push(ch);
                if let Some(escaped) = chars.next() {
                    s.push(escaped);
                }
            }
            (Some(s), _) => s.push(ch),
            (None, _) => {}
        }
    }
    found
}

/// Does `name` occur in `line` as a whole word (not inside a longer identifier)?
fn has_word(line: &str, name: &str) -> bool {
    line.match_indices(name).any(|(p, _)| {
        let before = line[..p].chars().next_back();
        let after = line[p + name.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// What is wrong with one source file: the layout names, numbered group names and path-like strings in its code.
fn violations(file: &str, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in code_of(text, true).lines().enumerate() {
        let at = n + 1;
        for name in LAYOUT_NAMES {
            if has_word(line, name) {
                found.push(format!("{file}:{at}: the layout name `{name}` (a group, dataset or attribute of the files) is in the primitives"));
            }
        }
        for prefix in LAYOUT_PREFIXES {
            let numbered = line.match_indices(prefix).any(|(p, _)| {
                let before = line[..p].chars().next_back();
                let after = line[p + prefix.len()..].chars().next();
                !before.is_some_and(is_ident)
                    && after.is_some_and(|c| c.is_ascii_digit() || matches!(c, '{' | 'N' | 'M'))
            });
            if numbered {
                found.push(format!("{file}:{at}: a numbered or generic group name (`{prefix}…`) is in the primitives"));
            }
        }
        for literal in string_literals(line) {
            let body = literal.strip_prefix('/').unwrap_or("");
            if body.starts_with(|c: char| c.is_ascii_alphabetic()) && body.contains('/') {
                found.push(format!(
                    "{file}:{at}: the string \"{literal}\" looks like an HDF5 path"
                ));
            }
        }
    }
    found
}

fn source_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)
        .expect("the primitives folder is readable")
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            files.extend(source_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

#[test]
fn no_layout_name_appears_in_the_primitives() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("hdf5");
    let files = source_files(&dir);
    // a guard that scanned nothing would pass for the wrong reason
    let names: Vec<String> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    for expected in [
        "mod.rs",
        "value.rs",
        "attr.rs",
        "dataset.rs",
        "range.rs",
        "error.rs",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "the guard did not find {expected} in {}",
            dir.display()
        );
    }
    let mut found = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("a readable source");
        found.extend(violations(
            &file.strip_prefix(&dir).unwrap().display().to_string(),
            &text,
        ));
    }
    assert!(
        found.is_empty(),
        "layout names in the primitives:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_guard_rejects_a_source_made_to_break_it() {
    let breaking = [
        (
            "a group name in code",
            "pub const GROUP: &str = \"Monitoring_Data\";",
            "the layout name `Monitoring_Data`",
        ),
        (
            "an attribute name in code",
            "fn marker() -> &'static str { \"File_Version\" }",
            "the layout name `File_Version`",
        ),
        (
            "a name as an identifier",
            "pub struct Built_Part;",
            "the layout name `Built_Part`",
        ),
        (
            "a numbered group",
            "let group = format!(\"Layer_{}\", n);",
            "numbered or generic group name",
        ),
        (
            "a generic group",
            "let kind = \"Laser_N\";",
            "numbered or generic group name",
        ),
        (
            "a path in a string",
            "let path = \"/Built_Part/Layer_1\";",
            "looks like an HDF5 path",
        ),
        (
            "a name in a raw string",
            "let g = r#\"Calibration_Data\"#;",
            "the layout name `Calibration_Data`",
        ),
        (
            "a name after a character literal",
            "let q = '\"'; let g = \"Pre_Exposure\";",
            "the layout name `Pre_Exposure`",
        ),
    ];
    for (what, source, expected) in breaking {
        let found = violations("made_to_break.rs", source);
        assert!(
            found.iter().any(|f| f.contains(expected)),
            "the guard did not reject {what}: {source:?} gave {found:?}"
        );
    }
    // a name in a comment is documentation, not code; and words that only look like names are not names
    let clean = [
        "// File_Version is read by the marker reader of IP-04\npub struct Encoding;",
        "/// The attribute `Power_Min` holds raw bytes in V17 March.\npub struct Value;",
        "/* Layer_12 and /Built_Part/Layer_1 in a block comment, /* nested Monitoring_Data */ still a comment */\nfn f() {}",
        "pub struct Player_1;\npub fn laser_state_of(x: u8) -> u8 { x }\nlet note = \"a value\";",
        "fn lifetimes<'a>(x: &'a str) -> &'a str { x }",
    ];
    for source in clean {
        let found = violations("clean.rs", source);
        assert!(
            found.is_empty(),
            "the guard rejected a clean source: {source:?} gave {found:?}"
        );
    }
}

/// The Rust files that lie directly in a folder, not those of its subfolders.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .expect("the folder is readable")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();
    files
}

#[test]
fn the_dispatcher_holds_no_layout_names() {
    // BL-VA-02: the dispatcher reads the marker, looks the version up and hands over, and knows no group or attribute. The place of the marker is the
    // data that each adapter set declares. The guard scans the dispatch core and the files of `adapters/` that are not a version; the folders of
    // `adapters/` are versions, and naming their layout is their job.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = source_files(&src.join("dispatch"));
    files.extend(files_in(&src.join("adapters")));
    let names: Vec<String> = files
        .iter()
        .map(|f| {
            f.strip_prefix(&src)
                .unwrap()
                .display()
                .to_string()
                .replace('\\', "/")
        })
        .collect();
    // a guard that scanned nothing would pass for the wrong reason
    for expected in [
        "dispatch/mod.rs",
        "dispatch/marker.rs",
        "dispatch/registry.rs",
        "dispatch/open.rs",
        "adapters/mod.rs",
        "adapters/set.rs",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "the guard did not find {expected}"
        );
    }
    let mut found = Vec::new();
    for (file, name) in files.iter().zip(&names) {
        let text = fs::read_to_string(file).expect("a readable source");
        found.extend(violations(name, &text));
    }
    assert!(
        found.is_empty(),
        "layout names in the dispatcher:\n{}",
        found.join("\n")
    );
}
