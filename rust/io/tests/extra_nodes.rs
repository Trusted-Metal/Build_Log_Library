//! Keeping the groups and datasets an adapter does not type (IP-13 phase 13D; requirement BL-CD-08): each is kept by its path with its own attributes, a
//! dataset with its extents and element type and never its samples, a typed child is skipped, and the walk is bounded.

mod common;

use build_log_io::hdf5::{
    keep_dataset_remainder, keep_group_remainder, keep_node, keep_other_nodes, MAX_DEPTH,
};
use build_log_model::{AttrValue, Extra, NodeKind, StoredClass};
use common::{put_scalar, put_text, temp_h5};

/// A scalar attribute on any object: a group or a dataset.
fn put<T: hdf5::H5Type>(object: &hdf5::Location, name: &str, value: T) {
    object
        .new_attr::<T>()
        .create(name)
        .expect("an attribute")
        .write_scalar(&value)
        .expect("it is written");
}
use hdf5::Group;

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

fn with_mount<T>(label: &str, write: impl FnOnce(&Group), read: impl FnOnce(&Group) -> T) -> T {
    let on_disk = temp_h5(label, write);
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    read(&mount)
}

#[test]
fn unmapped_groups_and_datasets_are_kept_by_path_with_their_attributes_and_extents() {
    let extra = with_mount(
        "nodes-tree",
        |mount| {
            let kept = mount.create_group("Kept").unwrap();
            put_text(&kept, "A_Note", "on a group");
            let inner = kept.create_group("Inner").unwrap();
            inner
                .new_dataset::<f64>()
                .shape((3, 4))
                .create("Table")
                .unwrap();
            put(&inner.dataset("Table").unwrap(), "A_Scale", 2.5f32);
            kept.new_dataset::<u8>().shape(5).create("Bytes").unwrap();
            mount.create_group("Typed").unwrap();
            mount
                .new_dataset::<i32>()
                .shape(1)
                .create("Also_Typed")
                .unwrap();
        },
        |mount| {
            let mut extra = Extra::new();
            keep_other_nodes(mount, &["Typed", "Also_Typed"], &[], &mut extra)
                .expect("the nodes are kept");
            extra
        },
    );
    // the typed children are skipped; everything else is there, in path order
    let paths: Vec<String> = extra.nodes().map(|n| n.path.join("/")).collect();
    assert_eq!(
        paths,
        ["Kept", "Kept/Bytes", "Kept/Inner", "Kept/Inner/Table"]
    );
    let group = extra.node(&path(&["Kept"])).unwrap();
    assert_eq!(group.kind, NodeKind::Group);
    assert_eq!(
        group.attribute("A_Note").unwrap().value,
        AttrValue::Text(vec!["on a group".into()])
    );
    // a dataset keeps its extents and its element type, and its own attributes; its samples are not read
    let table = extra.node(&path(&["Kept", "Inner", "Table"])).unwrap();
    assert_eq!(table.kind, NodeKind::Dataset);
    assert_eq!(table.dims.as_slice(), [3, 4]);
    let element = table.element.as_ref().expect("an element type");
    assert_eq!(
        (element.class.clone(), element.bits),
        (StoredClass::Float, Some(64))
    );
    assert_eq!(
        table.attribute("A_Scale").unwrap().value,
        AttrValue::Float(vec![2.5])
    );
    assert_eq!(
        extra
            .node(&path(&["Kept", "Bytes"]))
            .unwrap()
            .dims
            .as_slice(),
        [5]
    );
}

#[test]
fn a_node_can_be_kept_below_a_base_path_and_a_typed_group_keeps_only_what_is_left_of_it() {
    let extra = with_mount(
        "nodes-remainder",
        |mount| {
            let holder = mount.create_group("Holder").unwrap();
            holder
                .new_dataset::<f32>()
                .shape(2)
                .create("Typed_Column")
                .unwrap();
            holder
                .new_dataset::<f32>()
                .shape(2)
                .create("Spare_Column")
                .unwrap();
            put_scalar(&holder, "A_Count", 4i32);
            let quiet = mount.create_group("Quiet").unwrap();
            quiet
                .new_dataset::<f32>()
                .shape(2)
                .create("Typed_Column")
                .unwrap();
            mount.create_group("Beside").unwrap();
        },
        |mount| {
            let mut extra = Extra::new();
            for name in ["Holder", "Quiet"] {
                let group = mount.group(name).unwrap();
                keep_group_remainder(&group, &["Typed_Column"], &[], &path(&[name]), &mut extra)
                    .unwrap();
            }
            keep_node(mount, "Beside", &path(&[".."]), &mut extra).unwrap();
            extra
        },
    );
    let paths: Vec<String> = extra.nodes().map(|n| n.path.join("/")).collect();
    // the group Holder has an attribute and an unlisted dataset, so it is kept as a node with them; the group Quiet has nothing that is not typed, so it is not
    // a node at all; and a node beside the mount starts with `..`
    assert_eq!(paths, ["../Beside", "Holder", "Holder/Spare_Column"]);
    assert!(extra
        .node(&path(&["Holder"]))
        .unwrap()
        .attribute("A_Count")
        .is_some());
}

#[test]
fn a_typed_dataset_is_kept_only_if_it_has_attributes_that_are_not_typed() {
    let extra = with_mount(
        "nodes-dataset-remainder",
        |mount| {
            let plain = mount.new_dataset::<f32>().shape(2).create("Plain").unwrap();
            let _ = plain;
            let noted = mount.new_dataset::<f32>().shape(7).create("Noted").unwrap();
            put(&noted, "Units_Of_Something", 1i32);
            put(&noted, "Typed_Attribute", 2i32);
        },
        |mount| {
            let mut extra = Extra::new();
            keep_dataset_remainder(
                &mount.dataset("Plain").unwrap(),
                &[],
                &path(&["Plain"]),
                &mut extra,
            )
            .unwrap();
            keep_dataset_remainder(
                &mount.dataset("Noted").unwrap(),
                &["Typed_Attribute"],
                &path(&["Noted"]),
                &mut extra,
            )
            .unwrap();
            extra
        },
    );
    let paths: Vec<String> = extra.nodes().map(|n| n.path.join("/")).collect();
    assert_eq!(paths, ["Noted"]);
    let noted = extra.node(&path(&["Noted"])).unwrap();
    assert_eq!(noted.dims.as_slice(), [7]);
    let names: Vec<&str> = noted.attributes().map(|(n, _)| n).collect();
    assert_eq!(names, ["Units_Of_Something"]);
}

#[test]
fn the_walk_stops_at_the_depth_limit_and_says_so() {
    let extra = with_mount(
        "nodes-deep",
        |mount| {
            // a chain of groups deeper than the walk follows
            let mut group = mount.create_group("a").unwrap();
            for _ in 0..(MAX_DEPTH + 6) {
                group = group.create_group("a").unwrap();
            }
        },
        |mount| {
            let mut extra = Extra::new();
            keep_other_nodes(mount, &[], &[], &mut extra).expect("a deep file is not an error");
            extra
        },
    );
    let deepest = extra.nodes().max_by_key(|n| n.path.len()).unwrap();
    assert_eq!(
        deepest.path.len(),
        MAX_DEPTH + 1,
        "the walk stopped where it said it would"
    );
    match &deepest.kind {
        NodeKind::Other(why) => assert!(why.contains("not followed"), "{why}"),
        other => panic!("expected the reason, got {other:?}"),
    }
    assert!(
        extra.len() == MAX_DEPTH + 1,
        "nothing below the limit was kept: {} nodes",
        extra.len()
    );
}
