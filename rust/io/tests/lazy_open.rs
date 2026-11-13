//! The lazy open (IP-26 phase 26K, decision D26-19): a file is opened by its build's own level and the list of its layers, and **a layer is read when it is asked for.**
//!
//! * `a_lazy_build_is_the_eager_build`: the whole build that a lazy source gives is, on every miniature that opens, equal to the one the whole walk reads (`read_build`, which is
//!   what `open` used to call).
//! * `a_layer_by_itself_is_its_slice_of_the_whole`: for every layer of every miniature, the layer read by itself is the slice of the whole build for that layer, and so are the
//!   list of the layers and the lasers in a layer.
//! * `the_overview_is_the_build_level_of_the_whole`: the overview agrees with the whole build on everything that is the build's own, and on the parameters of each laser it lists.
//! * `a_request_walks_only_the_layer_it_names`: **the counters.** The open reads no layer; a layer read, a list of its lasers and a column read each read one layer, once; the whole
//!   build is walked the first time it is asked for, and once.
//! * `an_unknown_layer_is_an_invalid_index`, `a_fault_inside_a_layer_shows_when_that_layer_is_read`: the one change in behavior that the lazy open makes.

mod common;

use build_log_io::adapters::v16;
use build_log_io::adapters::v17::{self, layout, source::V17Source};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildMeta, BuildOverview, BuildSource, ColumnRequest, ColumnSource, ErrorKind, LayerDetail,
    LayerId, Presence, ReadContext,
};
use build_log_testkit::{miniature, miniature_path};
use common::{put_array, temp_h5};

/// The miniatures that open (the V18 draft does not).
const OPENABLE: [&str; 7] = [
    miniature::V16,
    miniature::V17_MARCH,
    miniature::V17_MOCK,
    miniature::V17_OCT15,
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
    miniature::V17_SPECIAL_LAYERS,
];

/// The build as the whole walk reads it, straight from the reader and not through a source.
fn eager(name: &str) -> BuildMeta {
    let file = hdf5::File::open(miniature_path(name)).expect("the miniature opens");
    let mount = file.group(layout::MOUNT).expect("the mount");
    if name == miniature::V16 {
        v16::read::read_build(&mount)
    } else {
        v17::read::read_build(&mount)
    }
    .expect("the whole walk reads the miniature")
}

#[test]
fn a_lazy_build_is_the_eager_build() {
    for name in OPENABLE {
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let lazy = opened.source().build().expect("the whole build");
        assert!(*lazy == eager(name), "{name}: the two builds differ");
    }
}

#[test]
fn a_layer_by_itself_is_its_slice_of_the_whole() {
    for name in OPENABLE {
        let whole = eager(name);
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let source = opened.source();
        // asked before the whole build is: no layer has been walked by anything but itself
        let ids = source.layers().expect("the layers");
        assert_eq!(ids, whole.layers.ids().collect::<Vec<_>>(), "{name}");
        for id in ids {
            let by_itself = source.layer_meta(id).expect("one layer");
            let slice = LayerDetail::from_build(&whole, id).expect("the layer is in the whole");
            assert!(*by_itself == slice, "{name}, layer {id}: the two differ");
            let lasers = source.lasers_in_layer(id).expect("the lasers of a layer");
            let in_whole: Vec<_> = whole
                .lasers()
                .iter()
                .filter(|laser| laser.layers.contains(id))
                .map(|laser| laser.key.clone())
                .collect();
            assert_eq!(lasers, in_whole, "{name}, layer {id}");
        }
    }
}

#[test]
fn the_overview_is_the_build_level_of_the_whole() {
    for name in OPENABLE {
        let whole = eager(name);
        let derived = BuildOverview::from_build(&whole);
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let overview = opened.source().overview().expect("the overview");
        assert_eq!(overview.identity, derived.identity, "{name}");
        assert_eq!(overview.layers, derived.layers, "{name}");
        assert_eq!(overview.layers_declared, derived.layers_declared, "{name}");
        assert_eq!(overview.layer_thickness, derived.layer_thickness, "{name}");
        assert_eq!(overview.quality, derived.quality, "{name}");
        assert_eq!(overview.provenance, derived.provenance, "{name}");
        assert_eq!(overview.context, derived.context, "{name}");
        assert_eq!(overview.origins, derived.origins, "{name}");
        // the lasers that the calibration names, each with the parameters that the whole build gives it
        for laser in &overview.lasers {
            let in_whole = derived
                .lasers
                .iter()
                .find(|other| other.key == laser.key)
                .unwrap_or_else(|| panic!("{name}: the whole build has no {}", laser.key));
            assert_eq!(
                laser.parameters, in_whole.parameters,
                "{name}, {}",
                laser.key
            );
        }
    }
}

/// A recorded column of the build: the laser, the channel and the layer, for the first channel that holds samples.
fn recorded_column(whole: &BuildMeta) -> ColumnRequest {
    for laser in whole.lasers() {
        for channel in &laser.channels {
            for (layer, presence) in channel.presence.iter() {
                if matches!(presence, Presence::Recorded { .. }) {
                    return ColumnRequest::new(layer, Some(laser.key.clone()), channel.key.clone());
                }
            }
        }
    }
    panic!("the build records no column");
}

fn v17_source(name: &str) -> V17Source {
    let file = hdf5::File::open(miniature_path(name)).expect("the miniature opens");
    let mount = file.group(layout::MOUNT).expect("the mount");
    V17Source::open(&mount).expect("the source opens")
}

#[test]
fn a_request_walks_only_the_layer_it_names() {
    // the two miniatures of V17 that hold more than one layer: the mock (two) and the special layers (seven)
    for name in [miniature::V17_MOCK, miniature::V17_SPECIAL_LAYERS] {
        let whole = eager(name);
        let ids: Vec<LayerId> = whole.layers.ids().collect();
        assert!(ids.len() >= 2, "{name}: the test needs two layers");

        // the open reads no layer
        let source = v17_source(name);
        let stats = source.meta().stats();
        assert_eq!(
            (stats.layers_read, stats.full_walks),
            (0, 0),
            "{name}: open"
        );

        // the list of the layers and the build's own level read none either
        assert_eq!(source.layers().expect("the layers"), ids);
        source.overview().expect("the overview");
        let stats = source.meta().stats();
        assert_eq!(
            (stats.layers_read, stats.full_walks),
            (0, 0),
            "{name}: overview"
        );

        // one layer, asked three ways, is read once
        source.layer_meta(ids[0]).expect("a layer");
        source.lasers_in_layer(ids[0]).expect("its lasers");
        source.layer_meta(ids[0]).expect("a layer again");
        let stats = source.meta().stats();
        assert_eq!(
            (stats.layers_read, stats.full_walks),
            (1, 0),
            "{name}: one layer"
        );

        // a column read reads the layer that it names, and the whole build is not walked for it
        let source = v17_source(name);
        let request = recorded_column(&whole);
        source
            .read(&request, &ReadContext::new())
            .expect("a recorded column");
        let stats = source.meta().stats();
        assert_eq!(
            (stats.layers_read, stats.full_walks),
            (1, 0),
            "{name}: a column"
        );

        // the whole build is walked the first time it is asked for, and once
        let first = source.build().expect("the whole build");
        let second = source.build().expect("the whole build again");
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(
            source.meta().stats().full_walks,
            1,
            "{name}: the whole build"
        );
        source.validate().expect("the findings");
        source.availability().expect("the availability");
        assert_eq!(source.meta().stats().full_walks, 1, "{name}: the reports");
    }
}

#[test]
fn an_unknown_layer_is_an_invalid_index() {
    let source = v17_source(miniature::V17_OCT15);
    for error in [
        source
            .layer_meta(LayerId(999_999))
            .map(|_| ())
            .expect_err("no such layer"),
        source
            .lasers_in_layer(LayerId(999_999))
            .map(|_| ())
            .expect_err("no such layer"),
    ] {
        assert!(
            matches!(error.kind(), ErrorKind::InvalidIndex { .. }),
            "{:?}",
            error.kind()
        );
    }
    assert_eq!(source.meta().stats().layers_read, 0);
}

#[test]
fn a_fault_inside_a_layer_shows_when_that_layer_is_read() {
    // the one change in behavior (decision D26-19): a layer whose own attribute is wrong no longer fails the open; it fails when that layer is read, and the whole walk fails
    let on_disk = temp_h5("lazy-open-fault", |mount| {
        let monitoring = mount
            .create_group(layout::MONITORING_DATA)
            .expect("the layers' group");
        for id in [1u32, 2] {
            monitoring
                .create_group(&format!("{}{id}", layout::LAYER_PREFIX))
                .expect("a layer's group");
        }
        // two values where one belongs
        let broken = monitoring
            .group(&format!("{}2", layout::LAYER_PREFIX))
            .expect("layer 2");
        put_array(&broken, layout::START_LAYER_TIME, &[1i64, 2]);
    });
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");

    let source =
        V17Source::open(&mount).expect("the open reads no layer, so it finds no fault in one");
    source
        .layer_meta(LayerId(1))
        .expect("the sound layer reads");
    let error = source
        .layer_meta(LayerId(2))
        .map(|_| ())
        .expect_err("the broken layer does not");
    assert!(
        matches!(error.kind(), ErrorKind::Corrupt { .. }),
        "{:?}",
        error.kind()
    );
    let whole = source
        .build()
        .map(|_| ())
        .expect_err("nor does the whole walk");
    assert!(
        matches!(whole.kind(), ErrorKind::Corrupt { .. }),
        "{:?}",
        whole.kind()
    );
}
