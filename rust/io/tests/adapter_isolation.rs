//! The isolation guards of the adapters (IP-04 phase 4E, task T04.9, decision D04-11; requirements BL-VA-05, BL-VA-06 and BL-TV-02).
//!
//! * `adapter_does_not_reference_other_versions`: each folder of `src/adapters/` is one version, and none of its code (comments and strings stripped) names another
//!   version's module as a path segment. A version that borrows from another can never be changed or removed alone (BL-VA-05), so this runs from the first commit
//!   and includes the mock version. The folders that are known or planned (`v16`, `v17`, `v18`, `mock`) are forbidden by name even before they exist.
//! * `every_adapter_folder_is_registered_exactly_once`: the production list of the registry names each folder once, the mock excepted, so that adding a version is a
//!   folder and one line, and a folder that was never registered (or was registered twice) is caught.
//!
//! * `layout_names_appear_only_in_the_layout_module` (IP-13 phase 13B, decision D13-10, requirement BL-ST-13): in a folder that has a `layout.rs`, no other file
//!   holds a name of that layout as a string. The names are read from the constants of `layout.rs`, so the guard needs no list of its own. Every folder but the
//!   mock has a `layout.rs`.
//!
//! Each guard is also run on source made to break it, so that one that stopped looking would be noticed.

use std::fs;
use std::path::{Path, PathBuf};

use build_log_testkit::scan::code_of;

/// The module names of the versions that exist or are planned. A folder of `adapters/` is named for its version.
const KNOWN_VERSIONS: [&str; 4] = ["mock", "v16", "v17", "v18"];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// What a folder must not name: every known version module and every other folder that exists.
fn forbidden_for(folder: &str, folders: &[String]) -> Vec<String> {
    let mut names: Vec<String> = KNOWN_VERSIONS.iter().map(|s| s.to_string()).collect();
    names.extend(folders.iter().cloned());
    names.sort();
    names.dedup();
    names.retain(|n| n != folder);
    names
}

/// The places in a file's code where another version's module is named as a segment of a path (`super::v18::read`, `crate::adapters::mock::x`, `v16::decode`).
/// A comment, a string, a longer identifier and a local variable that happens to be called the same are not.
fn violations(folder: &str, file: &str, text: &str, forbidden: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in code_of(text, false).lines().enumerate() {
        for name in forbidden {
            for (at, _) in line.match_indices(name.as_str()) {
                let before = line[..at].chars().next_back();
                let after = line[at + name.len()..].chars().next();
                if before.is_some_and(is_ident) || after.is_some_and(is_ident) {
                    continue; // part of a longer name
                }
                let on_a_path = line[..at].trim_end().ends_with("::")
                    || line[at + name.len()..].trim_start().starts_with("::");
                if on_a_path {
                    found.push(format!(
                        "adapters/{folder}/{file}:{}: names the module `{name}` of another version",
                        n + 1
                    ));
                }
            }
        }
    }
    found
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).expect("a readable folder").flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn adapters_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("adapters")
}

/// The folders of `adapters/`: one for each version.
fn adapter_folders() -> Vec<String> {
    let mut folders: Vec<String> = fs::read_dir(adapters_dir())
        .expect("the adapters folder")
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    folders.sort();
    folders
}

#[test]
fn adapter_does_not_reference_other_versions() {
    let folders = adapter_folders();
    // a guard that scanned nothing would pass for the wrong reason
    assert!(
        folders.iter().any(|f| f == "mock"),
        "the guard did not find the mock folder: {folders:?}"
    );
    let mut found = Vec::new();
    for folder in &folders {
        let forbidden = forbidden_for(folder, &folders);
        let root = adapters_dir().join(folder);
        let files = rust_files(&root);
        assert!(!files.is_empty(), "no source in adapters/{folder}");
        for file in files {
            let text = fs::read_to_string(&file).expect("a readable source");
            let name = file
                .strip_prefix(&root)
                .unwrap()
                .display()
                .to_string()
                .replace('\\', "/");
            found.extend(violations(folder, &name, &text, &forbidden));
        }
    }
    assert!(
        found.is_empty(),
        "adapters that name another version:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_guard_rejects_an_adapter_made_to_break_it() {
    let folders: Vec<String> = ["mock", "v17", "v18"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let breaking = [
        (
            "a use of a sibling's module",
            "v17",
            "use super::v18::read_layer;",
            "v18",
        ),
        (
            "a path through the crate",
            "v17",
            "let set = crate::adapters::mock::adapter_set();",
            "mock",
        ),
        (
            "a call through a bare path",
            "v18",
            "let x = v16::decode(1);",
            "v16",
        ),
        (
            "a path in a use",
            "mock",
            "use crate::adapters::v17::adapter_set;",
            "v17",
        ),
        (
            "a path from the crate root",
            "v17",
            "pub use ::build_log_io::adapters::v18 as other;",
            "v18",
        ),
    ];
    for (what, folder, source, named) in breaking {
        let found = violations(
            folder,
            "made_to_break.rs",
            source,
            &forbidden_for(folder, &folders),
        );
        assert!(
            found.iter().any(|f| f.contains(&format!("`{named}`"))),
            "the guard did not reject {what}: {source:?} gave {found:?}"
        );
    }
    // what is not a reference: the folder's own name, the framework, comments, strings, longer names and variables
    let clean = [
        (
            "its own module",
            "v17",
            "use super::v17::helper; v17::go();",
        ),
        (
            "the framework",
            "v17",
            "use super::set::AdapterSet; use crate::dispatch::VersionKey;",
        ),
        (
            "a comment",
            "v17",
            "// see super::v18::read for the other version\nfn f() {}",
        ),
        (
            "a doc comment",
            "v17",
            "/// Unlike crate::adapters::mock::read, this reads a layer.\nfn f() {}",
        ),
        (
            "a string",
            "v17",
            "let name = \"v18\"; let path = \"super::v18::x\";",
        ),
        (
            "a longer name",
            "v17",
            "let v180 = 1; let my_v18_value = v17::x(); let mock_data = 2;",
        ),
        (
            "a variable called the same",
            "v17",
            "let mock = 1; let v18 = mock + 1;",
        ),
    ];
    for (what, folder, source) in clean {
        let found = violations(folder, "clean.rs", source, &forbidden_for(folder, &folders));
        assert!(
            found.is_empty(),
            "the guard rejected {what}: {source:?} gave {found:?}"
        );
    }
}

/// The body of `fn build_production` in the registry's source, with comments and strings blanked, or `None`.
fn production_body(registry_source: &str) -> Option<String> {
    let code = code_of(registry_source, false);
    let start = code.find("fn build_production")?;
    let open = start + code[start..].find('{')?;
    let mut depth = 0;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(code[open..open + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// What is wrong with the production list of a registry that has these adapter folders: a folder that is not named exactly once, the mock named at all.
fn registration_problems(folders: &[String], registry_source: &str) -> Vec<String> {
    let Some(body) = production_body(registry_source) else {
        return vec!["`build_production` is not in the registry's source".to_string()];
    };
    let mut problems = Vec::new();
    for folder in folders.iter().filter(|f| f.as_str() != "mock") {
        let count = body.matches(&format!("adapters::{folder}::")).count();
        if count != 1 {
            problems.push(format!("the folder adapters/{folder} is named {count} times in the production list, and must be named once"));
        }
    }
    if body.contains("adapters::mock") {
        problems.push(
            "the mock version is in the production list: it is registered only by injection"
                .to_string(),
        );
    }
    problems
}

#[test]
fn every_adapter_folder_is_registered_exactly_once() {
    let registry = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("dispatch")
            .join("registry.rs"),
    )
    .expect("the registry's source");
    let folders = adapter_folders();
    assert!(
        folders.iter().any(|f| f == "mock"),
        "the guard did not find the mock folder: {folders:?}"
    );
    let problems = registration_problems(&folders, &registry);
    assert!(
        problems.is_empty(),
        "the production list:\n{}",
        problems.join("\n")
    );

    // and the guard fails on a list made to break it
    let two: Vec<String> = ["mock", "v17", "v18"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let list = |body: &str| {
        format!("fn build_production() -> Registry {{\n    let mut registry = Registry::new();\n{body}\n    registry\n}}")
    };
    let good = list("    registry.register(crate::adapters::v17::adapter_set()).unwrap();\n    registry.register(crate::adapters::v18::adapter_set()).unwrap();");
    assert!(
        registration_problems(&two, &good).is_empty(),
        "a complete list is accepted"
    );
    let missing = list("    registry.register(crate::adapters::v17::adapter_set()).unwrap();");
    assert!(
        registration_problems(&two, &missing)
            .iter()
            .any(|p| p.contains("adapters/v18")),
        "a folder that is not registered is reported"
    );
    let twice = list("    registry.register(crate::adapters::v17::adapter_set()).unwrap();\n    registry.register(crate::adapters::v17::adapter_set()).unwrap();\n    registry.register(crate::adapters::v18::adapter_set()).unwrap();");
    assert!(
        registration_problems(&two, &twice)
            .iter()
            .any(|p| p.contains("adapters/v17") && p.contains("2 times")),
        "a folder registered twice is reported"
    );
    let with_mock = list("    registry.register(crate::adapters::v17::adapter_set()).unwrap();\n    registry.register(crate::adapters::v18::adapter_set()).unwrap();\n    registry.register(crate::adapters::mock::adapter_set()).unwrap();");
    assert!(
        registration_problems(&two, &with_mock)
            .iter()
            .any(|p| p.contains("mock")),
        "the mock in the production list is reported"
    );
    let commented = list("    // crate::adapters::v17::adapter_set() is added by IP-13\n    registry.register(crate::adapters::v18::adapter_set()).unwrap();");
    assert!(
        registration_problems(&two, &commented)
            .iter()
            .any(|p| p.contains("adapters/v17")),
        "a comment is not a registration"
    );
    assert!(
        !registration_problems(&two, "fn something_else() {}").is_empty(),
        "a source without the production list is reported"
    );
}

// ---------------------------------------------------------------- layout names only in the layout module (BL-ST-13)

/// Does `name` occur in `text` as a whole word (not inside a longer identifier)?
fn has_word(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// The names that a layout module declares: the string constants (`pub const NAME: &str = "value";`), comments left out.
fn layout_names(layout_source: &str) -> Vec<String> {
    code_of(layout_source, true)
        .lines()
        .filter(|line| line.contains("const ") && line.contains(": &str"))
        .filter_map(|line| line.split('"').nth(1).map(str::to_string))
        .filter(|name| !name.is_empty())
        .collect()
}

/// Where the other files of a folder hold a name of its layout: in a string, or anywhere else in the code. A name is spelled in `layout.rs` and nowhere else,
/// so the search is of the whole line of code, with comments left out.
fn names_outside_the_layout(
    folder: &str,
    layout_source: &str,
    others: &[(String, String)],
) -> Vec<String> {
    let names = layout_names(layout_source);
    let mut found = Vec::new();
    for (file, text) in others {
        for (n, line) in code_of(text, true).lines().enumerate() {
            for name in &names {
                if has_word(line, name) {
                    found.push(format!(
                        "adapters/{folder}/{file}:{}: the code holds the layout name `{name}`: use the constant of layout.rs",
                        n + 1
                    ));
                }
            }
        }
    }
    found
}

#[test]
fn layout_names_appear_only_in_the_layout_module() {
    let mut checked = Vec::new();
    let mut without_layout = Vec::new();
    let mut found = Vec::new();
    for folder in adapter_folders() {
        let root = adapters_dir().join(&folder);
        let layout = root.join("layout.rs");
        if !layout.is_file() {
            without_layout.push(folder);
            continue;
        }
        let layout_source = fs::read_to_string(&layout).expect("a readable layout module");
        assert!(
            !layout_names(&layout_source).is_empty(),
            "adapters/{folder}/layout.rs declares no name: the guard would check nothing"
        );
        let others: Vec<(String, String)> = rust_files(&root)
            .into_iter()
            .filter(|file| file != &layout)
            .map(|file| {
                let name = file
                    .strip_prefix(&root)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/");
                (name, fs::read_to_string(&file).expect("a readable source"))
            })
            .collect();
        found.extend(names_outside_the_layout(&folder, &layout_source, &others));
        checked.push(folder);
    }
    // a guard that checked no folder would pass for the wrong reason; only the mock may go without a layout module
    assert!(
        checked.iter().any(|f| f == "v17"),
        "the guard did not check v17: {checked:?}"
    );
    assert_eq!(
        without_layout,
        ["mock"],
        "every folder but the mock has a layout.rs"
    );
    assert!(
        found.is_empty(),
        "layout names outside a layout module:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_layout_guard_rejects_a_name_outside_the_layout_module() {
    let layout = "//! the layout\npub const MOUNT: &str = \"Built_Part\";\n/// the marker\npub const MARKER: &str = \"File_Version\";\npub const LIST: [&str; 2] = [MOUNT, MARKER];\n";
    assert_eq!(layout_names(layout), ["Built_Part", "File_Version"]);
    let other = |text: &str| vec![("read.rs".to_string(), text.to_string())];
    let breaking = [
        (
            "a name as a string",
            "let group = \"Built_Part\";",
            "Built_Part",
        ),
        (
            "a name inside a message",
            "let m = format!(\"no File_Version on {}\", mount);",
            "File_Version",
        ),
        (
            "a name in a raw string",
            "let g = r#\"Built_Part\"#;",
            "Built_Part",
        ),
        (
            "a name after a character literal",
            "let q = '\"'; let g = \"File_Version\";",
            "File_Version",
        ),
    ];
    for (what, source, named) in breaking {
        let found = names_outside_the_layout("v17", layout, &other(source));
        assert!(
            found.iter().any(|f| f.contains(&format!("`{named}`"))),
            "the guard did not reject {what}: {source:?} gave {found:?}"
        );
    }
    let clean = [
        ("the constant", "let group = layout::MOUNT;"),
        ("a comment", "// the mount is Built_Part\nlet x = 1;"),
        (
            "a doc comment",
            "/// reads File_Version from the mount\nfn f() {}",
        ),
        (
            "a longer name",
            "let g = \"Built_Parts\"; let h = \"my_File_Version_2\";",
        ),
        ("another string", "let m = \"the mount has no marker\";"),
    ];
    for (what, source) in clean {
        let found = names_outside_the_layout("v17", layout, &other(source));
        assert!(
            found.is_empty(),
            "the guard rejected {what}: {source:?} gave {found:?}"
        );
    }
}
