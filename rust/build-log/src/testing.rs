//! Support for a consumer's own tests (IP-04 phase 4F, task T04.12).
//!
//! The mock version is a version of the library that exists so that a consumer can be tried **before any real adapter reads a real file**: it has two
//! attributes and a marker on a mount, and a key from 9000 to 9999 (the range that is reserved for versions of this kind). It is never in the production
//! registry; a test makes a registry that holds it ([`mock_registry`]), writes a file of it ([`write_mock_file`], [`write_mock_group`]) and opens that
//! file through [`crate::open_with`] or [`crate::open_at_with`]. A second mock version stands for a version that the consumer has never seen, which is how a
//! consumer is shown to need no change for one.

use std::path::Path;

use build_log_io::adapters::mock;
use build_log_io::binding::{File, Group};
use build_log_io::hdf5::from_binding;
use build_log_model::Error;

use crate::Registry;

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
