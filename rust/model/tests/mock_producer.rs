//! The mock producer of `examples/mock_producer.rs` builds what the CDM can hold, for both protocols (IP-02 phase 2M, task T02.27).
//!
//! The example is included here so that it is compiled and run by `cargo test`: a change to the CDM that breaks what tauri would hand over fails this test.

#[allow(dead_code)]
#[path = "../examples/mock_producer.rs"]
mod producer;

use build_log_model::{
    Axis, CapturedEncoding, ChannelDType, ChannelKind, EnumLabels, LayerId, NumericType, Presence,
};
use producer::{add_layer, build_meta, channels_for, example_config, Protocol};

#[test]
fn a_producer_built_like_taurus_writer_hands_over_a_valid_cdm_for_both_protocols() {
    for protocol in [Protocol::Xy2100E, Protocol::Sl2100] {
        let config = example_config(protocol);
        let mut meta = build_meta(&config);
        add_layer(
            &mut meta,
            LayerId(1),
            727_922,
            (1_792_165_086_845_000, 1_792_165_087_746_592),
        );
        // the identity is what tauri has, and what it has not is stated as absent
        assert_eq!(meta.identity.build_id, "example-build");
        assert_eq!(
            meta.identity.date.as_deref(),
            Some("2026-10-16 15:38:06 UTC+00:00")
        );
        assert_eq!(
            (
                meta.identity.project_id.clone(),
                meta.identity.source_mcf_hash.clone()
            ),
            (None, None)
        );
        assert_eq!(meta.layers_declared, Some(0));
        // a layer that was recorded by every laser is a layer of the build, and every laser has it
        assert_eq!(meta.layers_present(), 1);
        for laser in meta.lasers() {
            assert_eq!(
                laser.layers.get(LayerId(1)).map(|l| l.presence),
                Some(Presence::from_count(727_922))
            );
            assert!(
                laser.channels.iter().all(|c| c.check().is_ok()),
                "every descriptor is valid"
            );
            assert!(
                laser
                    .channels
                    .iter()
                    .all(|c| c.presence.get(LayerId(1)).is_some()),
                "every channel says it was recorded in the layer"
            );
        }
    }
}

#[test]
fn xy2_and_sl2_differ_in_the_channels_and_the_stored_type_of_the_commanded_positions() {
    let laser = &example_config(Protocol::Xy2100E).lasers[0];
    let xy2 = channels_for(Protocol::Xy2100E, laser);
    let sl2 = channels_for(Protocol::Sl2100, laser);
    let has = |channels: &[build_log_model::ChannelDescriptor], kind: ChannelKind| {
        channels.iter().any(|c| c.kind == kind)
    };
    // Actual Z exists on SL2-100 only
    assert!(
        !has(&xy2, ChannelKind::ActualPosition { axis: Axis::Z })
            && has(&sl2, ChannelKind::ActualPosition { axis: Axis::Z })
    );
    assert_eq!((xy2.len(), sl2.len()), (7, 8));
    // commanded positions are int16 on XY2-100E and int32 on SL2-100, and the captured width is its own fact
    let commanded = |channels: &[build_log_model::ChannelDescriptor]| {
        channels
            .iter()
            .find(|c| c.kind == ChannelKind::CommandedPosition { axis: Axis::X })
            .cloned()
            .expect("commanded X")
    };
    let (x, s) = (commanded(&xy2), commanded(&sl2));
    assert_eq!(
        (x.dtype.clone(), s.dtype.clone()),
        (
            ChannelDType::Numeric(NumericType::I16),
            ChannelDType::Numeric(NumericType::I32)
        )
    );
    assert_eq!(
        x.captured_width.map(|w| (w.bits, w.encoding)),
        Some((16, CapturedEncoding::OffsetBinary))
    );
    // the laser state is an enum whose labels the producer declared from tauri's own comment (1 is on, 0 is off)
    let state = xy2
        .iter()
        .find(|c| c.kind == ChannelKind::LaserState)
        .expect("the laser state");
    assert!(matches!(
        state.dtype,
        ChannelDType::Enum {
            labels: EnumLabels::Declared(_)
        }
    ));
    assert_eq!(state.dtype.labels().map(|l| l.len()), Some(2));
    // the raw units are not the producer's to state: the adapter of the version declares them (J7)
    assert!(xy2.iter().all(|c| !c.unit.is_known()));
}
