//! Support for a consumer's own tests (IP-04 phase 4F, task T04.12).
//!
//! The mock version is a version of the library that exists so that a consumer can be tried **before any real adapter reads a real file**: it has two
//! attributes and a marker on a mount, and a key from 9000 to 9999 (the range that is reserved for versions of this kind). It is never in the production
//! registry; a test makes a registry that holds it ([`mock_registry`]), writes a file of it ([`write_mock_file`], [`write_mock_group`]) and opens that
//! file through [`crate::open_with`] or [`crate::open_at_with`]. A second mock version stands for a version that the consumer has never seen, which is how a
//! consumer is shown to need no change for one.

use std::path::{Path, PathBuf};

use build_log_io::adapters::mock;
use build_log_io::binding::{File, Group};
use build_log_io::hdf5::from_binding;
use build_log_model::Error;

use crate::Registry;

/// The names of the library's committed miniatures (IP-26 phase 26L): small real files of each version and flavour, for a consumer's own tests. A consumer that opens them
/// asks [`miniature_path`] for where each is.
pub mod miniature {
    /// V16, from a 15-layer trimmed copy of a 200-layer build.
    pub const V16: &str = "v16.h5";
    /// V17 of March 2026, from a 15-layer trimmed copy of a 2,110-layer real build.
    pub const V17_MARCH: &str = "v17_march.h5";
    /// V17 as the current tauri release writes it (the 2026-10-16 mock capture).
    pub const V17_MOCK: &str = "v17_mock.h5";
    /// V17 of 2026-10-15, a real machine build with an empty `Laser_2`.
    pub const V17_OCT15: &str = "v17_oct15.h5";
    /// V17 of 2026-10-19 with the OPC UA join output: one layer, both lasers recorded, no `Synchronous_*` samples.
    pub const V17_TRIGGERED_2026_10_19: &str = "v17_triggered_2026_10_19.h5";
    /// V17 of 2026-08-18, the fullest one: three layers with the camera images and the OPC UA events inside each layer.
    pub const V17_2026_08_18_IMAGES: &str = "v17_2026_08_18_images.h5";
    /// Layers 1964 to 1967 and 2006 to 2008 of the 2,110-layer V17 original: gaps in the layer ids, a long pause and a laser that did not fire.
    pub const V17_SPECIAL_LAYERS: &str = "special_layers_v17.h5";
    /// V18, which is provisional: its structure is still being decided, so the library does not open it yet.
    pub const V18: &str = "v18.h5";
    /// Every miniature, in a stable order. **The library opens every one except [`V18`].**
    pub const ALL: [&str; 8] = [
        V16,
        V17_MARCH,
        V17_MOCK,
        V17_OCT15,
        V17_TRIGGERED_2026_10_19,
        V17_2026_08_18_IMAGES,
        V17_SPECIAL_LAYERS,
        V18,
    ];
}

/// The folder of the library's committed miniatures, found from where the facade's source lives (`<repository>/rust/build-log`), so it does not depend on the working
/// directory of a test. **It exists only in a checkout of the library's repository**, which is what a path or a git dependency gives; a package that is distributed without
/// `testdata/` gives a path that does not exist.
pub fn miniature_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the facade lives two folders below the repository root")
        .join("testdata")
}

/// The path of a committed miniature: `miniature_path(miniature::V16)`. It is a path and not a check: the file may not be there (see [`miniature_dir`]).
pub fn miniature_path(name: &str) -> PathBuf {
    miniature_dir().join(name)
}

/// A registry that holds the mock versions `keys` (each from 9000 to 9999, such as `"9001"`), and nothing else.
pub fn mock_registry(keys: &[&str]) -> Result<Registry, Error> {
    let mut registry = Registry::new();
    for key in keys {
        registry.register(mock::adapter_set_for(key)?)?;
    }
    Ok(registry)
}

/// The name of the mount group of the mock version `key` in a standalone file.
pub fn mount_for(key: &str) -> String {
    mock::mount_for(key)
}

/// Writes a mock build of the version `key` into `mount`, a group anywhere in a file: the marker, the build's id and a number of layers.
pub fn write_mock_group(
    mount: &Group,
    key: &str,
    build_id: &str,
    layers: i32,
) -> Result<(), Error> {
    mock::write_mock_file_for(mount, key, build_id, layers)
}

/// Makes a standalone file at `path` that holds a mock build of the version `key`. An existing file is replaced.
pub fn write_mock_file(
    path: impl AsRef<Path>,
    key: &str,
    build_id: &str,
    layers: i32,
) -> Result<(), Error> {
    let file = File::create(path.as_ref()).map_err(|e| from_binding("creating a mock file", e))?;
    let mount = file
        .create_group(&mount_for(key))
        .map_err(|e| from_binding("creating the mount of a mock file", e))?;
    write_mock_group(&mount, key, build_id, layers)
}
