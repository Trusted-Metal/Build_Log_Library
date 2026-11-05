//! An adapter records a failed read as a **typed origin**, never as a sentence (IP-07 phase 7B, task T07.2, decision D07-7).
//!
//! The adapters said `Origin::Other("not read: ...")` where a laser, a channel, a series, a frame or a set of parameters could not be read, and the validation report and the availability
//! of an item (phases 7C and 7D) would have had to read that sentence to tell a fault of the file from a type the library does not read. They say `Origin::not_read(&error)` now, which
//! makes `Origin::NotRead { cause, reason }` from the error.
//!
//! * `no_free_text_not_read_origin_remains` reads every source file of the adapters and fails if the code of one makes an `Origin::Other` whose text begins `not read`.
//! * `the_guard_rejects_a_source_made_to_break_it` shows that the scan fails on the old spelling, on a spelling that only differs in its spaces, and on a text built in two steps, and
//!   passes the typed origin and an `Origin::Other` that says something else (a column shorter than its timestamps), and a comment that mentions the old text.

use std::fs;
use std::path::{Path, PathBuf};

use build_log_testkit::scan::code_of;

/// The places in `text` where code makes an `Origin::Other` whose text starts with `not read`, with the line.
fn free_text_not_read(text: &str) -> Vec<String> {
    let code = code_of(text, true);
    let mut found = Vec::new();
    for (at, line) in code.lines().enumerate() {
        // the call may be split over lines, so look at this line and the next two as one
        let joined: String = code.lines().skip(at).take(3).collect::<Vec<_>>().join(" ");
        let compact: String = joined.split_whitespace().collect::<Vec<_>>().join("");
        // the old spelling, with a format or with a plain text
        if line.contains("Origin::Other")
            && (compact.contains("Origin::Other(format!(\"notread")
                || compact.contains("Origin::Other(\"notread"))
        {
            found.push(format!("line {}: {}", at + 1, line.trim()));
        }
    }
    found
}

fn sources_in(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("the folder can be read").flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources_in(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_free_text_not_read_origin_remains() {
    let adapters = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("adapters");
    let mut files = Vec::new();
    sources_in(&adapters, &mut files);
    assert!(
        files.len() >= 10,
        "the adapters' sources were found: {files:?}"
    );
    let mut found = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path).expect("a source");
        for line in free_text_not_read(&text) {
            found.push(format!("{}: {line}", path.display()));
        }
    }
    assert!(
        found.is_empty(),
        "an adapter records a failed read as `Origin::not_read(&error)`, not as text:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_guard_rejects_a_source_made_to_break_it() {
    for (what, source) in [
        (
            "the old spelling",
            "origins.record(\"events\", Origin::Other(format!(\"not read: {error}\")));",
        ),
        (
            "the old spelling over several lines",
            "origins.record(\n    \"events\",\n    Origin::Other(format!(\n        \"not read: {error}\"\n    )),\n);",
        ),
        (
            "a plain text",
            "origins.record(\"laser\", Origin::Other(\"not read: the laser\".into()));",
        ),
    ] {
        assert_eq!(free_text_not_read(source).len(), 1, "{what}");
    }
    for (what, source) in [
        ("the typed origin", "origins.record(\"events\", Origin::not_read(&error));"),
        (
            "an origin that says something else",
            "part.origins.record(\"values\", Origin::Other(format!(\"the values column holds {held} entries\")));",
        ),
        (
            "a comment that mentions the old text",
            "// it used to be Origin::Other(format!(\"not read: {error}\"))\nlet x = 1;",
        ),
    ] {
        assert!(free_text_not_read(source).is_empty(), "{what}");
    }
}
