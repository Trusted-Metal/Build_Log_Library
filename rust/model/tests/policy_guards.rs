//! The policy guards of the Model crate (IP-02 task T02.16, phase 2A; requirements BL-CD-03, BL-CD-08, BL-CD-09; decision D02-2).
//!
//! Six rules keep the Common Data Model the same shape however many types are added to it, and every later task keeps them green:
//!
//! 1. **Every enum is `#[non_exhaustive]`, or says `// closed: <reason>`** (and not both).
//! 2. **Every public struct says who builds it**, with a line `// constructed-by: producer`, `library` or `caller-options`.
//! 3. **Producer structs are plain** (no `#[non_exhaustive]`, no `Default`), so a new field is a compile error that makes the producer decide about it;
//!    **library and caller-options structs are `#[non_exhaustive]`** and have `Default` or `new`.
//! 4. **Bulk data is never a field of a metadata struct**: no `Vec`, boxed or shared slice or reference to a slice of numbers. A type that is meant to
//!    hold bulk data says `// bulk-container: <reason>`.
//! 5. **An enum for an open domain says `// open-domain: <reason>` and has an `Other(…)` variant** that carries what it could not name.
//! 6. **No HDF5 path or group name** appears in the code of the Model: the Model is layout-neutral.
//!
//! The scanner reads the source of the crate as text. It needs no parser (the crate must stay free of dependencies) because the rules are about the
//! lines above a type, its derives, its fields and the names it uses. Each rule is also run on source made to break it, so a guard that stopped
//! looking would be noticed. The five files of the pre-cleanup types are skipped until phase 2T removes them.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

/// The modules of the Common Data Model (IP-02 Appendix A). Each is a file in `src/`, and each is scanned.
const CDM_MODULES: [&str; 14] = [
    "capabilities",
    "channel",
    "column",
    "error",
    "events",
    "extra",
    "identity",
    "json",
    "meta",
    "params",
    "provenance",
    "quality",
    "time",
    "units",
];

/// The pre-cleanup types: shaped like the V18 file, holding whole `Vec`s. They leave in phase 2T (task T02.23), and these guards do not apply to them.
/// **This list may only get shorter**: `every_source_file_is_a_cdm_module_or_a_legacy_one` fails for a file that is on neither list.
const LEGACY_MODULES: [&str; 5] = ["build", "by_layer", "corrections", "laser", "sensor"];

const NUMERIC: [&str; 14] = [
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64",
];

/// Names of the groups and datasets of the committed miniatures (read from them on 2026-10-20). None of them may appear in the Model. A new dialect
/// that brings new names adds them here.
const LAYOUT_NAMES: [&str; 33] = [
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
    "Source_ts",
    "Source_Ts",
    "Galvo_X",
    "Galvo_Y",
    "Galvo_Z",
    "Actual_X",
    "Commanded_X",
    "Start_ns",
    "Stop_ns",
];

/// A numbered or generic group (`Layer_12`, `Laser_N`, `Optical_Train_2`, `Layer_{n}`).
const LAYOUT_PREFIXES: [&str; 4] = ["Layer_", "Laser_", "Optical_Train_", "Laser_Calibration_"];

// ---------------------------------------------------------------- the scanner

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Struct,
    Enum,
}

#[derive(Debug)]
struct Item {
    kind: Kind,
    name: String,
    line: usize,
    /// The text inside each `#[...]` directly above the item.
    attributes: Vec<String>,
    /// Each plain `//` comment line directly above the item, without the slashes (documentation comments are not markers).
    markers: Vec<String>,
    /// Is the body `{ ... }` (true), or `( ... )` or nothing?
    braced: bool,
    /// The text between the outer braces or parentheses, without comments.
    body: String,
}

#[derive(Debug)]
struct Source {
    file: String,
    /// The file without comments, line breaks kept.
    code: String,
    items: Vec<Item>,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The text without `//` and `/* */` comments. String and character literals are kept whole; line breaks stay, so line numbers still match.
fn strip_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_string, mut block) = (0, false, 0usize);
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if block > 0 {
            if c == '*' && next == Some('/') {
                block -= 1;
                i += 2;
            } else if c == '/' && next == Some('*') {
                block += 1;
                i += 2;
            } else {
                if c == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = next {
                    out.push(n);
                    i += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match (c, next) {
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                block = 1;
                i += 2;
            }
            ('"', _) => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            ('\'', _) => {
                // a character literal ('x' or '\n') is copied whole, so that '"' does not open a string; anything else is a lifetime
                let literal_len = if next == Some('\\') {
                    (i + 2..chars.len().min(i + 8))
                        .find(|&j| chars[j] == '\'')
                        .map(|j| j + 1 - i)
                } else if chars.get(i + 2) == Some(&'\'') {
                    Some(3)
                } else {
                    None
                };
                let len = literal_len.unwrap_or(1);
                out.extend(&chars[i..i + len]);
                i += len;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// How many `[` are still open in `text`.
fn bracket_balance(text: &str) -> i32 {
    text.chars().fold(0, |depth, c| match c {
        '[' => depth + 1,
        ']' => depth - 1,
        _ => depth,
    })
}

/// `pub struct Name` or `pub enum Name` at the start of a line (`pub(crate)` and private items are not public API).
fn item_header(line: &str) -> Option<(Kind, String)> {
    let (kind, rest) = if let Some(rest) = line.strip_prefix("pub struct ") {
        (Kind::Struct, rest)
    } else {
        let rest = line.strip_prefix("pub enum ")?;
        (Kind::Enum, rest)
    };
    let name: String = rest.chars().take_while(|c| is_ident(*c)).collect();
    (!name.is_empty()).then_some((kind, name))
}

/// The balanced block that starts at `text[open]` (a `{`, `(` or `[`): the text inside, and the index just after the closing bracket.
fn balanced(text: &str, open: usize) -> Option<(String, usize)> {
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' | '(' | '[' => {
                if depth == 0 {
                    start = open + i + 1;
                }
                depth += 1;
            }
            '}' | ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[start..open + i].to_string(), open + i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// The body of the item declared at the start of `from` (comment-free code from the item's line on): braces, parentheses or nothing.
fn item_body(from: &str, kind: Kind, name: &str) -> (bool, String) {
    let keyword = if kind == Kind::Struct {
        "struct"
    } else {
        "enum"
    };
    let declaration = format!("{keyword} {name}");
    let at = from.find(&declaration).map_or(0, |p| p + declaration.len());
    let rest = &from[at..];
    // skip the generics
    let mut skip = 0;
    if rest.trim_start().starts_with('<') {
        let open = rest.find('<').unwrap_or(0);
        let mut depth = 0;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        skip = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let rest = &rest[skip..];
    match rest.find(['{', '(', ';']) {
        Some(p) if rest[p..].starts_with('{') => {
            (true, balanced(rest, p).map(|b| b.0).unwrap_or_default())
        }
        Some(p) if rest[p..].starts_with('(') => {
            (false, balanced(rest, p).map(|b| b.0).unwrap_or_default())
        }
        _ => (false, String::new()),
    }
}

fn scan(file: &str, text: &str) -> Source {
    let code = strip_comments(text);
    let mut offsets = Vec::new();
    let mut at = 0;
    for line in code.split_inclusive('\n') {
        offsets.push(at);
        at += line.len();
    }
    let raw: Vec<&str> = text.lines().collect();
    let mut items = Vec::new();
    let mut attributes: Vec<String> = Vec::new();
    let mut markers: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let line = raw[i].trim();
        if line.is_empty() {
            attributes.clear();
            markers.clear();
        } else if line.starts_with("///") || line.starts_with("//!") {
            // documentation belongs to the item or the module, and may sit between a marker and the item
        } else if let Some(rest) = line.strip_prefix("//") {
            markers.push(rest.trim().to_string());
        } else if line.starts_with("#[") || line.starts_with("#![") {
            let mut joined = line.to_string();
            while bracket_balance(&joined) > 0 && i + 1 < raw.len() {
                i += 1;
                joined.push(' ');
                joined.push_str(raw[i].trim());
            }
            let inner = joined
                .trim_start_matches("#!")
                .trim_start_matches('#')
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string();
            attributes.push(inner);
        } else {
            if let Some((kind, name)) = item_header(line) {
                let (braced, body) =
                    item_body(&code[offsets.get(i).copied().unwrap_or(0)..], kind, &name);
                items.push(Item {
                    kind,
                    name,
                    line: i + 1,
                    attributes: std::mem::take(&mut attributes),
                    markers: std::mem::take(&mut markers),
                    braced,
                    body,
                });
            }
            attributes.clear();
            markers.clear();
        }
        i += 1;
    }
    Source {
        file: file.to_string(),
        code,
        items,
    }
}

/// `text` split at the commas that are not inside brackets, braces, parentheses or angle brackets.
fn split_top_level(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut previous = ' ';
    for c in text.chars() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' if previous != '-' && previous != '=' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            parts.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(c);
        }
        previous = c;
    }
    parts.push(current.trim().to_string());
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

fn strip_attributes(mut part: &str) -> &str {
    while part.starts_with("#[") {
        match balanced(part, 1) {
            Some((_, end)) => part = part[end..].trim_start(),
            None => break,
        }
    }
    part
}

fn strip_visibility(part: &str) -> &str {
    let part = part.trim_start();
    if let Some(rest) = part.strip_prefix("pub(") {
        return rest.split_once(')').map_or(part, |(_, r)| r.trim_start());
    }
    part.strip_prefix("pub ").map_or(part, str::trim_start)
}

/// What each field (or variant) of an item holds: `(name, type or payload)`. A tuple struct's fields are named by their position.
fn members(item: &Item) -> Vec<(String, String)> {
    split_top_level(&item.body)
        .iter()
        .enumerate()
        .map(|(position, part)| {
            let part = strip_visibility(strip_attributes(part));
            match item.kind {
                Kind::Enum => {
                    let name: String = part.chars().take_while(|c| is_ident(*c)).collect();
                    (name.clone(), part[name.len()..].trim().to_string())
                }
                Kind::Struct if item.braced => {
                    let colon = part
                        .char_indices()
                        .find(|&(i, c)| {
                            c == ':' && !part[i + 1..].starts_with(':') && !part[..i].ends_with(':')
                        })
                        .map(|(i, _)| i);
                    match colon {
                        Some(i) => (
                            part[..i].trim().to_string(),
                            part[i + 1..].trim().to_string(),
                        ),
                        None => (part.to_string(), String::new()),
                    }
                }
                Kind::Struct => (position.to_string(), part.to_string()),
            }
        })
        .collect()
}

fn has_attribute(item: &Item, name: &str) -> bool {
    item.attributes
        .iter()
        .any(|a| a == name || a.starts_with(&format!("{name}(")))
}

/// The value of each marker `// key: value` above the item.
fn marker_values(item: &Item, key: &str) -> Vec<String> {
    item.markers
        .iter()
        .filter_map(|m| m.strip_prefix(key)?.strip_prefix(':'))
        .map(|v| v.trim().to_string())
        .collect()
}

/// The traits in the `#[derive(...)]` attributes, by last path segment.
fn derives(item: &Item) -> BTreeSet<String> {
    item.attributes
        .iter()
        .filter_map(|a| a.strip_prefix("derive("))
        .flat_map(|d| d.trim_end_matches(')').split(','))
        .map(|t| t.rsplit("::").next().unwrap_or("").trim().to_string())
        .collect()
}

/// The positions in `text` where `word` stands as a whole word.
fn word_positions(text: &str, word: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(p) = text[from..].find(word) {
        let at = from + p;
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        if !before.is_some_and(is_ident) && !after.is_some_and(is_ident) {
            found.push(at);
        }
        from = at + word.len();
    }
    found
}

/// Does the file implement `Default` for the type (`impl Default for Name`, with or without generics)?
fn implements_default(src: &Source, name: &str) -> bool {
    word_positions(&src.code, name)
        .into_iter()
        .any(|at| src.code[..at].trim_end().ends_with("Default for"))
}

/// Does an inherent `impl Name { ... }` have a public `new`?
fn has_public_new(src: &Source, name: &str) -> bool {
    for at in word_positions(&src.code, "impl") {
        let rest = &src.code[at + 4..];
        let Some(open) = rest.find('{') else { continue };
        let header = rest[..open].trim();
        let header = match header.strip_prefix('<') {
            // skip the generics of the impl
            Some(after) => after.split_once('>').map_or("", |(_, r)| r.trim()),
            None => header,
        };
        let names_the_type = header
            .strip_prefix(name)
            .is_some_and(|r| !r.chars().next().is_some_and(is_ident));
        if !names_the_type || header.contains(" for ") {
            continue;
        }
        if let Some((block, _)) = balanced(rest, open) {
            if block.contains("pub fn new") || block.contains("pub const fn new") {
                return true;
            }
        }
    }
    false
}

fn describe(src: &Source, item: &Item, what: &str) -> String {
    format!("{}:{}: `{}` {}", src.file, item.line, item.name, what)
}

// ---------------------------------------------------------------- the guards

/// Rule 1: every enum is `#[non_exhaustive]` or says `// closed: <reason>`.
fn enum_violations(src: &Source) -> Vec<String> {
    let mut found = Vec::new();
    for item in src.items.iter().filter(|i| i.kind == Kind::Enum) {
        let closed = marker_values(item, "closed");
        let non_exhaustive = has_attribute(item, "non_exhaustive");
        if closed.len() > 1 {
            found.push(describe(src, item, "has more than one `// closed:` marker"));
        } else if closed.iter().any(|reason| reason.is_empty()) {
            found.push(describe(src, item, "says `// closed:` without a reason"));
        } else if non_exhaustive && !closed.is_empty() {
            found.push(describe(
                src,
                item,
                "is marked closed and is `#[non_exhaustive]`: it cannot be both",
            ));
        } else if !non_exhaustive && closed.is_empty() {
            found.push(describe(
                src,
                item,
                "is an enum that is neither `#[non_exhaustive]` nor marked `// closed: <reason>`",
            ));
        }
    }
    found
}

const CONSTRUCTORS: [&str; 3] = ["producer", "library", "caller-options"];

/// The value of the one `// constructed-by:` marker of a struct, if it has exactly one valid one.
fn constructed_by(item: &Item) -> Option<String> {
    let values = marker_values(item, "constructed-by");
    (values.len() == 1 && CONSTRUCTORS.contains(&values[0].as_str())).then(|| values[0].clone())
}

/// Rule 2: every public struct says who builds it.
fn marker_violations(src: &Source) -> Vec<String> {
    let mut found = Vec::new();
    for item in src.items.iter().filter(|i| i.kind == Kind::Struct) {
        let values = marker_values(item, "constructed-by");
        match values.len() {
            0 => found.push(describe(src, item, "is a public struct without a `// constructed-by: producer|library|caller-options` marker")),
            1 if !CONSTRUCTORS.contains(&values[0].as_str()) => found.push(describe(
                src,
                item,
                &format!("says `constructed-by: {}`, which is not producer, library or caller-options", values[0]),
            )),
            1 => {}
            _ => found.push(describe(src, item, "has more than one `constructed-by` marker")),
        }
    }
    found
}

/// Rule 3: producer structs are plain; library and caller-options structs are `#[non_exhaustive]` with `Default` or `new`.
fn construction_violations(src: &Source) -> Vec<String> {
    let mut found = Vec::new();
    for item in src.items.iter().filter(|i| i.kind == Kind::Struct) {
        let Some(who) = constructed_by(item) else {
            continue;
        };
        let non_exhaustive = has_attribute(item, "non_exhaustive");
        let has_default = derives(item).contains("Default") || implements_default(src, &item.name);
        if who == "producer" {
            if non_exhaustive {
                found.push(describe(src, item, "is a producer struct and must not be `#[non_exhaustive]` (a new field has to break the producer's build)"));
            }
            if has_default {
                found.push(describe(src, item, "is a producer struct and must not have `Default` (a forgotten field would be written as a default)"));
            }
        } else {
            if !non_exhaustive {
                found.push(describe(
                    src,
                    item,
                    &format!("is a {who} struct and must be `#[non_exhaustive]`"),
                ));
            }
            if !has_default && !has_public_new(src, &item.name) {
                found.push(describe(
                    src,
                    item,
                    &format!("is a {who} struct and needs `Default` or a public `new`"),
                ));
            }
        }
    }
    found
}

/// The first piece of `ty` that is bulk data: a vector, boxed or shared slice, or reference to a slice of numbers.
fn bulk_fragment(ty: &str) -> Option<String> {
    let t: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
    let starts_numeric = |s: &str| {
        NUMERIC.iter().any(|n| {
            s.strip_prefix(n)
                .is_some_and(|rest| !rest.chars().next().is_some_and(is_ident))
        })
    };
    let mut from = 0;
    while let Some(p) = t[from..].find("Vec<") {
        let inner = &t[from + p + 4..];
        let tuple_of_numbers = inner.starts_with('(')
            && balanced(inner, 0).is_some_and(|(elements, _)| {
                split_top_level(&elements).iter().any(|e| starts_numeric(e))
            });
        if starts_numeric(inner)
            || inner.starts_with("Vec<")
            || inner.starts_with('[')
            || tuple_of_numbers
        {
            return Some(format!("Vec<{}", inner.split('>').next().unwrap_or("")));
        }
        from += p + 4;
    }
    let mut from = 0;
    while let Some(p) = t[from..].find('[') {
        let at = from + p;
        if let Some((inner, end)) = balanced(&t, at) {
            if !inner.contains(';') && starts_numeric(&inner) {
                return Some(format!("[{inner}]"));
            }
            from = end;
        } else {
            break;
        }
    }
    None
}

/// Rule 4: no bulk data in a field, unless the type says `// bulk-container: <reason>`.
fn bulk_violations(src: &Source) -> Vec<String> {
    let mut found = Vec::new();
    for item in &src.items {
        let exempt = marker_values(item, "bulk-container");
        if exempt.iter().any(|reason| reason.is_empty()) {
            found.push(describe(
                src,
                item,
                "says `// bulk-container:` without a reason",
            ));
        }
        if !exempt.is_empty() {
            continue;
        }
        for (member, ty) in members(item) {
            if let Some(bulk) = bulk_fragment(&ty) {
                found.push(describe(
                    src,
                    item,
                    &format!("holds bulk data in `{member}` ({bulk}); bulk data is reached through a column accessor, never held by a metadata type"),
                ));
            }
        }
    }
    found
}

/// Rule 5: an enum marked `// open-domain:` has an `Other(…)` variant that carries something.
fn open_enum_violations(src: &Source) -> Vec<String> {
    let mut found = Vec::new();
    for item in src.items.iter().filter(|i| i.kind == Kind::Enum) {
        let open = marker_values(item, "open-domain");
        if open.is_empty() {
            continue;
        }
        if open.iter().any(|reason| reason.is_empty()) {
            found.push(describe(
                src,
                item,
                "says `// open-domain:` without a reason",
            ));
        }
        if !marker_values(item, "closed").is_empty() {
            found.push(describe(src, item, "is marked both open-domain and closed"));
        }
        let other = members(item).into_iter().find(|(name, _)| name == "Other");
        match other {
            None => found.push(describe(
                src,
                item,
                "is an open-domain enum without an `Other(…)` variant",
            )),
            Some((_, payload)) if payload.is_empty() => found.push(describe(
                src,
                item,
                "has an `Other` variant that carries nothing: it must hold what it could not name",
            )),
            Some(_) => {}
        }
    }
    found
}

/// The contents of the string literals on a line of comment-free code.
fn string_literals(line: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut current: Option<String> = None;
    let mut escaped = false;
    for c in line.chars() {
        match (&mut current, c) {
            (Some(text), '\\') if !escaped => {
                escaped = true;
                text.push(c);
            }
            (Some(text), '"') if !escaped => {
                literals.push(std::mem::take(text));
                current = None;
            }
            (Some(text), _) => {
                escaped = false;
                text.push(c);
            }
            (None, '"') => current = Some(String::new()),
            (None, _) => {}
        }
    }
    literals
}

/// Rule 6: no HDF5 path or group name anywhere in the code of the Model.
fn layout_violations(src: &Source) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in src.code.lines().enumerate() {
        let at = n + 1;
        for name in LAYOUT_NAMES {
            if !word_positions(line, name).is_empty() {
                found.push(format!("{}:{at}: the layout name `{name}` (a group or dataset of the files) is in the Model", src.file));
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
                found.push(format!(
                    "{}:{at}: a numbered or generic group name (`{prefix}…`) is in the Model",
                    src.file
                ));
            }
        }
        for literal in string_literals(line) {
            let body = literal.strip_prefix('/').unwrap_or("");
            if body.starts_with(|c: char| c.is_ascii_alphabetic()) && body.contains('/') {
                found.push(format!(
                    "{}:{at}: the string \"{literal}\" looks like an HDF5 path",
                    src.file
                ));
            }
        }
    }
    found
}

// ---------------------------------------------------------------- the real crate

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn real_sources() -> Vec<Source> {
    CDM_MODULES
        .iter()
        .map(|module| {
            let path = src_dir().join(format!("{module}.rs"));
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} cannot be read: {e}", path.display()));
            scan(&format!("{module}.rs"), &text)
        })
        .collect()
}

fn assert_clean(rule: &str, found: Vec<String>) {
    assert!(
        found.is_empty(),
        "{rule}: {} violation(s) in the Model:\n  {}",
        found.len(),
        found.join("\n  ")
    );
}

fn on_real_crate(guard: fn(&Source) -> Vec<String>) -> Vec<String> {
    real_sources().iter().flat_map(guard).collect()
}

#[test]
fn enums_are_non_exhaustive_or_marked_closed() {
    assert_clean(
        "every enum is #[non_exhaustive] or marked closed",
        on_real_crate(enum_violations),
    );
}

#[test]
fn every_public_struct_declares_who_constructs_it() {
    assert_clean(
        "every public struct has a constructed-by marker",
        on_real_crate(marker_violations),
    );
}

#[test]
fn producer_structs_are_plain_and_library_structs_are_non_exhaustive() {
    assert_clean(
        "construction follows the marker",
        on_real_crate(construction_violations),
    );
}

#[test]
fn bulk_data_is_never_a_metadata_field() {
    assert_clean(
        "no bulk data in a metadata type",
        on_real_crate(bulk_violations),
    );
}

#[test]
fn open_enums_have_an_other_arm() {
    assert_clean(
        "open-domain enums have Other(...)",
        on_real_crate(open_enum_violations),
    );
}

#[test]
fn no_layout_names_in_public_types() {
    assert_clean(
        "no HDF5 path or group name in the Model",
        on_real_crate(layout_violations),
    );
}

// ---------------------------------------------------------------- the module layout (phase 2A)

#[test]
fn every_module_of_appendix_a_exists_and_states_its_rule() {
    let lib = fs::read_to_string(src_dir().join("lib.rs")).expect("lib.rs can be read");
    for module in CDM_MODULES {
        let path = src_dir().join(format!("{module}.rs"));
        let text = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("{module}.rs is missing: Appendix A lists the module `{module}`")
        });
        for required in ["//! * **Layer:**", "//! * **Constructed by:**"] {
            assert!(
                text.lines().any(|l| l.trim().starts_with(required)),
                "{module}.rs does not say `{required}` in its module documentation"
            );
        }
        assert!(
            lib.lines().any(|l| l.trim() == format!("mod {module};")),
            "lib.rs does not declare `mod {module};`"
        );
    }
}

#[test]
fn every_source_file_is_a_cdm_module_or_a_legacy_one() {
    let mut present: BTreeSet<String> = fs::read_dir(src_dir())
        .expect("src can be listed")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".rs").map(str::to_string))
        .collect();
    assert!(present.remove("lib"), "src/lib.rs exists");
    let known: BTreeSet<String> = CDM_MODULES
        .iter()
        .chain(LEGACY_MODULES.iter())
        .map(|s| s.to_string())
        .collect();
    let unknown: Vec<_> = present.difference(&known).collect();
    assert!(
        unknown.is_empty(),
        "these source files are neither a module of Appendix A nor on the legacy list, so no guard would read them: {unknown:?}"
    );
    let gone: Vec<_> = known.difference(&present).collect();
    assert!(
        gone.is_empty(),
        "these modules are listed but their files are gone (take legacy files off LEGACY_MODULES when phase 2T removes them): {gone:?}"
    );
}

// ---------------------------------------------------------------- the guards are checked: source made to break each rule

const COMPLIANT: &str = r#"
//! A module.

// constructed-by: producer
#[derive(Debug, Clone, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
    pub origin: Option<String>,
    pub pair: [f64; 2],
}

// constructed-by: library
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct TimeAxis {
    pub window: Option<(i64, i64)>,
    pub segments: Vec<String>,
}

// constructed-by: caller-options
#[non_exhaustive]
pub struct ColumnRequest {
    pub first: u64,
}

impl ColumnRequest {
    pub fn new(first: u64) -> Self {
        Self { first }
    }
}

// constructed-by: library
// bulk-container: the typed, shared result of a column read
#[non_exhaustive]
pub struct Samples {
    pub values: std::sync::Arc<[f32]>,
}

impl Default for Samples {
    fn default() -> Self {
        Self { values: std::sync::Arc::from(Vec::new()) }
    }
}

// closed: a laser is on or off and nothing else exists
pub enum Switch {
    On,
    Off,
}

// open-domain: units that files spell in ways nobody has listed yet
/// The unit of a quantity.
#[non_exhaustive]
pub enum Unit {
    Volt,
    Other(String),
}

// constructed-by: producer
pub struct LayerId(pub u32);
"#;

fn violations_in(text: &str) -> Vec<String> {
    let src = scan("sample.rs", text);
    [
        enum_violations(&src),
        marker_violations(&src),
        construction_violations(&src),
        bulk_violations(&src),
        open_enum_violations(&src),
        layout_violations(&src),
    ]
    .concat()
}

#[test]
fn a_compliant_sample_passes_every_guard_and_is_really_scanned() {
    let src = scan("sample.rs", COMPLIANT);
    let names: Vec<_> = src.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Quantity",
            "TimeAxis",
            "ColumnRequest",
            "Samples",
            "Switch",
            "Unit",
            "LayerId"
        ],
        "the scanner finds every public struct and enum"
    );
    assert_eq!(violations_in(COMPLIANT), Vec::<String>::new());
}

/// Source with one violation, the rule it breaks, and a piece of the message that names the offender.
const BROKEN: [(&str, &str, &str); 17] = [
    ("an enum that is neither non-exhaustive nor closed", "pub enum Mode { A, B }", "`Mode` is an enum that is neither"),
    ("an enum marked closed without a reason", "// closed:\npub enum Mode { A }", "without a reason"),
    ("an enum marked closed and non-exhaustive", "// closed: fixed\n#[non_exhaustive]\npub enum Mode { A }", "cannot be both"),
    ("a struct without a marker", "pub struct Bare { pub a: u8 }", "without a `// constructed-by"),
    ("a struct with an unknown constructor", "// constructed-by: somebody\npub struct Odd { pub a: u8 }", "which is not producer, library or caller-options"),
    ("a struct with two markers", "// constructed-by: producer\n// constructed-by: library\npub struct Twice { pub a: u8 }", "more than one `constructed-by`"),
    ("a producer struct that is non-exhaustive", "// constructed-by: producer\n#[non_exhaustive]\npub struct P { pub a: u8 }", "must not be `#[non_exhaustive]`"),
    ("a producer struct that derives Default", "// constructed-by: producer\n#[derive(Debug, Default)]\npub struct P { pub a: u8 }", "must not have `Default`"),
    ("a producer struct that implements Default", "// constructed-by: producer\npub struct P { pub a: u8 }\nimpl Default for P { fn default() -> Self { P { a: 0 } } }", "must not have `Default`"),
    ("a library struct that is not non-exhaustive", "// constructed-by: library\n#[derive(Default)]\npub struct L { pub a: u8 }", "must be `#[non_exhaustive]`"),
    ("a library struct without Default or new", "// constructed-by: library\n#[non_exhaustive]\npub struct L { pub a: u8 }", "needs `Default` or a public `new`"),
    ("a vector of floats in a metadata struct", "// constructed-by: producer\npub struct M { pub samples: Vec<f64> }", "holds bulk data in `samples`"),
    ("shared bytes, or a vector in a variant", "// closed: test\npub enum E { Data(std::sync::Arc<[u8]>), Grid { rows: Vec<Vec<f32>> } }", "holds bulk data in `Data`"),
    ("an open-domain enum without Other", "// open-domain: kinds files add\n#[non_exhaustive]\npub enum Kind { A, B }", "without an `Other(…)` variant"),
    ("an Other variant that carries nothing", "// open-domain: kinds files add\n#[non_exhaustive]\npub enum Kind { A, Other }", "carries nothing"),
    ("a group name in the code", "// constructed-by: producer\npub struct S { pub where_from: &'static str }\npub const GROUP: &str = \"Monitoring_Data\";", "the layout name `Monitoring_Data`"),
    ("a path and a numbered group in the code", "pub const PATH: &str = \"/Built_Part/Layer_1\";\npub struct Layer_12;", "looks like an HDF5 path"),
];

#[test]
fn each_policy_guard_rejects_a_synthetic_violation() {
    for (what, source, expected) in BROKEN {
        let found = violations_in(source);
        assert!(
            found.iter().any(|v| v.contains(expected)),
            "{what}: expected a violation saying {expected:?}, got {found:?}"
        );
    }
}

#[test]
fn a_comment_or_a_string_does_not_hide_or_invent_a_violation() {
    // a layout name in a comment is documentation, not code
    assert!(layout_violations(&scan(
        "c.rs",
        "// the group Monitoring_Data holds it\n/// and Built_Part too\npub struct X;"
    ))
    .is_empty());
    // `//` inside a string is not a comment, and a character literal can hold a quote
    let stripped = strip_comments("let a = \"http://x\"; let q = '\"'; let b = 1; // note");
    assert!(
        stripped.contains("http://x")
            && stripped.contains("let b = 1;")
            && !stripped.contains("note")
    );
    // a marker separated from its item by a blank line is not the item's
    let src = scan(
        "m.rs",
        "// constructed-by: producer\n\npub struct Apart { pub a: u8 }",
    );
    assert!(marker_violations(&src)
        .iter()
        .any(|v| v.contains("without a `// constructed-by")));
    // an attribute over several lines is read whole
    let src = scan(
        "a.rs",
        "// constructed-by: library\n#[derive(\n    Debug,\n    Default,\n)]\n#[non_exhaustive]\npub struct Wide { pub a: u8 }",
    );
    assert!(
        construction_violations(&src).is_empty(),
        "{:?}",
        construction_violations(&src)
    );
}

#[test]
fn bulk_types_are_recognised_and_small_descriptors_are_not() {
    for bulk in [
        "Vec<f64>",
        "Option<Vec<u8>>",
        "Vec<Vec<f32>>",
        "Vec<[f64; 3]>",
        "Vec<(f64, f64)>",
        "std::sync::Arc<[f32]>",
        "Box<[i16]>",
        "&'a [u64]",
        "Vec <  i32 >",
    ] {
        assert!(bulk_fragment(bulk).is_some(), "{bulk} is bulk data");
    }
    for small in [
        "f64",
        "[f64; 3]",
        "[u8; 8]",
        "Vec<String>",
        "Vec<Unit>",
        "Option<(i64, i64)>",
        "std::collections::BTreeMap<String, String>",
        "String",
    ] {
        assert!(bulk_fragment(small).is_none(), "{small} is not bulk data");
    }
}
