//! The mock version, key `9001` (IP-04 phase 4D, task T04.6, decision D04-10; requirement BL-VA-08).
//!
//! A version that exists so that the dispatch can be proved **before any real adapter exists**, and that stays: **it is registered only by injection** (a test
//! passes [`adapter_set`] to a registry it builds; the production registry never holds it, and a test says so), and 9000 to 9999 is reserved for such
//! versions (decision D00-4). Its layout is two attributes and a marker on a mount group (Appendix D of the plan):
//!
//! | Attribute | Stored as | Meaning |
//! |---|---|---|
//! | the marker | `int32`, a scalar, 9001 | the version |
//! | `Build_ID` | a variable-length UTF-8 string | the build's id |
//! | `Layer_Count` | `int32`, a scalar | the layers 1 to the count, all declared and present |
//!
//! It is **reader-only**: its writer and patcher are declared unsupported. It covers dispatch and not yet the change categories of the migration manifests
//! (BL-VA-07), which arrive with IP-14 and IP-18. Its names are its own and are meant to be copied by the first real adapter, not shared with it; like every
//! adapter it reads **every path relative to its mount**, so the same data opened standalone or nested at any depth gives the same build (decision D04-9).

use build_log_model::{
    BuildIdentity, BuildMeta, BuildSource, Capabilities, Error, ErrorKind, InMemorySource, LayerId,
    LayerMeta, Violation,
};
use hdf5::types::VarLenUnicode;
use hdf5::Group;

use crate::adapters::{AdapterSet, MarkerPlace};
use crate::hdf5::{from_binding, read_attribute, Value};

/// The version the mock stands for.
pub const KEY: &str = "9001";
/// The adapter's name in a version report.
pub const NAME: &str = "mock";
/// The name of the mount group that a standalone mock file holds.
pub const MOUNT: &str = "Mock_Mount";
const MARKER: &str = "File_Version";
const BUILD_ID: &str = "Build_ID";
const LAYER_COUNT: &str = "Layer_Count";

/// The mount name of the mock version `key`: `Mock_Mount` for 9001 and `Mock_Mount_<key>` for any other, so that two mock versions can sit in one file.
pub fn mount_for(key: &str) -> String {
    if key == KEY {
        MOUNT.to_string()
    } else {
        format!("{MOUNT}_{key}")
    }
}

/// The registration unit of the mock version `key`, any key from 9000 to 9999 (the range reserved for test-only versions, decision D00-4). Every mock version
/// reads the same layout, and differs in its key and its mount; a test uses a second one to show that a new version needs no change on a consumer's side.
pub fn adapter_set_for(key: &str) -> Result<AdapterSet, Error> {
    let in_range = key.parse::<u32>().is_ok_and(|n| (9000..=9999).contains(&n));
    if !in_range {
        return Err(Error::new(ErrorKind::ValidationError {
            violations: vec![Violation::new(format!(
                "{key:?} is not a mock version: they are 9000 to 9999"
            ))],
        }));
    }
    let name = if key == KEY {
        NAME.to_string()
    } else {
        format!("{NAME}-{key}")
    };
    AdapterSet::builder(key, name, MarkerPlace::new(mount_for(key), MARKER))
        .capabilities(Capabilities::new())
        .reader(read)
        .without_writer("the mock version has no writer")
        .without_patcher("the mock version has no patcher")
        .build()
}

/// The registration unit of the mock version 9001. Inject it into a registry; it is never in the production one.
pub fn adapter_set() -> AdapterSet {
    adapter_set_for(KEY).expect("9001 is a mock version")
}

/// The reader: the build of the mount, as a source of metadata. Every attribute is read from the mount that was handed in.
fn read(mount: &Group) -> Result<Box<dyn BuildSource>, Error> {
    let corrupt = |detail: String| Error::new(ErrorKind::Corrupt { detail });
    let build_id = match read_attribute(mount, BUILD_ID)?.value {
        Value::Text(mut v) if v.len() == 1 => v.remove(0),
        other => {
            return Err(corrupt(format!(
                "the mock's build id is not one text: {other:?}"
            )))
        }
    };
    let count = match read_attribute(mount, LAYER_COUNT)?.value {
        Value::I32(v) if v.len() == 1 && v[0] >= 0 => v[0],
        other => {
            return Err(corrupt(format!(
                "the mock's layer count is not one whole number of zero or more: {other:?}"
            )))
        }
    };
    let mut meta = BuildMeta::new(BuildIdentity {
        build_id,
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    });
    meta.layers_declared = Some(i64::from(count));
    for n in 1..=count.unsigned_abs() {
        meta.layers.insert(LayerId(n), LayerMeta::new(LayerId(n)));
    }
    let mut source = InMemorySource::new();
    source.set_build(meta);
    Ok(Box::new(source))
}

/// Writes a mock build of the version `key` into a group: the marker, the build's id and the number of layers. The group can be anywhere, because nothing the
/// reader reads depends on where it is. This is how a test makes a mock file, standalone or nested.
pub fn write_mock_file_for(
    mount: &Group,
    key: &str,
    build_id: &str,
    layers: i32,
) -> Result<(), Error> {
    let written = "writing a mock file";
    let version: i32 = key.parse().map_err(|_| {
        Error::new(ErrorKind::Corrupt {
            detail: format!("{written}: {key:?} is not a mock version"),
        })
    })?;
    mount
        .new_attr::<i32>()
        .create(MARKER)
        .and_then(|a| a.write_scalar(&version))
        .map_err(|e| from_binding(written, e))?;
    let text: VarLenUnicode = build_id.parse().map_err(|e| {
        Error::new(ErrorKind::Corrupt {
            detail: format!("{written}: {e}"),
        })
    })?;
    mount
        .new_attr::<VarLenUnicode>()
        .create(BUILD_ID)
        .and_then(|a| a.write_scalar(&text))
        .map_err(|e| from_binding(written, e))?;
    mount
        .new_attr::<i32>()
        .create(LAYER_COUNT)
        .and_then(|a| a.write_scalar(&layers))
        .map_err(|e| from_binding(written, e))?;
    Ok(())
}

/// [`write_mock_file_for`] for the mock version 9001.
pub fn write_mock_file(mount: &Group, build_id: &str, layers: i32) -> Result<(), Error> {
    write_mock_file_for(mount, KEY, build_id, layers)
}
