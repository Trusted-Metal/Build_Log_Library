//! Adding a version is a checkable property (IP-04 phase 4E, task T04.10; requirement BL-VA-06).
//!
//! A second version, `9002`, is defined **entirely in this file**: its own mount, its own marker attribute stored as **text** (the mock's is an integer), and its
//! own layout (the data sits in a subgroup of the mount, which the mock has no use for). It is injected into a registry beside the mock and opened. Nothing in
//! the dispatcher, the registry or the mock changes to make it work, and the list of supported versions grows without anyone editing a list. If this test ever
//! needs a change outside this file to pass, adding a version has stopped being only an adapter.

mod common;

use build_log_io::adapters::{mock, AdapterSet, MarkerPlace};
use build_log_io::dispatch::{open, open_at, production, Registry};
use build_log_io::hdf5::{read_attribute, Value};
use build_log_model::{
    BuildIdentity, BuildMeta, BuildSource, Error, ErrorKind, InMemorySource, LayerId, LayerMeta,
};
use common::{put_scalar, put_text, temp_file, TempH5};
use hdf5::Group;

const MOUNT: &str = "Second_Mount";

/// The second version's reader. It reads the title and the count from the subgroup `meta` of the mount it is handed: **relative to the mount**, like every adapter.
fn read_second(mount: &Group) -> Result<Box<dyn BuildSource>, Error> {
    let meta = mount.group("meta").map_err(|e| {
        Error::new(ErrorKind::Corrupt {
            detail: format!("no meta group: {e}"),
        })
    })?;
    let title = match read_attribute(&meta, "title")?.value {
        Value::Text(mut v) if v.len() == 1 => v.remove(0),
        other => {
            return Err(Error::new(ErrorKind::Corrupt {
                detail: format!("the title is not one text: {other:?}"),
            }))
        }
    };
    let count = match read_attribute(&meta, "n")?.value {
        Value::I64(v) if v.len() == 1 => v[0],
        other => {
            return Err(Error::new(ErrorKind::Corrupt {
                detail: format!("the count is not one integer: {other:?}"),
            }))
        }
    };
    let mut build = BuildMeta::new(BuildIdentity {
        build_id: title,
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    });
    build.layers_declared = Some(count);
    for n in 1..=count {
        let id = LayerId(u32::try_from(n).expect("a layer id"));
        build.layers.insert(id, LayerMeta::new(id));
    }
    let mut source = InMemorySource::new();
    source.set_build(build);
    Ok(Box::new(source))
}

fn second_version() -> AdapterSet {
    AdapterSet::builder("9002", "second", MarkerPlace::new(MOUNT, "Schema"))
        .reader(read_second)
        .without_writer("the second version has no writer")
        .without_patcher("the second version has no patcher")
        .build()
        .expect("a complete set")
}

/// The second version's layout, written into any group.
fn write_second(mount: &Group, title: &str, layers: i64) {
    put_text(mount, "Schema", "9002");
    let meta = mount.create_group("meta").expect("a subgroup");
    put_text(&meta, "title", title);
    put_scalar(&meta, "n", layers);
}

fn second_file(label: &str, title: &str, layers: i64) -> TempH5 {
    temp_file(label, |f| {
        let mount = f.create_group(MOUNT).expect("the mount");
        write_second(&mount, title, layers);
    })
}

#[test]
fn adding_a_version_touches_only_its_own_adapter() {
    let before = Registry::new()
        .with(mock::adapter_set())
        .expect("the mock registers");
    assert_eq!(before.supported_versions(), ["9001"]);
    // adding the version is one registration: no dispatcher, registry or mock code changes, and the list of versions is derived
    let registry = before
        .clone()
        .with(second_version())
        .expect("a new key registers");
    assert_eq!(registry.supported_versions(), ["9001", "9002"]);

    let second = second_file("adding-9002", "the second version's build", 5);
    let opened = open(&registry, &second.0).expect("the new version opens");
    assert_eq!(opened.key().as_str(), "9002");
    assert_eq!(opened.adapter(), "second");
    let build = opened.source().build().expect("the metadata");
    assert_eq!(build.identity.build_id, "the second version's build");
    assert_eq!(opened.source().layers().expect("layers").len(), 5);

    // the version that was there is untouched: its files read as before, through the same registry
    let first = temp_file("adding-9001", |f| {
        let mount = f.create_group(mock::MOUNT).expect("the mount");
        mock::write_mock_file(&mount, "the first version's build", 2).expect("a mock file");
    });
    let opened = open(&registry, &first.0).expect("the old version still opens");
    assert_eq!(opened.key().as_str(), "9001");
    assert_eq!(
        opened.source().build().unwrap().identity.build_id,
        "the first version's build"
    );

    // a mount of the new version nested at any depth reads the same, because the adapter reads relative to its mount
    let nested = temp_file("adding-9002-nested", |f| {
        let deep = f
            .create_group("a")
            .and_then(|g| g.create_group("b"))
            .expect("nested groups");
        write_second(&deep, "the second version's build", 5);
    });
    let file = hdf5::File::open(&nested.0).expect("opens");
    let attached = open_at(&registry, &file.group("a/b").expect("the nested mount"))
        .expect("a nested mount opens");
    assert_eq!(
        *attached.source().build().unwrap(),
        *open(&registry, &second.0)
            .unwrap()
            .source()
            .build()
            .unwrap()
    );

    // and none of it reached the production registry, which only real versions are added to
    assert!(!production()
        .supported_versions()
        .iter()
        .any(|v| v == "9002" || v == "9001"));
}
