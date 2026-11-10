//! `build-log-testkit`: **test support**, for use as a dev-dependency only.
//!
//! * **Layer:** none. It is not part of the library; it helps tests find their data.
//! * **May depend on:** no workspace crate (it offers paths and skip helpers only).
//! * **Who uses it:** every crate may list it under `[dev-dependencies]`. Nothing lists it under `[dependencies]`.
//!
//! # Three tiers of test data (IP-00 decision D00-9)
//!
//! 1. **Miniatures**, committed in `testdata/`: small redacted files with the structure of the real ones. [`miniature_path`] finds them from any crate,
//!    and `testdata/SHA256SUMS` lists the hash of each. These are the only files CI reads.
//! 2. **Large local fixtures**, in the git-ignored `fixtures/` folder (`Build Log Fixtures/`, `extracts/`): [`large_fixture`] returns `None` when a file
//!    is not there, and [`skip_unless!`] / [`large_fixture_or_skip!`] let a test say so and stop. The folder is `BUILD_LOG_FIXTURES_DIR`, default `<repo>/fixtures`.
//! 3. **The two untrimmed originals** (89 GB): **no Rust code reads them.** Only the Python fixture tools of IP-00 did, once.
//!
//! A Rust test cannot be marked "skipped", so a test that cannot run prints `SKIPPED: <reason>` to standard error and returns; run with
//! `-- --nocapture` to see the reasons.

use std::path::{Path, PathBuf};

pub mod scan;

/// The environment variable that names the folder holding the large local fixtures.
pub const FIXTURES_ENV: &str = "BUILD_LOG_FIXTURES_DIR";

/// A committed miniature may be at most this large (IP-00 decision D00-5: 12 MB).
pub const MINIATURE_FILE_LIMIT_BYTES: u64 = 12_000_000;
/// All the committed miniatures together may be at most this large (IP-00 decision D00-5: 50 MB, raised to 75 MB by the owner on 2026-10-20 when the seventh miniature was added).
pub const MINIATURE_TOTAL_LIMIT_BYTES: u64 = 75_000_000;

/// The names of the committed miniatures in `testdata/`. A new miniature is a new constant, and a line in `testdata/SHA256SUMS`.
pub mod miniature {
    /// V16, from the 15-layer trimmed copy of the 200-layer build.
    pub const V16: &str = "v16.h5";
    /// V17 of March 2026, from the 15-layer trimmed copy of the 2,110-layer real build.
    pub const V17_MARCH: &str = "v17_march.h5";
    /// V17 as the current tauri release writes it (the 2026-10-16 mock capture).
    pub const V17_MOCK: &str = "v17_mock.h5";
    /// V17 of 2026-10-15, a real machine build with an empty `Laser_2`.
    pub const V17_OCT15: &str = "v17_oct15.h5";
    /// V17 of 2026-10-19 with the OPC UA join output (`Global_Sensors`, `Build_Status`): one layer, both lasers recorded, no `Synchronous_*` samples.
    pub const V17_TRIGGERED_2026_10_19: &str = "v17_triggered_2026_10_19.h5";
    /// V17 of 2026-08-18 as tauri writes it, the fullest one: three layers with the camera images (`Pre_Exposure/raw`, `Post_Exposure/raw`) and the OPC UA events inside each layer.
    pub const V17_2026_08_18_IMAGES: &str = "v17_2026_08_18_images.h5";
    /// V18, which is provisional: its owner is still finalizing it.
    pub const V18: &str = "v18.h5";
    /// Layers 1964 to 1967 and 2006 to 2008 of the V17 original: a 39.9-hour pause and a laser that did not fire.
    pub const V17_SPECIAL_LAYERS: &str = "special_layers_v17.h5";
}

/// The repository root, found from where this crate lives (`<repo>/rust/testkit`), so it does not depend on the working directory of a test.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the test kit lives two folders below the repository root")
        .to_path_buf()
}

/// The folder of the committed miniatures: `<repo>/testdata`.
pub fn testdata_dir() -> PathBuf {
    repo_root().join("testdata")
}

/// The path of a committed miniature, from any crate: `miniature_path(miniature::V16)`. The file may not exist yet (it is a path, not a check).
pub fn miniature_path(name: &str) -> PathBuf {
    testdata_dir().join(name)
}

/// The folder of the large local fixtures, given the value of [`FIXTURES_ENV`] (if set and not empty) and the repository root.
pub fn fixtures_dir_from(env_value: Option<&str>, repo_root: &Path) -> PathBuf {
    match env_value {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => repo_root.join("fixtures"),
    }
}

/// The folder of the large local fixtures: `BUILD_LOG_FIXTURES_DIR`, or `<repo>/fixtures`.
pub fn fixtures_dir() -> PathBuf {
    fixtures_dir_from(std::env::var(FIXTURES_ENV).ok().as_deref(), &repo_root())
}

/// A large local fixture by its path inside the fixtures folder (`"Build Log Fixtures/V18_fixture.h5"`, `"extracts/v16_layer_1.h5"`), or `None`
/// when the folder or the file is not there. A test that needs one uses [`large_fixture_or_skip!`].
pub fn large_fixture(name: &str) -> Option<PathBuf> {
    large_fixture_in(&fixtures_dir(), name)
}

/// [`large_fixture`] in a given folder.
pub fn large_fixture_in(folder: &Path, name: &str) -> Option<PathBuf> {
    let path = folder.join(name);
    path.is_file().then_some(path)
}

/// The line a skipped test prints.
pub fn skip_message(reason: &str) -> String {
    format!("SKIPPED: {reason}")
}

/// Stop the test, with a printed reason, unless a condition holds: `skip_unless!(dir.is_dir(), "{} is not there", dir.display())`.
/// The test returns normally (a Rust test cannot be marked skipped); the reason is in the output with `-- --nocapture`.
/// Use it in a function that returns `()`.
#[macro_export]
macro_rules! skip_unless {
    ($condition:expr, $($reason:tt)+) => {
        if !$condition {
            eprintln!("{}", $crate::skip_message(&format!($($reason)+)));
            return;
        }
    };
}

/// The path of a large local fixture, or stop the test with a printed reason when it is not there. Use it in a function that returns `()`.
#[macro_export]
macro_rules! large_fixture_or_skip {
    ($name:expr) => {
        match $crate::large_fixture($name) {
            Some(path) => path,
            None => {
                eprintln!(
                    "{}",
                    $crate::skip_message(&format!(
                        "the large fixture {} is not in {} (set {} to the folder that holds it)",
                        $name,
                        $crate::fixtures_dir().display(),
                        $crate::FIXTURES_ENV
                    ))
                );
                return;
            }
        }
    };
}
