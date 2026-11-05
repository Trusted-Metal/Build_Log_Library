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
//! * `an_adapter_borrows_no_file_of_another_folder` and `only_the_registry_names_a_version_outside_the_adapters` (IP-12 phase 12E, requirements BL-VA-05 and BL-VA-06): **the versions are separate in
//!   both directions.** An adapter does not take a file from outside its folder by a way that names no module (`#[path = "../v17/x.rs"]`, `include!`, a string that climbs out of the folder), and nothing
//!   of the shared code names a version's module (the toolkit has its own guard below): the registry alone does, so that a version is a folder and a line, and can be added or removed alone.
//!
//! * `the_toolkit_holds_only_what_two_versions_use` (IP-12 phase 12E, decision D12-12): every public function of the toolkit is used by at least two adapters, or by other code of the toolkit (which
//!   is shared plumbing of a part that two versions use); one that a single version uses lives in that version, and an exception names the second user that is expected.
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

// ---------------------------------------------------------------- the versions are separate in both directions (IP-12 phase 12E)

/// What in an adapter's code takes a file or a module from outside its folder **without naming a module on a path**, so that [`violations`] cannot see it: an attribute that sets the file of a
/// module, a macro that includes a file, and a string that climbs out of the folder. Comments are left out and strings are kept, because the borrowing is in the string.
fn borrowings(folder: &str, file: &str, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in code_of(text, true).lines().enumerate() {
        for (token, what) in [
            ("#[path", "sets the file of a module"),
            ("include!", "includes a file"),
            ("include_str!", "includes a file as text"),
            ("include_bytes!", "includes a file as bytes"),
            ("../", "climbs out of its folder"),
            ("..\\", "climbs out of its folder"),
        ] {
            if line.contains(token) {
                found.push(format!(
                    "adapters/{folder}/{file}:{}: `{token}` {what}: a version takes nothing from another folder",
                    n + 1
                ));
            }
        }
    }
    found
}

#[test]
fn an_adapter_borrows_no_file_of_another_folder() {
    let folders = adapter_folders();
    let mut found = Vec::new();
    let mut scanned = 0;
    for folder in &folders {
        let root = adapters_dir().join(folder);
        for file in rust_files(&root) {
            let text = fs::read_to_string(&file).expect("a readable source");
            let name = file
                .strip_prefix(&root)
                .unwrap()
                .display()
                .to_string()
                .replace('\\', "/");
            found.extend(borrowings(folder, &name, &text));
            scanned += 1;
        }
    }
    // a guard that scanned nothing would pass for the wrong reason: the mock, V16 and V17 are 20 files
    assert!(scanned >= 20, "the guard scanned only {scanned} files");
    assert!(
        found.is_empty(),
        "adapters that borrow from another folder:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_borrowing_guard_rejects_an_adapter_made_to_break_it() {
    let breaking = [
        (
            "a module whose file is in another folder",
            "#[path = \"../v17/columns.rs\"]\nmod borrowed;",
        ),
        (
            "a file included as code",
            "include!(\"../v17/layout_table.rs\");",
        ),
        (
            "a file included as text",
            "const T: &str = include_str!(\"other/notes.txt\");",
        ),
        (
            "a file included as bytes",
            "const B: &[u8] = include_bytes!(\"table.bin\");",
        ),
        (
            "a string that climbs out of the folder",
            "let path = \"../v17/layout.rs\";",
        ),
    ];
    for (what, source) in breaking {
        let found = borrowings("v16", "made_to_break.rs", source);
        assert!(
            !found.is_empty(),
            "the guard did not reject {what}: {source:?}"
        );
    }
    // what is not a borrowing: a comment, a module of the folder's own, the framework, and the two dots of a path inside a file
    let clean = [
        (
            "a comment",
            "// #[path = \"../v17/x.rs\"] would borrow\nfn f() {}",
        ),
        (
            "a doc comment",
            "/// include!(\"../x.rs\") is not used\nfn f() {}",
        ),
        ("a module of its own", "mod layers;\nuse super::layout;"),
        (
            "the framework",
            "use crate::toolkit::values::optional_text;",
        ),
        (
            "the two dots of a node's path",
            "let up = [String::from(\"..\")];",
        ),
    ];
    for (what, source) in clean {
        let found = borrowings("v16", "clean.rs", source);
        assert!(
            found.is_empty(),
            "the guard rejected {what}: {source:?} gave {found:?}"
        );
    }
}

/// The places in the source of the IO layer, outside the adapters, where a version's module is named as a path segment. The registry is the one file that may, for the versions that it registers.
fn versions_named_outside_the_adapters(file: &str, text: &str) -> Vec<String> {
    let all: Vec<String> = KNOWN_VERSIONS.iter().map(|s| s.to_string()).collect();
    violations("", file, text, &all)
        .into_iter()
        .map(|found| found.replacen("adapters//", "", 1))
        .collect()
}

#[test]
fn only_the_registry_names_a_version_outside_the_adapters() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut scanned = 0;
    for file in rust_files(&src) {
        let name = file
            .strip_prefix(&src)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        // the adapters are the versions; the registry is the one place that lists them (and `every_adapter_folder_is_registered_exactly_once` checks that list)
        if name.starts_with("adapters/") || name == "dispatch/registry.rs" {
            continue;
        }
        let text = fs::read_to_string(&file).expect("a readable source");
        found.extend(versions_named_outside_the_adapters(&name, &text));
        scanned += 1;
    }
    // the dispatcher, the primitives, the column engine and the toolkit: more than a dozen files
    assert!(scanned > 12, "the guard scanned only {scanned} files");
    assert!(
        found.is_empty(),
        "code outside the adapters that names a version's module:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_guard_of_the_shared_code_rejects_a_source_made_to_break_it() {
    let breaking = [
        (
            "a use of a version's module",
            "use crate::adapters::v17::layout;",
        ),
        (
            "a call through a path",
            "let set = crate::adapters::v16::adapter_set();",
        ),
        (
            "a path in a type",
            "fn f(x: super::adapters::v18::Source) {}",
        ),
        ("the mock", "use crate::adapters::mock::adapter_set;"),
    ];
    for (what, source) in breaking {
        let found = versions_named_outside_the_adapters("columns/engine.rs", source);
        assert!(
            !found.is_empty(),
            "the guard did not reject {what}: {source:?}"
        );
    }
    // the framework is not a version: the set, the registry, the dispatcher, and a comment, a string or a longer name
    let clean = [
        (
            "the framework",
            "use crate::adapters::{AdapterSet, Surface};",
        ),
        ("a comment", "// the v17 adapter reads this\nfn f() {}"),
        ("a string", "let label = \"adapters::v17\";"),
        ("a longer name", "let v170 = 1; let a_v17_value = 2;"),
    ];
    for (what, source) in clean {
        let found = versions_named_outside_the_adapters("columns/engine.rs", source);
        assert!(
            found.is_empty(),
            "the guard rejected {what}: {source:?} gave {found:?}"
        );
    }
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

// ---------------------------------------------------------------- the toolkit names no layout and no version (IP-12 phase 12A, decision D12-1)

/// How a version is called in a message. A label is data that an adapter passes to the toolkit, never a word of the toolkit's own code.
const VERSION_LABELS: [&str; 3] = ["V16", "V17", "V18"];

fn toolkit_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("toolkit")
}

/// The names that the layout modules of all the adapters declare and that look like the name of a thing in a file (a capital letter, four characters or more): the guard needs no list of
/// its own, and a version added later is guarded at once.
fn all_layout_names() -> Vec<String> {
    let mut names = Vec::new();
    for folder in adapter_folders() {
        if let Ok(source) = fs::read_to_string(adapters_dir().join(&folder).join("layout.rs")) {
            names.extend(layout_names(&source));
        }
    }
    names.retain(|n| n.len() >= 4 && n.chars().any(|c| c.is_ascii_uppercase()));
    names.sort();
    names.dedup();
    names
}

/// What is wrong with one source of the toolkit: the name of a layout, a version's module (or the adapters) as a segment of a path, or a version's label, in its code.
fn toolkit_problems(file: &str, text: &str, names: &[String], folders: &[String]) -> Vec<String> {
    let mut forbidden = forbidden_for("", folders);
    forbidden.push("adapters".to_string());
    let mut found: Vec<String> = violations("toolkit", file, text, &forbidden)
        .into_iter()
        .map(|message| message.replacen("adapters/toolkit/", "toolkit/", 1))
        .collect();
    for (n, line) in code_of(text, true).lines().enumerate() {
        for name in names {
            if has_word(line, name) {
                found.push(format!(
                    "toolkit/{file}:{}: the code holds the layout name `{name}`: an adapter passes its names as data",
                    n + 1
                ));
            }
        }
        for label in VERSION_LABELS {
            if has_word(line, label) {
                found.push(format!(
                    "toolkit/{file}:{}: the code names the version `{label}`: an adapter passes it as data",
                    n + 1
                ));
            }
        }
    }
    found
}

#[test]
fn the_toolkit_names_no_attribute_group_or_version() {
    let dir = toolkit_dir();
    let files = rust_files(&dir);
    // a guard that scanned nothing would pass for the wrong reason
    for expected in [
        "mod.rs",
        "values.rs",
        "content.rs",
        "context.rs",
        "lasers.rs",
        "columns.rs",
        "source.rs",
    ] {
        assert!(
            files
                .iter()
                .any(|f| f.file_name().is_some_and(|n| n == expected)),
            "the guard did not find toolkit/{expected}"
        );
    }
    let names = all_layout_names();
    assert!(
        names.iter().any(|n| n == "Built_Part") && names.len() > 50,
        "the guard read no layout names: {names:?}"
    );
    let folders = adapter_folders();
    let mut found = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("a readable source");
        let name = file
            .strip_prefix(&dir)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        found.extend(toolkit_problems(&name, &text, &names, &folders));
    }
    assert!(
        found.is_empty(),
        "the toolkit names a layout or a version:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_toolkit_guard_rejects_a_source_made_to_break_it() {
    let names: Vec<String> = ["Laser_Power", "Built_Part"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let folders: Vec<String> = ["mock", "v17", "v18"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let breaking = [
        (
            "a layout name in a string",
            "let group = \"Laser_Power\";",
            "the layout name `Laser_Power`",
        ),
        (
            "a layout name as a constant",
            "pub const MOUNT: &str = \"Built_Part\";",
            "the layout name `Built_Part`",
        ),
        (
            "a version's module on a path",
            "use crate::adapters::v17::layout;",
            "names the module `v17`",
        ),
        (
            "the adapters module",
            "use super::adapters::set::AdapterSet;",
            "names the module `adapters`",
        ),
        (
            "a version's label in a message",
            "let text = \"V17 has none that `read` serves\";",
            "the version `V17`",
        ),
        (
            "a version's label as a word",
            "let label = V16;",
            "the version `V16`",
        ),
    ];
    for (what, source, expected) in breaking {
        let found = toolkit_problems("made_to_break.rs", source, &names, &folders);
        assert!(
            found.iter().any(|f| f.contains(expected)),
            "the guard did not reject {what}: {source:?} gave {found:?}"
        );
    }
    // what is not a reference: a comment, a layout value that was passed in as data, a longer name
    let clean = [
        ("a comment", "// Laser_Power in V17 is the power\nfn f() {}"),
        (
            "a doc comment",
            "/// reads Built_Part of V16 and V18\nfn f() {}",
        ),
        (
            "a layout value passed as data",
            "let group = layout.layers_group;",
        ),
        (
            "a longer name",
            "let v170 = 1; let my_V17_thing = 2; let Laser_Powers = 3;",
        ),
    ];
    for (what, source) in clean {
        let found = toolkit_problems("clean.rs", source, &names, &folders);
        assert!(
            found.is_empty(),
            "the guard rejected {what}: {source:?} gave {found:?}"
        );
    }
}

/// What is wrong with the folders of `adapters/` as far as the toolkit is concerned: a folder named for it. Every folder there is a version, so the shared code lives beside them.
fn toolkit_folder_problems(folders: &[String]) -> Vec<String> {
    folders
        .iter()
        .filter(|folder| folder.as_str() == "toolkit")
        .map(|_| {
            "adapters/toolkit is a folder of the adapters, and every folder there is a version: the code that versions share is in src/toolkit".to_string()
        })
        .collect()
}

#[test]
fn no_adapter_folder_is_named_toolkit() {
    let folders = adapter_folders();
    assert!(
        folders.iter().any(|f| f == "v17"),
        "the guard did not find the v17 folder: {folders:?}"
    );
    assert!(
        toolkit_dir().join("mod.rs").is_file(),
        "the toolkit is src/toolkit, beside the adapters and not in them"
    );
    let problems = toolkit_folder_problems(&folders);
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    // and the guard fails on a list of folders made to break it
    let with_it: Vec<String> = ["mock", "toolkit", "v17"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(toolkit_folder_problems(&with_it).len(), 1);
    let without: Vec<String> = ["mock", "v17", "v18"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert!(toolkit_folder_problems(&without).is_empty());
}

// ---------------------------------------------------------------- the toolkit holds what two versions use (IP-12 phase 12E, decision D12-12)

/// The free functions that a toolkit file makes public: `pub fn name` at the start of a line. Methods are indented and are not counted: their names (`new`, `read`) are too common for a count of
/// words to mean anything.
fn public_functions(text: &str) -> Vec<String> {
    code_of(text, false)
        .lines()
        .filter_map(|line| line.strip_prefix("pub fn "))
        .filter_map(|rest| rest.split(|c: char| !is_ident(c)).next())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// Does `code` use the function `name`: the word, anywhere but where a function of that name is being defined (`fn name`).
fn uses_function(code: &str, name: &str) -> bool {
    code.lines().any(|line| {
        line.match_indices(name).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            let after = line[at + name.len()..].chars().next();
            if before.is_some_and(is_ident) || after.is_some_and(is_ident) {
                return false;
            }
            let head = line[..at].trim_end();
            let defined_here = head.ends_with("fn")
                && !head[..head.len() - 2]
                    .chars()
                    .next_back()
                    .is_some_and(is_ident);
            !defined_here
        })
    })
}

/// The public functions of the toolkit that fewer than two adapters use and that no other toolkit file uses, and the exceptions that are no longer needed. `toolkit` and `adapters` are
/// `(file or folder, source)`; the mock version is a stand-in for tests and is not a version; `allowed` is `(function, who is expected to use it next)`.
fn sharing_problems(
    toolkit: &[(String, String)],
    adapters: &[(String, String)],
    allowed: &[(&str, &str)],
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen_allowed = Vec::new();
    for (file, text) in toolkit {
        for name in public_functions(text) {
            let adapters_using: Vec<&str> = adapters
                .iter()
                .filter(|(folder, _)| folder != "mock")
                .filter(|(_, code)| uses_function(&code_of(code, false), &name))
                .map(|(folder, _)| folder.as_str())
                .collect();
            let by_the_toolkit = toolkit
                .iter()
                .filter(|(other, _)| other != file)
                .any(|(_, code)| uses_function(&code_of(code, false), &name));
            let enough = adapters_using.len() >= 2 || by_the_toolkit;
            match allowed.iter().find(|(allowed_name, _)| *allowed_name == name) {
                Some((_, next)) => {
                    seen_allowed.push(name.clone());
                    if enough {
                        problems.push(format!(
                            "toolkit/{file}: `{name}` is used by {} and no longer needs its exception ({next}): remove it from the list",
                            if by_the_toolkit { "the toolkit".to_string() } else { adapters_using.join(" and ") }
                        ));
                    }
                }
                None if !enough => problems.push(format!(
                    "toolkit/{file}: `{name}` is used by {}: a function that fewer than two versions use is in that version's folder (decision D12-12)",
                    if adapters_using.is_empty() { "no adapter".to_string() } else { adapters_using.join(" only") + " only" }
                )),
                None => {}
            }
        }
    }
    for (name, next) in allowed {
        if !seen_allowed.iter().any(|seen| seen == name) {
            problems.push(format!(
                "the exception for `{name}` ({next}) names a function that the toolkit does not have: remove it from the list"
            ));
        }
    }
    problems
}

/// The toolkit functions that one version alone uses **for now**, each with the second user that is expected. An entry leaves this list when that user arrives: the guard says so.
const ONE_USER_FOR_NOW: [(&str, &str); 0] = [];

#[test]
fn the_toolkit_holds_only_what_two_versions_use() {
    let read_all = |dir: &Path| -> Vec<(String, String)> {
        rust_files(dir)
            .into_iter()
            .map(|file| {
                let name = file
                    .strip_prefix(dir)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/");
                (name, fs::read_to_string(&file).expect("a readable source"))
            })
            .collect()
    };
    let toolkit: Vec<(String, String)> = read_all(&toolkit_dir())
        .into_iter()
        .filter(|(name, _)| name != "mod.rs")
        .collect();
    // each adapter folder as one body of code
    let adapters: Vec<(String, String)> = adapter_folders()
        .into_iter()
        .map(|folder| {
            let code = read_all(&adapters_dir().join(&folder))
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("\n");
            (folder, code)
        })
        .collect();
    // a guard that looked at nothing would pass for the wrong reason: the toolkit has eleven public functions, and V16 and V17 are adapters
    let functions: usize = toolkit.iter().map(|(_, t)| public_functions(t).len()).sum();
    assert!(
        functions >= 11,
        "the guard found only {functions} public functions"
    );
    assert!(adapters.iter().any(|(f, _)| f == "v16") && adapters.iter().any(|(f, _)| f == "v17"));
    let problems = sharing_problems(&toolkit, &adapters, &ONE_USER_FOR_NOW);
    assert!(
        problems.is_empty(),
        "the toolkit holds what fewer than two versions use:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_sharing_guard_rejects_a_toolkit_made_to_break_it() {
    let toolkit = |text: &str| vec![("values.rs".to_string(), text.to_string())];
    let adapters = |v16: &str, v17: &str| {
        vec![
            ("v16".to_string(), v16.to_string()),
            ("v17".to_string(), v17.to_string()),
            ("mock".to_string(), "let x = lonely(1);".to_string()),
        ]
    };
    let none: [(&str, &str); 0] = [];
    // used by two versions: shared
    assert!(sharing_problems(
        &toolkit("pub fn shared(x: u8) -> u8 { x }"),
        &adapters("let a = shared(1);", "use crate::toolkit::values::shared;"),
        &none
    )
    .is_empty());
    // used by one version alone, by no version, and by the mock alone (which is not a version): each is reported
    for (what, v16, v17) in [
        ("one version", "let a = lonely(1);", "let b = 2;"),
        ("no version", "let a = 1;", "let b = 2;"),
    ] {
        let found = sharing_problems(
            &toolkit("pub fn lonely(x: u8) -> u8 { x }"),
            &adapters(v16, v17),
            &none,
        );
        assert!(
            found.iter().any(|p| p.contains("`lonely`")),
            "the guard did not report a function used by {what}: {found:?}"
        );
    }
    // a function that is only defined in an adapter is not a use of it, and a comment or a longer name is not either
    let found = sharing_problems(
        &toolkit("pub fn lonely(x: u8) -> u8 { x }"),
        &adapters(
            "fn lonely() {}\n// lonely(2)\nlet y = lonely_two(3);",
            "let z = 1;",
        ),
        &none,
    );
    assert!(found.iter().any(|p| p.contains("`lonely`")), "{found:?}");
    // another file of the toolkit that uses it is plumbing of a shared part: enough
    let two_files = vec![
        (
            "values.rs".to_string(),
            "pub fn plumbing(x: u8) -> u8 { x }".to_string(),
        ),
        (
            "lasers.rs".to_string(),
            "fn body() { plumbing(1); }".to_string(),
        ),
    ];
    assert!(sharing_problems(&two_files, &adapters("let a = 1;", "let b = 2;"), &none).is_empty());
    // a method is not counted: its name is too common to mean anything
    assert!(sharing_problems(
        &toolkit("impl X {\n    pub fn new() -> X { X }\n}"),
        &adapters("let a = 1;", "let b = 2;"),
        &none
    )
    .is_empty());
    // an exception holds while one version uses the function, and says so when it is no longer needed or no longer true
    let one = adapters("let a = lonely(1);", "let b = 2;");
    let allowed = [("lonely", "V17 will")];
    assert!(
        sharing_problems(&toolkit("pub fn lonely(x: u8) -> u8 { x }"), &one, &allowed).is_empty()
    );
    let both = adapters("let a = lonely(1);", "let b = lonely(2);");
    assert!(sharing_problems(
        &toolkit("pub fn lonely(x: u8) -> u8 { x }"),
        &both,
        &allowed
    )
    .iter()
    .any(|p| p.contains("no longer needs its exception")));
    assert!(
        sharing_problems(&toolkit("pub fn other(x: u8) -> u8 { x }"), &both, &allowed)
            .iter()
            .any(|p| p.contains("does not have"))
    );
}
