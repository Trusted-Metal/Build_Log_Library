//! The camera images of a V17 build (IP-13 phase 13F, task T13.8; decisions D13-21 and D13-26).
//!
//! * Each phase of a layer's images (`Pre_Exposure` and `Post_Exposure`) is a channel of the whole build, of the kind image, recorded in the layers that have a frame,
//!   with the format of the frame: its rows, its columns and the type of a pixel. A frame is described and not read.
//! * Only a frame of two dimensions is described, and a channel has one format: anything else is reported on its layer and kept whole.
//! * What else an image holds is kept in the layer's `extra`. The reader declares that it returns images.
//!
//! The conformance of the images file, images included, is held by `v17_gate.rs`, and nothing is dropped there too. The small files here are made at test time.

mod common;

use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildMeta, ChannelDType, ChannelKind, ClockClass, Direction, Feature, ImageFormat, LayerId,
    NumericType, Origin, Presence,
};
use build_log_testkit::{miniature, miniature_path};
use common::{layer, monitoring, put_text, temp_h5, TempH5};
use hdf5::Group;

fn images_file() -> BuildMeta {
    let opened = open(
        production(),
        miniature_path(miniature::V17_2026_08_18_IMAGES),
    )
    .expect("the file opens");
    (*opened.source().build().expect("the metadata")).clone()
}

/// A file whose mount has a build id and the layers that `write` makes, read as a V17 build.
fn synthetic(label: &str, write: impl FnOnce(&Group)) -> BuildMeta {
    let on_disk: TempH5 = temp_h5(label, |mount| {
        put_text(mount, layout::BUILD_ID, "a build");
        write(&monitoring(mount));
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    v17::read::read_build(&mount).expect("the build reads")
}

/// A frame of the given shape and pixel type in the image group `phase` of a layer's group.
fn frame<T: hdf5::H5Type>(layer: &Group, phase: &str, shape: &[usize]) {
    let holder = layer
        .group(phase)
        .or_else(|_| layer.create_group(phase))
        .expect("an image group");
    holder
        .new_dataset::<T>()
        .shape(shape)
        .create(layout::RAW)
        .expect("a frame");
}

fn format_of(build: &BuildMeta, phase: &str) -> Option<ImageFormat> {
    build
        .sensors
        .iter()
        .find(|c| c.key.to_string() == phase)
        .and_then(|c| match &c.dtype {
            ChannelDType::Image(format) => Some(format.clone()),
            _ => None,
        })
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

// ---------------------------------------------------------------- the images file

#[test]
fn an_image_is_a_channel_of_the_build_with_its_frame_and_its_layers() {
    let build = images_file();
    let keys: Vec<String> = build.sensors.iter().map(|c| c.key.to_string()).collect();
    // in the order of the phases, before and then after the exposure
    assert_eq!(keys, [layout::PRE_EXPOSURE, layout::POST_EXPOSURE]);
    for channel in &build.sensors {
        assert_eq!(channel.kind, ChannelKind::Image);
        assert_eq!(channel.clock_class, ClockClass::Image);
        assert_eq!(
            channel.dtype,
            ChannelDType::Image(ImageFormat::new(128, 128, NumericType::U8))
        );
        // one frame in each of the three layers, and no unit that anyone has said
        let layers: Vec<(u32, Option<u64>)> = channel
            .presence
            .iter()
            .map(|(id, p)| (id.0, p.samples()))
            .collect();
        assert_eq!(layers, [(1, Some(1)), (2, Some(1)), (3, Some(1))]);
        assert_eq!(channel.unit_source, build_log_model::UnitSource::Unknown);
    }
}

#[test]
fn the_images_file_conforms() {
    use build_log_conformance::{check, expectations_dir, Expectation};
    let expectation =
        Expectation::load(&expectations_dir().join("v17_2026_08_18_images.json")).expect("a file");
    let found = check(&images_file(), &expectation);
    assert!(found.is_empty(), "{}", found.join("\n  "));
}

#[test]
fn what_else_an_image_holds_is_kept_in_its_layer() {
    let build = images_file();
    let layer = build.layers.get(LayerId(1)).expect("the first layer");
    // the attributes of the frame (the HDF5 image conventions) are kept, by the path of the frame below the layer; the frame's samples are not held
    let frame = layer
        .extra
        .node(&path(&[layout::PRE_EXPOSURE, layout::RAW]))
        .expect("the frame's attributes are kept");
    assert_eq!(frame.dims.as_slice(), [128, 128]);
    let names: Vec<&str> = frame.attributes().map(|(n, _)| n).collect();
    assert_eq!(names, ["CLASS", "IMAGE_SUBCLASS", "IMAGE_VERSION"]);
    // the image group itself holds nothing but the frame, so it is not a node of its own; the events inside the layer are read as series (phase 13K) and hold
    // nothing more, so they are not nodes either
    assert!(layer.extra.node(&path(&[layout::PRE_EXPOSURE])).is_none());
    assert!(!layer
        .extra
        .nodes()
        .any(|n| n.path.first().map(String::as_str) == Some(layout::GLOBAL_SENSORS)));
    // and nothing is said about an image that was read
    assert!(layer.origins.get("image Pre_Exposure").is_none());
}

#[test]
fn a_file_with_no_images_has_no_image_channel() {
    for file in [
        miniature::V17_MOCK,
        miniature::V17_TRIGGERED_2026_10_19,
        miniature::V17_MARCH,
    ] {
        let opened = open(production(), miniature_path(file)).expect("opens");
        let build = opened.source().build().expect("the metadata");
        assert!(build.sensors.is_empty(), "{file}");
    }
}

// ---------------------------------------------------------------- the format of a frame

#[test]
fn rows_and_columns_are_not_mixed_up_and_the_pixel_type_is_the_stored_one() {
    let build = synthetic("images-format", |layers| {
        let first = layer(layers, 1);
        frame::<u8>(&first, layout::PRE_EXPOSURE, &[4, 6]);
        frame::<u16>(&first, layout::POST_EXPOSURE, &[3, 5]);
    });
    assert_eq!(
        format_of(&build, layout::PRE_EXPOSURE),
        Some(ImageFormat::new(4, 6, NumericType::U8))
    );
    assert_eq!(
        format_of(&build, layout::POST_EXPOSURE),
        Some(ImageFormat::new(3, 5, NumericType::U16))
    );
}

#[test]
fn a_layer_without_an_image_has_no_presence_for_it() {
    let build = synthetic("images-presence", |layers| {
        frame::<u8>(&layer(layers, 1), layout::PRE_EXPOSURE, &[2, 2]);
        layer(layers, 2);
        frame::<u8>(&layer(layers, 3), layout::PRE_EXPOSURE, &[2, 2]);
    });
    let channel = &build.sensors[0];
    let ids: Vec<u32> = channel.presence.ids().map(|id| id.0).collect();
    assert_eq!(ids, [1, 3]);
    assert_eq!(
        channel.presence.get(LayerId(3)).copied(),
        Some(Presence::from_count(1))
    );
    // only the phase that has a frame is a channel
    assert_eq!(build.sensors.len(), 1);
}

#[test]
fn a_frame_that_is_not_two_dimensional_is_reported_on_its_layer_and_kept_whole() {
    let build = synthetic("images-three-dimensions", |layers| {
        let first = layer(layers, 1);
        frame::<u8>(&first, layout::PRE_EXPOSURE, &[2, 2, 3]);
        frame::<u8>(&first, layout::POST_EXPOSURE, &[2, 2]);
    });
    // the second phase is read; the first is a fault in its layer: no channel, an origin that says why, and its group kept whole
    assert!(format_of(&build, layout::PRE_EXPOSURE).is_none());
    assert!(format_of(&build, layout::POST_EXPOSURE).is_some());
    let layer = build.layers.get(LayerId(1)).unwrap();
    match layer.origins.get("image Pre_Exposure") {
        Some(Origin::NotRead { cause, reason }) => {
            assert_eq!(*cause, build_log_model::NotReadCause::Fault);
            assert!(reason.contains("two dimensions"), "{reason}");
        }
        other => panic!("expected the reason, got {other:?}"),
    }
    let kept = layer
        .extra
        .node(&path(&[layout::PRE_EXPOSURE, layout::RAW]))
        .expect("the frame that was not read is kept, with its extents");
    assert_eq!(kept.dims.as_slice(), [2, 2, 3]);
}

#[test]
fn a_frame_stored_differently_in_another_layer_is_reported_on_that_layer() {
    let build = synthetic("images-two-formats", |layers| {
        frame::<u8>(&layer(layers, 1), layout::PRE_EXPOSURE, &[4, 6]);
        frame::<u8>(&layer(layers, 2), layout::PRE_EXPOSURE, &[5, 6]);
    });
    // a channel has one format: the first layer's, and the second layer is left out with the reason
    assert_eq!(
        format_of(&build, layout::PRE_EXPOSURE),
        Some(ImageFormat::new(4, 6, NumericType::U8))
    );
    let ids: Vec<u32> = build.sensors[0].presence.ids().map(|id| id.0).collect();
    assert_eq!(ids, [1]);
    let second = build.layers.get(LayerId(2)).unwrap();
    assert!(matches!(
        second.origins.get("image Pre_Exposure"),
        Some(Origin::NotRead { reason: why, .. }) if why.contains("other layers")
    ));
    assert!(second
        .extra
        .node(&path(&[layout::PRE_EXPOSURE, layout::RAW]))
        .is_some());
}

#[test]
fn an_image_group_with_no_frame_is_kept_whole_and_describes_no_channel() {
    let build = synthetic("images-no-frame", |layers| {
        let first = layer(layers, 1);
        let holder = first.create_group(layout::PRE_EXPOSURE).unwrap();
        holder.create_group("Notes").unwrap();
    });
    assert!(build.sensors.is_empty());
    let kept = build
        .layers
        .get(LayerId(1))
        .unwrap()
        .extra
        .node(&path(&[layout::PRE_EXPOSURE, "Notes"]));
    assert!(kept.is_some());
}

// ---------------------------------------------------------------- the capabilities

#[test]
fn images_are_declared_in_the_capabilities() {
    let set = production().sets().next().expect("the V17 adapter set");
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Images));
    // reading is declared and writing is not: the writer arrives with IP-08
    assert!(!set
        .capabilities()
        .supports(Direction::Write, Feature::Images));
}
