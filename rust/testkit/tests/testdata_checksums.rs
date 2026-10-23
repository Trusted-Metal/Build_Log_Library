//! The committed miniatures are held to `testdata/SHA256SUMS` (IP-01 task T01.6; IP-00 decisions D00-5, D00-9 and D00-10).
//!
//! `testdata_matches_checksums` fails if an `.h5` file in `testdata/` differs from its listed hash, is not listed, or has no sidecar; if a listed file is
//! missing; or if one file is over 12 MB or the files together are over 75 MB. It runs on every operating system in CI, so it is also the proof that a checkout
//! with `core.autocrlf=true` did not change a byte (`*.h5` is marked binary in `.gitattributes`). A miniature is committed and never rebuilt: changing one needs
//! the owner's approval and a new line in `SHA256SUMS`.
//!
//! The checking is in `verify`, a function over a folder, so the tests below can run it on folders made to be wrong.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use build_log_testkit::{
    skip_unless, testdata_dir, MINIATURE_FILE_LIMIT_BYTES, MINIATURE_TOTAL_LIMIT_BYTES,
};
use sha2::{Digest, Sha256};

const SUMS: &str = "SHA256SUMS";

fn sha256_hex(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// `<64 hex digits>, two spaces, <name>` per line, as `sha256sum` writes it. A trailing CR is tolerated (the file is kept LF by `.gitattributes`).
fn parse_sums(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut listed = BTreeMap::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let (hash, name) = line
            .split_once("  ")
            .ok_or_else(|| format!("line {}: expected `<sha256>  <name>`", index + 1))?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!(
                "line {}: {hash:?} is not a lower-case SHA-256",
                index + 1
            ));
        }
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(format!(
                "line {}: {name:?} is not a plain file name",
                index + 1
            ));
        }
        if listed.insert(name.to_string(), hash.to_string()).is_some() {
            return Err(format!("line {}: {name} is listed twice", index + 1));
        }
    }
    Ok(listed)
}

/// What is wrong with a `testdata/`-shaped folder; empty when it is as approved.
fn verify(dir: &Path, file_limit: u64, total_limit: u64) -> Vec<String> {
    let text = match fs::read_to_string(dir.join(SUMS)) {
        Ok(text) => text,
        Err(e) => return vec![format!("{SUMS} cannot be read: {e}")],
    };
    let listed = match parse_sums(&text) {
        Ok(listed) => listed,
        Err(e) => return vec![format!("{SUMS}: {e}")],
    };
    let mut problems = Vec::new();
    let mut present = BTreeSet::new();
    let mut total = 0_u64;
    for entry in fs::read_dir(dir).expect("a readable folder").flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".h5") {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        total += size;
        if size > file_limit {
            problems.push(format!(
                "{name} is {size} bytes, over the {file_limit} limit for one file"
            ));
        }
        if !dir
            .join(format!("{}.sidecar.json", name.trim_end_matches(".h5")))
            .is_file()
        {
            problems.push(format!(
                "{name} has no sidecar ({}.sidecar.json)",
                name.trim_end_matches(".h5")
            ));
        }
        match listed.get(&name) {
            None => problems.push(format!("{name} is not listed in {SUMS}")),
            Some(expected) => match sha256_hex(&entry.path()) {
                Ok(actual) if &actual == expected => {}
                Ok(actual) => problems.push(format!(
                    "{name} is not the approved file (SHA-256 {actual}, approved {expected})"
                )),
                Err(e) => problems.push(format!("{name} cannot be read: {e}")),
            },
        }
        present.insert(name);
    }
    problems.extend(
        listed
            .keys()
            .filter(|n| !present.contains(*n))
            .map(|n| format!("{n} is listed in {SUMS} but not in the folder")),
    );
    if total > total_limit {
        problems.push(format!(
            "the miniatures are {total} bytes together, over the {total_limit} limit"
        ));
    }
    problems
}

#[test]
fn testdata_matches_checksums() {
    let dir = testdata_dir();
    skip_unless!(
        dir.join(SUMS).is_file(),
        "{} is not there (the miniatures are committed by IP-00 phase 0L)",
        dir.join(SUMS).display()
    );
    let problems = verify(
        &dir,
        MINIATURE_FILE_LIMIT_BYTES,
        MINIATURE_TOTAL_LIMIT_BYTES,
    );
    assert!(
        problems.is_empty(),
        "testdata/ is not as approved:\n  - {}",
        problems.join("\n  - ")
    );
    // and it is not an empty check: the folder holds miniatures
    let listed = parse_sums(&fs::read_to_string(dir.join(SUMS)).expect("SHA256SUMS can be read"))
        .expect("SHA256SUMS parses");
    assert!(!listed.is_empty(), "{SUMS} lists nothing");
}

// ---------------------------------------------------------------- the checker is checked: a folder made to be wrong is refused

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A folder with two small "miniatures", their sidecars and a correct SHA256SUMS.
fn good_folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "build-log-checksums-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a folder");
    let mut sums = String::new();
    for (name, bytes) in [
        ("a.h5", b"first miniature".as_slice()),
        ("b.h5", b"second\r\nminiature, with line ends".as_slice()),
    ] {
        fs::write(dir.join(name), bytes).expect("a file");
        fs::write(
            dir.join(format!("{}.sidecar.json", name.trim_end_matches(".h5"))),
            b"{}",
        )
        .expect("a sidecar");
        sums.push_str(&format!(
            "{}  {name}\n",
            sha256_hex(&dir.join(name)).expect("a hash")
        ));
    }
    fs::write(dir.join(SUMS), sums).expect("the sums");
    dir
}

#[test]
fn a_correct_folder_has_no_problems() {
    let dir = good_folder();
    assert_eq!(verify(&dir, 1000, 1000), Vec::<String>::new());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn each_way_a_folder_can_be_wrong_is_found() {
    let find = |dir: &Path, expected: &str, file_limit: u64, total_limit: u64| {
        let problems = verify(dir, file_limit, total_limit);
        assert!(
            problems.iter().any(|p| p.contains(expected)),
            "expected a problem containing {expected:?}, got {problems:?}"
        );
    };

    let dir = good_folder();
    fs::write(dir.join("a.h5"), b"first miniature, altered").expect("a file");
    find(&dir, "a.h5 is not the approved file", 1000, 1000);
    let _ = fs::remove_dir_all(&dir);

    let dir = good_folder();
    fs::write(dir.join("c.h5"), b"not approved").expect("a file");
    fs::write(dir.join("c.sidecar.json"), b"{}").expect("a sidecar");
    find(&dir, "c.h5 is not listed in SHA256SUMS", 1000, 1000);
    let _ = fs::remove_dir_all(&dir);

    let dir = good_folder();
    fs::remove_file(dir.join("b.h5")).expect("remove");
    find(
        &dir,
        "b.h5 is listed in SHA256SUMS but not in the folder",
        1000,
        1000,
    );
    let _ = fs::remove_dir_all(&dir);

    let dir = good_folder();
    fs::remove_file(dir.join("a.sidecar.json")).expect("remove");
    find(&dir, "a.h5 has no sidecar", 1000, 1000);
    let _ = fs::remove_dir_all(&dir);

    let dir = good_folder();
    find(&dir, "over the 10 limit for one file", 10, 1000);
    find(&dir, "together, over the 20 limit", 1000, 20);
    let _ = fs::remove_dir_all(&dir);

    let dir = good_folder();
    fs::remove_file(dir.join(SUMS)).expect("remove");
    find(&dir, "SHA256SUMS cannot be read", 1000, 1000);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_sums_file_is_parsed_strictly_but_a_carriage_return_is_tolerated() {
    let hash = "a".repeat(64);
    assert_eq!(
        parse_sums(&format!("{hash}  a.h5\r\n{hash}  b.h5\n"))
            .expect("parses")
            .len(),
        2
    );
    for (text, expected) in [
        (format!("{hash} a.h5\n"), "expected `<sha256>  <name>`"),
        (
            format!("{}  a.h5\n", "A".repeat(64)),
            "not a lower-case SHA-256",
        ),
        (
            format!("{}  a.h5\n", "a".repeat(63)),
            "not a lower-case SHA-256",
        ),
        (format!("{hash}  dir/a.h5\n"), "not a plain file name"),
        (format!("{hash}  a.h5\n{hash}  a.h5\n"), "listed twice"),
    ] {
        let error = parse_sums(&text).expect_err("this text is not valid");
        assert!(
            error.contains(expected),
            "{error:?} does not contain {expected:?}"
        );
    }
}
