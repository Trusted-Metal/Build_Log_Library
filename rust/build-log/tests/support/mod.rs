//! The workspace as the architecture guards see it, and the guards themselves (IP-01 tasks T01.3 and T01.5, decision D01-4).
//!
//! A guard is a pure function over a [`Workspace`]: a small model of the packages, their dependencies, the resolved dependency graph and
//! the directories that matter. [`load_workspace`] builds it from `cargo metadata`; the tests that feed a guard a violation build one by hand.
//! So every guard is tested twice: it passes on the real workspace, and it names the offender in a workspace made to break its rule.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The public entry point, the one crate nothing inside the workspace may depend on.
pub const FACADE: &str = "build-log";
pub const MODEL: &str = "build-log-model";
pub const IO: &str = "build-log-io";
pub const PROCESSING: &str = "build-log-processing";
pub const TESTKIT: &str = "build-log-testkit";
pub const CONFORMANCE: &str = "build-log-conformance";

/// The five crates every checkout has, and where each lives (decisions D01-1 and D01-2).
pub const LAYERS: [(&str, &str); 6] = [
    (MODEL, "rust/model"),
    (IO, "rust/io"),
    (PROCESSING, "rust/processing"),
    (FACADE, "rust/build-log"),
    (TESTKIT, "rust/testkit"),
    (CONFORMANCE, "rust/conformance"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Normal,
    Dev,
    Build,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Normal => "normal",
            Kind::Dev => "dev",
            Kind::Build => "build",
        }
    }

    fn from_word(word: &str) -> Result<Kind, String> {
        match word {
            "normal" => Ok(Kind::Normal),
            "dev" => Ok(Kind::Dev),
            "build" => Ok(Kind::Build),
            other => Err(format!("unknown dependency kind {other:?}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub kind: Kind,
    pub optional: bool,
    /// The path or the source (a git URL, a registry) the dependency is taken from; empty when cargo says nothing.
    pub origin: String,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    /// The package's folder relative to the repository root, with forward slashes: `rust/model`.
    pub dir: String,
    pub publishable: bool,
    pub lib_name: Option<String>,
    /// The `rust-version` the package declares, if any.
    pub rust_version: Option<String>,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub members: Vec<Package>,
    /// Package name to the names of the packages it depends on (every kind), over the whole resolved graph.
    pub graph: BTreeMap<String, BTreeSet<String>>,
    /// The package names in `Cargo.lock`.
    pub lock_packages: BTreeSet<String>,
    /// The `rust-version` each package of the resolved graph declares (packages that declare none are absent).
    pub rust_versions: BTreeMap<String, String>,
    /// Each folder directly under `rust/`, and whether it has a `Cargo.toml`.
    pub rust_dirs: Vec<(String, bool)>,
    pub schema_top_level: bool,
    /// Folders named `schema` anywhere under `rust/`.
    pub schema_dirs_under_rust: Vec<String>,
    /// Text of the files that name packages outside Cargo (`package.json`, Python requirements): `(relative path, text)`.
    pub text_files: Vec<(String, String)>,
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the facade lives two folders below the repository root")
        .to_path_buf()
}

// ---------------------------------------------------------------- reading the real workspace

pub fn load_workspace() -> Result<Workspace, String> {
    let root = repo_root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("could not run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cargo metadata is not JSON: {e}"))?;
    workspace_from_metadata(&metadata, &root)
}

fn relative_dir(manifest_path: &str, root: &Path) -> String {
    let manifest = Path::new(manifest_path);
    let dir = manifest.parent().unwrap_or(manifest);
    dir.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| dir.to_string_lossy().replace('\\', "/"))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata has no string `{key}`"))
}

fn array_field<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("cargo metadata has no array `{key}`"))
}

pub fn workspace_from_metadata(metadata: &Value, root: &Path) -> Result<Workspace, String> {
    let packages = array_field(metadata, "packages")?;
    let mut name_of_id = BTreeMap::new();
    for package in packages {
        name_of_id.insert(
            str_field(package, "id")?.to_string(),
            str_field(package, "name")?.to_string(),
        );
    }
    let member_ids: BTreeSet<&str> = array_field(metadata, "workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut ws = Workspace::default();
    for package in packages {
        if let Some(version) = package.get("rust_version").and_then(Value::as_str) {
            ws.rust_versions
                .insert(str_field(package, "name")?.to_string(), version.to_string());
        }
    }
    for package in packages {
        if !member_ids.contains(str_field(package, "id")?) {
            continue;
        }
        let mut dependencies = Vec::new();
        for dep in array_field(package, "dependencies")? {
            let kind = match dep.get("kind").and_then(Value::as_str) {
                Some(word) => Kind::from_word(word)?,
                None => Kind::Normal,
            };
            let origin = dep
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| dep.get("source").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            dependencies.push(Dependency {
                name: str_field(dep, "name")?.to_string(),
                kind,
                optional: dep
                    .get("optional")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                origin,
            });
        }
        let lib_name = package
            .get("targets")
            .and_then(Value::as_array)
            .and_then(|targets| {
                targets.iter().find(|t| {
                    t.get("kind")
                        .and_then(Value::as_array)
                        .is_some_and(|k| k.iter().any(|x| x == "lib"))
                })
            })
            .and_then(|t| t.get("name").and_then(Value::as_str))
            .map(str::to_string);
        // `publish` is null when any registry is allowed and an empty array for `publish = false`
        let publishable = !package
            .get("publish")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty);
        ws.members.push(Package {
            name: str_field(package, "name")?.to_string(),
            dir: relative_dir(str_field(package, "manifest_path")?, root),
            publishable,
            lib_name,
            rust_version: package
                .get("rust_version")
                .and_then(Value::as_str)
                .map(str::to_string),
            dependencies,
        });
    }
    if let Some(nodes) = metadata
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .and_then(Value::as_array)
    {
        for node in nodes {
            let name = name_of_id
                .get(str_field(node, "id")?)
                .cloned()
                .unwrap_or_default();
            let entry = ws.graph.entry(name).or_default();
            for dep in array_field(node, "deps")? {
                if let Some(target) = name_of_id.get(str_field(dep, "pkg")?) {
                    entry.insert(target.clone());
                }
            }
        }
    }
    if let Ok(lock) = fs::read_to_string(root.join("Cargo.lock")) {
        ws.lock_packages = lock
            .lines()
            .filter_map(|l| l.strip_prefix("name = \""))
            .filter_map(|l| l.strip_suffix('"'))
            .map(str::to_string)
            .collect();
    }
    if let Ok(entries) = fs::read_dir(root.join("rust")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                ws.rust_dirs.push((
                    entry.file_name().to_string_lossy().to_string(),
                    entry.path().join("Cargo.toml").is_file(),
                ));
            }
        }
        ws.rust_dirs.sort();
    }
    ws.schema_top_level = root.join("schema").is_dir();
    find_dirs_named(
        &root.join("rust"),
        "schema",
        root,
        &mut ws.schema_dirs_under_rust,
    );
    for candidate in ["package.json", "tools/fixtures/requirements.txt"] {
        if let Ok(text) = fs::read_to_string(root.join(candidate)) {
            ws.text_files.push((candidate.to_string(), text));
        }
    }
    Ok(ws)
}

fn find_dirs_named(dir: &Path, wanted: &str, root: &Path, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || entry.file_name() == "target" {
            continue;
        }
        if entry.file_name() == wanted {
            found.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
        find_dirs_named(&path, wanted, root, found);
    }
}

// ---------------------------------------------------------------- the allowed-dependency data

#[derive(Debug, Clone)]
pub struct Rule {
    pub from: String,
    pub to: String,
    pub kinds: Vec<Kind>,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct Allowed {
    pub rules: Vec<Rule>,
}

impl Allowed {
    pub fn rule(&self, from: &str, to: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.from == from && r.to == to)
    }
}

/// The matrix of Appendix C, read from data (`tests/data/allowed_dependencies.json`) so that IP-02 adds its row by editing a file.
pub fn allowed_from_json(text: &str) -> Result<Allowed, String> {
    let doc: Value = serde_json::from_str(text)
        .map_err(|e| format!("the allowed-dependency data is not JSON: {e}"))?;
    let mut rules = Vec::new();
    for rule in array_field(&doc, "allowed")? {
        let kinds = array_field(rule, "kinds")?
            .iter()
            .filter_map(Value::as_str)
            .map(Kind::from_word)
            .collect::<Result<Vec<_>, _>>()?;
        rules.push(Rule {
            from: str_field(rule, "from")?.to_string(),
            to: str_field(rule, "to")?.to_string(),
            kinds,
            optional: rule
                .get("optional")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(Allowed { rules })
}

pub fn allowed_dependencies() -> Allowed {
    allowed_from_json(include_str!("../data/allowed_dependencies.json"))
        .expect("tests/data/allowed_dependencies.json is valid")
}

// ---------------------------------------------------------------- the guards: each returns what it found, empty when the rule holds

fn names(kinds: &[Kind]) -> String {
    kinds
        .iter()
        .map(|k| k.word())
        .collect::<Vec<_>>()
        .join(" or ")
}

/// One-way layering: an edge between two workspace crates exists only if the matrix allows it, as that kind, and optional where it says so.
pub fn layering_violations(ws: &Workspace, allowed: &Allowed) -> Vec<String> {
    let members: BTreeSet<&str> = ws.members.iter().map(|p| p.name.as_str()).collect();
    let mut found = Vec::new();
    for package in &ws.members {
        for dep in package
            .dependencies
            .iter()
            .filter(|d| members.contains(d.name.as_str()))
        {
            let edge = format!("{} -> {} ({})", package.name, dep.name, dep.kind.word());
            if dep.name == FACADE {
                found.push(format!(
                    "{edge}: nothing inside the workspace may depend on the facade `{FACADE}`"
                ));
                continue;
            }
            match allowed.rule(&package.name, &dep.name) {
                None => found.push(format!(
                    "{edge}: this dependency is not allowed (IP-01 Appendix C)"
                )),
                Some(rule) if !rule.kinds.contains(&dep.kind) => found.push(format!(
                    "{edge}: allowed only as a {} dependency",
                    names(&rule.kinds)
                )),
                Some(rule) if rule.optional && !dep.optional => {
                    found.push(format!("{edge}: must be optional, behind a feature"))
                }
                Some(_) => {}
            }
        }
    }
    found
}

/// The shortest chain from `start` to any package whose name satisfies `target`, through the resolved graph.
fn chain_to(ws: &Workspace, start: &str, target: impl Fn(&str) -> bool) -> Vec<Vec<String>> {
    let mut parent: BTreeMap<String, String> = BTreeMap::new();
    let mut queue = VecDeque::from([start.to_string()]);
    let mut seen = BTreeSet::from([start.to_string()]);
    let mut chains = Vec::new();
    while let Some(current) = queue.pop_front() {
        for next in ws.graph.get(&current).into_iter().flatten() {
            if seen.insert(next.clone()) {
                parent.insert(next.clone(), current.clone());
                if target(next) {
                    let mut chain = vec![next.clone()];
                    while let Some(p) = parent.get(chain.last().expect("a chain is not empty")) {
                        chain.push(p.clone());
                    }
                    chain.reverse();
                    chains.push(chain);
                }
                queue.push_back(next.clone());
            }
        }
    }
    chains
}

/// The Model has no HDF5 in its dependencies, however far down (it must build for wasm32).
pub fn model_hdf5_violations(ws: &Workspace) -> Vec<String> {
    chain_to(ws, MODEL, |name| name.starts_with("hdf5"))
        .iter()
        .map(|c| format!("{MODEL} reaches an HDF5 package: {}", c.join(" -> ")))
        .collect()
}

/// Processing does not depend on IO, directly or through another crate.
pub fn processing_io_violations(ws: &Workspace) -> Vec<String> {
    chain_to(ws, PROCESSING, |name| name == IO)
        .iter()
        .map(|c| format!("{PROCESSING} reaches the IO layer: {}", c.join(" -> ")))
        .collect()
}

fn is_mcf_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("machine-config") || lower.starts_with("machine_config")
}

/// The library never loads or depends on MCF: no package of that name in the graph, the lock file, the manifests or the tooling files.
pub fn mcf_violations(ws: &Workspace) -> Vec<String> {
    let mut found = Vec::new();
    let mut graph_names: BTreeSet<&str> = ws.graph.keys().map(String::as_str).collect();
    graph_names.extend(ws.graph.values().flatten().map(String::as_str));
    graph_names.extend(ws.members.iter().map(|p| p.name.as_str()));
    for name in graph_names.into_iter().filter(|n| is_mcf_name(n)) {
        found.push(format!(
            "the dependency graph contains the MCF package `{name}`"
        ));
    }
    for name in ws.lock_packages.iter().filter(|n| is_mcf_name(n)) {
        found.push(format!("Cargo.lock contains the MCF package `{name}`"));
    }
    for package in &ws.members {
        for dep in &package.dependencies {
            if is_mcf_name(&dep.name)
                || dep.origin.to_lowercase().contains("machine_config_library")
            {
                found.push(format!(
                    "{} depends on `{}` from `{}`: that is MCF",
                    package.name, dep.name, dep.origin
                ));
            }
        }
    }
    for (path, text) in &ws.text_files {
        let lower = text.to_lowercase();
        if ["machine-config", "machine_config"]
            .iter()
            .any(|word| lower.contains(word))
        {
            found.push(format!("{path} names an MCF package or folder"));
        }
    }
    found
}

/// `schema/` is a top-level folder, and no folder named `schema` is under `rust/`: the specification belongs to no language.
pub fn spec_location_violations(ws: &Workspace) -> Vec<String> {
    let mut found = Vec::new();
    if !ws.schema_top_level {
        found.push("schema/ does not exist as a top-level folder".to_string());
    }
    found.extend(ws.schema_dirs_under_rust.iter().map(|d| {
        format!("{d} is a `schema` folder under rust/: the specification belongs to no language")
    }));
    found
}

// ---------------------------------------------------------------- the structure of the workspace (phase 1B)

/// The four layers and the test kit exist, where decision D01-1 puts them, with the names of D01-2, unpublishable (D01-3).
pub fn layer_violations(ws: &Workspace) -> Vec<String> {
    let mut found = Vec::new();
    for (name, dir) in LAYERS {
        match ws.members.iter().find(|p| p.name == name) {
            None => found.push(format!(
                "the workspace has no package `{name}` (expected in {dir})"
            )),
            Some(p) => {
                if p.dir != dir {
                    found.push(format!("package `{name}` is in {} (expected {dir})", p.dir));
                }
                if p.publishable {
                    found.push(format!(
                        "package `{name}` is publishable: it must say `publish = false` (D01-3)"
                    ));
                }
            }
        }
    }
    if let Some(facade) = ws.members.iter().find(|p| p.name == FACADE) {
        if facade.lib_name.as_deref() != Some("build_log") {
            found.push(format!(
                "the facade's library is named {:?}, not `build_log`",
                facade.lib_name
            ));
        }
    }
    found
}

/// Every folder under `rust/` is a crate and a member of the workspace, and every member is in `rust/`.
pub fn crate_directory_violations(ws: &Workspace) -> Vec<String> {
    let mut found = Vec::new();
    for (dir, has_manifest) in &ws.rust_dirs {
        let path = format!("rust/{dir}");
        if !has_manifest {
            found.push(format!(
                "{path} has no Cargo.toml: a folder under rust/ is a crate"
            ));
        } else if !ws.members.iter().any(|p| p.dir == path) {
            found.push(format!("{path} is not a member of the workspace"));
        }
    }
    for p in &ws.members {
        if !p.dir.starts_with("rust/") {
            found.push(format!(
                "member `{}` is in {}, outside rust/",
                p.name, p.dir
            ));
        }
    }
    found
}

/// `1.85.1` as numbers; a missing part counts as 0 (`1.85` is `1.85.0`).
pub fn parse_rust_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split('.').map(|p| p.parse::<u32>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let micro = parts.next().unwrap_or(Ok(0)).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, micro))
}

/// The minimum supported Rust version a document records: the number after the words "Minimum supported Rust version".
pub fn recorded_msrv(document: &str) -> Option<String> {
    let after = &document[document.find("Minimum supported Rust version")?
        + "Minimum supported Rust version".len()..];
    let start = after.find(|c: char| c.is_ascii_digit())?;
    let number: String = after[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    Some(number.trim_end_matches('.').to_string())
}

/// The minimum supported Rust version is measured, recorded, and set: every crate declares exactly the recorded version, and no package in the
/// resolved dependency graph declares a newer one (so the workspace can be built with the oldest compiler we claim).
pub fn msrv_violations(ws: &Workspace, recorded: &str) -> Vec<String> {
    let Some(wanted) = parse_rust_version(recorded) else {
        return vec![format!(
            "the recorded minimum Rust version {recorded:?} is not a version number"
        )];
    };
    let mut found = Vec::new();
    for package in &ws.members {
        match package.rust_version.as_deref() {
            None => found.push(format!(
                "package `{}` declares no rust-version (set `rust-version.workspace = true`)",
                package.name
            )),
            Some(declared) if parse_rust_version(declared) != Some(wanted) => {
                found.push(format!("package `{}` declares rust-version {declared}, but {recorded} is the recorded minimum", package.name));
            }
            Some(_) => {}
        }
    }
    for (name, declared) in &ws.rust_versions {
        if parse_rust_version(declared).is_some_and(|v| v > wanted) {
            found.push(format!(
                "`{name}` requires Rust {declared}, newer than the recorded minimum {recorded}"
            ));
        }
    }
    found
}

pub fn assert_clean(what: &str, violations: Vec<String>) {
    assert!(
        violations.is_empty(),
        "{what}:\n  - {}",
        violations.join("\n  - ")
    );
}
