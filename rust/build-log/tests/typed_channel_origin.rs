//! The typed lookup of a channel's origin (IP-26 phase 26C, finding F16 of IP-24), shown on the committed miniatures through the facade alone.
//!
//! Before it, a consumer that wanted to know whether a channel's dataspace is null, or why a channel was not read, had to build the text key `channel <name>` and read the
//! side table of the laser's entry for the layer. `LaserLayerMeta::channel_origin` asks the same thing by a method, so the consumer never builds the key. The change adds
//! and moves nothing: this file shows that on every miniature that the facade opens, **the lookup returns exactly what the text key holds**, in both directions:
//!
//! * every origin that the side table holds under a channel's key is found by the lookup, with the same value;
//! * for every channel that a laser describes, the lookup returns what the key built by hand holds, so nothing is invented where the table holds nothing.
//!
//! **The miniatures are healthy files: none of them holds a channel origin today** (the count this test prints says so), so on them the test shows the second direction, that
//! the lookup invents nothing. The first direction, with origins that exist, is shown by the IO tests on files that are made abnormal on purpose (a null dataspace, a channel
//! that cannot be read: `v17_streams.rs`, `v17_lasers.rs` and `v16_lasers.rs`), which now hold the lookup to the text key as well.
//!
//! V18 is not here: it is a draft, and the production registry reads V16 and V17 only. The text `channel ` below is spelled out on purpose: it is the convention that
//! consumers built by hand until now, and the test holds the lookup to it.

use build_log::open;
use build_log_testkit::{miniature, miniature_path};

/// The miniatures that the facade opens: V16 and the six of V17.
const OPENABLE: [&str; 7] = [
    miniature::V16,
    miniature::V17_MARCH,
    miniature::V17_MOCK,
    miniature::V17_OCT15,
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
    miniature::V17_SPECIAL_LAYERS,
];

#[test]
fn on_every_miniature_the_typed_lookup_returns_what_the_text_key_holds() {
    let mut entries = 0usize;
    let mut channels_asked = 0usize;
    let mut origins_found = 0usize;
    for file in OPENABLE {
        let opened = open(miniature_path(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let build = opened
            .source()
            .build()
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        for laser in build.lasers() {
            for (layer, entry) in laser.layers.iter() {
                entries += 1;
                let here = format!("{file}, laser {:?}, layer {layer:?}", laser.key);
                // every origin the side table holds under a channel's key is found by the lookup
                for (field, origin) in entry.origins.iter() {
                    if let Some(name) = field.strip_prefix("channel ") {
                        origins_found += 1;
                        assert_eq!(entry.channel_origin(name), Some(origin), "{here}: {field}");
                    }
                }
                // and for every channel the laser describes, the lookup holds what the key built by hand holds
                for channel in &laser.channels {
                    let Some(name) = channel.key.name() else {
                        continue;
                    };
                    channels_asked += 1;
                    assert_eq!(
                        entry.channel_origin(name),
                        entry.origins.get(&format!("channel {name}")),
                        "{here}: {name}"
                    );
                }
            }
        }
    }
    println!(
        "{} miniatures, {entries} laser entries, {channels_asked} channels asked, {origins_found} channel origins found",
        OPENABLE.len()
    );
    assert!(
        entries > 0 && channels_asked > 0,
        "the test looked at nothing"
    );
}
