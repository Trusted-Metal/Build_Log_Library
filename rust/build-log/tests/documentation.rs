//! The relative links in the documentation resolve (IP-01 task T01.15, phase 1J).
//!
//! A relative link in a Markdown file must point at a file or folder that is part of the repository (tracked by git, or new and not ignored), and a
//! `#fragment` must name a heading of the file it points at. A document that is kept only on one machine is not part of the repository, so a link to
//! it is a broken link for everyone else: that is the mistake this test is for.
//!
//! The check works on a list of files and their texts, so `a_broken_link_of_each_kind_is_found` can feed it a repository made to break each rule.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("two folders below the repository root")
        .to_path_buf()
}

/// What the check looks at: every file of the repository (as `/`-separated paths from its root) and the text of each Markdown file.
struct Docs {
    files: BTreeSet<String>,
    markdown: BTreeMap<String, String>,
}

impl Docs {
    fn is_file_or_folder(&self, path: &str) -> bool {
        path.is_empty() // the root of the repository
            || self.files.contains(path)
            || self
                .files
                .range(format!("{path}/")..)
                .next()
                .is_some_and(|f| f.starts_with(&format!("{path}/")))
    }
}

/// The files git would commit: tracked ones, and new ones that are not ignored. `None` when this is not a git checkout (a source export).
fn repository_files(root: &Path) -> Option<BTreeSet<String>> {
    if !root.join(".git").exists() {
        return None;
    }
    // `safe.directory=*`: a CI container checks the files out as another user than the one running the tests, and git refuses such a folder otherwise
    let output = Command::new("git")
        .args(["-c", "safe.directory=*"])
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .expect("git can be run in a git checkout");
    assert!(
        output.status.success(),
        "git ls-files failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(
        String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn load_docs(root: &Path, files: BTreeSet<String>) -> Docs {
    let markdown = files
        .iter()
        .filter(|f| f.ends_with(".md"))
        .filter(|f| root.join(f).is_file()) // listed but deleted from the working tree
        .map(|f| {
            let text = std::fs::read_to_string(root.join(f))
                .unwrap_or_else(|e| panic!("{f} cannot be read: {e}"));
            (f.clone(), text)
        })
        .collect();
    Docs { files, markdown }
}

/// The lines of a Markdown text that are not inside a fenced code block.
fn prose_lines(text: &str) -> Vec<&str> {
    let mut fence: Option<char> = None;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let marker = if trimmed.starts_with("```") {
            Some('`')
        } else if trimmed.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        match (fence, marker) {
            (None, Some(m)) => fence = Some(m),
            (Some(open), Some(m)) if open == m => fence = None,
            (None, None) => lines.push(line),
            _ => {}
        }
    }
    lines
}

/// The targets of the inline links (`[text](target)`) and link definitions (`[id]: target`) of a line, leaving out code spans.
fn link_targets(line: &str) -> Vec<String> {
    let mut targets = Vec::new();

    // a link definition: `[id]: target "title"`
    let trimmed = line.trim_start();
    if trimmed.starts_with('[') {
        if let Some(at) = trimmed.find("]:") {
            if let Some(first) = trimmed[at + 2..].split_whitespace().next() {
                targets.push(first.trim_matches(['<', '>']).to_string());
            }
        }
    }

    // inline links, outside code spans
    let mut outside = String::new();
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
        } else if !in_code {
            outside.push(c);
        }
    }
    let mut rest = outside.as_str();
    while let Some(at) = rest.find("](") {
        let after = &rest[at + 2..];
        let mut depth = 0;
        let mut end = after.len();
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => {
                    end = i;
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        let inside = after[..end].trim();
        let target = if let Some(stripped) = inside.strip_prefix('<') {
            stripped.split('>').next().unwrap_or("")
        } else {
            inside.split_whitespace().next().unwrap_or("")
        };
        if !target.is_empty() {
            targets.push(target.to_string());
        }
        rest = &after[end.min(after.len())..];
    }
    targets
}

/// The anchors GitHub gives the headings of a Markdown text: lower case, letters, digits, `-` and `_` kept, spaces turned into `-`, and a repeated
/// heading numbered `-1`, `-2`.
fn heading_anchors(text: &str) -> BTreeSet<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut anchors = BTreeSet::new();
    for line in prose_lines(text) {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if !(1..=6).contains(&hashes) || !line[hashes..].starts_with(' ') {
            continue;
        }
        let mut heading = line[hashes..]
            .trim()
            .trim_end_matches('#')
            .trim()
            .to_string();
        // a link in a heading counts as its text only
        while let Some(at) = heading.find("](") {
            if let Some(close) = heading[at..].find(')') {
                heading.replace_range(at..at + close + 1, "]");
            } else {
                break;
            }
        }
        let slug: String = heading
            .to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                '-' | '_' => Some(c),
                c if c.is_alphanumeric() => Some(c),
                _ => None,
            })
            .collect();
        let count = seen.entry(slug.clone()).or_insert(0);
        anchors.insert(if *count == 0 {
            slug.clone()
        } else {
            format!("{slug}-{count}")
        });
        *count += 1;
    }
    anchors
}

/// `from_dir` joined with `relative`, with `.` and `..` resolved. `None` if it climbs out of the repository.
fn resolve(from_dir: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = if relative.starts_with('/') {
        Vec::new() // a path from the root of the repository
    } else {
        from_dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

fn is_external(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:") || target.starts_with("tel:")
}

/// Every relative link that does not resolve, as a sentence naming the file, the link and what is wrong.
fn broken_links(docs: &Docs) -> Vec<String> {
    let anchors: BTreeMap<&String, BTreeSet<String>> = docs
        .markdown
        .iter()
        .map(|(path, text)| (path, heading_anchors(text)))
        .collect();
    let mut problems = Vec::new();
    for (file, text) in &docs.markdown {
        let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
        for line in prose_lines(text) {
            for target in link_targets(line) {
                if is_external(&target) {
                    continue;
                }
                let (path_part, fragment) = match target.split_once('#') {
                    Some((p, f)) => (p, Some(f)),
                    None => (target.as_str(), None),
                };
                let resolved = if path_part.is_empty() {
                    file.clone()
                } else {
                    match resolve(dir, path_part) {
                        Some(r) => r,
                        None => {
                            problems.push(format!("{file}: [{target}] leaves the repository"));
                            continue;
                        }
                    }
                };
                if !docs.is_file_or_folder(&resolved) {
                    problems.push(format!(
                        "{file}: [{target}] points at {resolved}, which is not in the repository (missing, ignored, or kept only on one machine)"
                    ));
                    continue;
                }
                if let Some(fragment) = fragment.filter(|f| !f.is_empty()) {
                    let is_line_anchor = fragment.starts_with('L')
                        && fragment[1..]
                            .chars()
                            .all(|c| c.is_ascii_digit() || c == '-' || c == 'L');
                    if let Some(known) = anchors.get(&resolved) {
                        if !known.contains(fragment) {
                            problems.push(format!(
                                "{file}: [{target}] names the heading #{fragment}, which {resolved} does not have"
                            ));
                        }
                    } else if !is_line_anchor && resolved.ends_with(".md") {
                        problems.push(format!("{file}: [{target}] names a heading of {resolved}, which cannot be read"));
                    }
                }
            }
        }
    }
    problems
}

fn docs_of(files: &[&str], markdown: &[(&str, &str)]) -> Docs {
    Docs {
        files: files.iter().map(|f| f.to_string()).collect(),
        markdown: markdown
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect(),
    }
}

#[test]
fn documentation_links_resolve() {
    let root = repo_root();
    let Some(files) = repository_files(&root) else {
        eprintln!(
            "{}",
            build_log_testkit::skip_message(
                "this is not a git checkout, so there is no list of the repository's files"
            )
        );
        return;
    };
    let docs = load_docs(&root, files);
    assert!(
        docs.markdown.contains_key("README.md"),
        "the README is part of the repository"
    );
    let problems = broken_links(&docs);
    assert!(
        problems.is_empty(),
        "{} broken link(s) in the documentation:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

#[test]
fn a_broken_link_of_each_kind_is_found() {
    let files = [
        "README.md",
        "docs/guide.md",
        "docs/notes.md",
        "src/lib.rs",
        "rust/model/Cargo.toml",
    ];
    let docs = docs_of(
        &files,
        &[
            ("README.md", "# Title\n\n## Build and test\n"),
            (
                "docs/guide.md",
                "[ok file](../README.md) [ok folder](../rust/model) [ok heading](../README.md#build-and-test) [ok same file](#second-part)\n\
                 [web](https://example.com/x) [mail](mailto:a@b.c) [line](../src/lib.rs#L12)\n\
                 `[in a code span](missing.md)`\n\
                 ```text\n[in a fence](missing.md)\n```\n\
                 # Second part\n",
            ),
            (
                "docs/notes.md",
                "[missing file](nowhere.md)\n\
                 [local only](../docs/plans/plan.md)\n\
                 [missing heading](../README.md#no-such-heading)\n\
                 [missing same-file heading](#nothing-here)\n\
                 [climbs out](../../../outside.md)\n\
                 [missing folder](../rust/ghost)\n",
            ),
        ],
    );
    let problems = broken_links(&docs);
    let in_guide: Vec<_> = problems
        .iter()
        .filter(|p| p.starts_with("docs/guide.md"))
        .collect();
    assert!(
        in_guide.is_empty(),
        "good links, links in code and links to the web are not problems: {in_guide:?}"
    );
    let notes: Vec<_> = problems
        .iter()
        .filter(|p| p.starts_with("docs/notes.md"))
        .collect();
    assert_eq!(notes.len(), 6, "{notes:?}");
    for expected in [
        "[nowhere.md] points at docs/nowhere.md",
        "[../docs/plans/plan.md] points at docs/plans/plan.md",
        "[../README.md#no-such-heading] names the heading #no-such-heading",
        "[#nothing-here] names the heading #nothing-here",
        "[../../../outside.md] leaves the repository",
        "[../rust/ghost] points at rust/ghost",
    ] {
        assert!(
            notes.iter().any(|p| p.contains(expected)),
            "no problem says {expected:?}: {notes:?}"
        );
    }
}

#[test]
fn heading_anchors_follow_githubs_rules() {
    let anchors = heading_anchors(
        "# Build and test\n## What `rust/model` holds (today)\n## Same\n## Same\n```text\n# not a heading\n```\n### CI: the `Test` check — required\n",
    );
    for expected in [
        "build-and-test",
        "what-rustmodel-holds-today",
        "same",
        "same-1",
        "ci-the-test-check--required",
    ] {
        assert!(anchors.contains(expected), "{expected} in {anchors:?}");
    }
    assert!(!anchors.contains("not-a-heading"));
}
