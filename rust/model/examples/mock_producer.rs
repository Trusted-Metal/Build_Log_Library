//! A mock producer: what tauri would hand to the library when it starts a build (IP-02 phase 2M, task T02.27, question Q2-4).
//!
//! It is written from tauri's own code (`clearbox-tauri/src-tauri/src/`: `hdf5/writer.rs` for the configuration, `hdf5/streaming.rs` for the channels and their types, and
//! `capture/pipeline.rs` for the times; read on 2026-10-21 and **never modified**; the channels are those of tauri's streaming path, which is its default), and it is the *write-side* check
//! of the slice: the CDM must be able to hold what a producer has, not only what a file shows. `TauriConfig` is a small copy of the fields of tauri's `Hdf5Config`
//! and `LaserHdf5Config` that the slice takes; nothing else of tauri is used.
//!
//! Run it with `cargo run -p build-log-model --example mock_producer`. A test (`tests/mock_producer.rs`) builds the same values, so the example stays true.
//!
//! **What it shows** (the findings are in the review packet, `docs/models/slice-review.md`):
//! * `BuildIdentity` is built with a full struct literal, so tauri has to say `None` for what it does not have (`project_id`, `source_mcf_*`).
//! * tauri **formats the date itself** (`chrono::Utc::now()` as `%Y-%m-%d %H:%M:%S UTC+00:00`), so the CDM receives a string.
//! * XY2-100E and SL2-100 give different channels: commanded positions are `i16` on XY2 and `i32` on SL2, and `Actual_Z` exists on SL2 only.
//! * tauri knows the **captured width** of each axis (`x_commanded_bit_res` and its siblings), so `CapturedWidth` has a real source.
//! * tauri's own comment says laser state is `1 = on, 0 = off`, so a producer can **declare the labels**.

use build_log_model::{
    Accuracy, Axis, BuildIdentity, BuildMeta, ByLayer, CapturedEncoding, CapturedWidth,
    ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind, ClockClass, EnumLabel, EnumLabels,
    LaserKey, LaserLayerMeta, LaserMeta, LayerId, NumericType, Presence, Quantity, TimeAxis,
    TimeValue, TimeWindow,
};

/// The protocol of the capture (tauri's `Protocol`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    Xy2100E,
    Sl2100,
}

/// One laser's configuration (the fields of tauri's `LaserHdf5Config` that the slice takes). tauri keeps these **per laser, with per-axis values inside**.
pub struct TauriLaser {
    pub number: u32,
    /// `x_actual_bit_res`, `y_actual_bit_res` (and `z_` where there is a Z).
    pub actual_bits: u8,
    /// `x_commanded_bit_res` and its siblings.
    pub commanded_bits: u8,
    /// `Scanner_Card.Sample_Period`; tauri's comment says "typically 10.0 microseconds".
    pub sample_period: f32,
}

/// The build configuration (the fields of tauri's `Hdf5Config` that the slice takes).
pub struct TauriConfig {
    pub build_id: String,
    pub operator: String,
    pub alloy: String,
    pub num_of_parts: i32,
    pub layer_thickness: f32,
    /// tauri writes `chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC+00:00")` when it writes the configuration: the moment the file's configuration is written.
    pub date: String,
    pub protocol: Protocol,
    pub lasers: Vec<TauriLaser>,
}

/// A configuration of the kind the mock capture of 2026-10-16 shows: an XY2-100E machine with two lasers.
pub fn example_config(protocol: Protocol) -> TauriConfig {
    TauriConfig {
        build_id: "example-build".to_string(),
        operator: "operator".to_string(),
        // tauri writes an empty `Alloys` attribute when none is set; a producer that wants the same logical file passes `Some("")`
        alloy: String::new(),
        num_of_parts: 1,
        layer_thickness: 0.03,
        date: "2026-10-16 15:38:06 UTC+00:00".to_string(),
        protocol,
        lasers: vec![
            TauriLaser {
                number: 1,
                actual_bits: 18,
                commanded_bits: 16,
                sample_period: 10.0,
            },
            TauriLaser {
                number: 2,
                actual_bits: 18,
                commanded_bits: 16,
                sample_period: 10.0,
            },
        ],
    }
}

/// `BuildIdentity` from tauri's configuration. Every field is stated, so a new field of the CDM breaks this function and makes tauri decide (decision D02-2).
pub fn build_identity(config: &TauriConfig) -> BuildIdentity {
    BuildIdentity {
        build_id: config.build_id.clone(),
        date: Some(config.date.clone()),
        username: Some(config.operator.clone()),
        alloy: Some(config.alloy.clone()),
        part_count: Some(i64::from(config.num_of_parts)),
        // tauri writes none of these (requirement CT-UC-10)
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    }
}

fn key(segments: &[&str]) -> ChannelKey {
    ChannelKey::new(segments.iter().copied())
}

fn position(
    commanded: bool,
    axis: Axis,
    name: &str,
    stored: NumericType,
    bits: u8,
    encoding: CapturedEncoding,
) -> ChannelDescriptor {
    let kind = if commanded {
        ChannelKind::CommandedPosition { axis }
    } else {
        ChannelKind::ActualPosition { axis }
    };
    ChannelDescriptor::new(
        key(&[
            "position",
            if commanded { "commanded" } else { "actual" },
            name,
        ]),
        kind,
        ClockClass::LaserSynchronous,
        ChannelDType::Numeric(stored),
    )
    .with_captured_width(CapturedWidth { bits, encoding })
}

/// The channels of one laser, by protocol. The raw units (volts for the power, bits for the positions) are **not stated here**: the adapter of the version
/// declares them (decision J7), and the producer has nothing to add.
pub fn channels_for(protocol: Protocol, laser: &TauriLaser) -> Vec<ChannelDescriptor> {
    let mut channels = vec![
        // tauri: "Laser power (0-10V scaled) - stored as float"
        ChannelDescriptor::new(
            key(&["power"]),
            ChannelKind::LaserPower,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(NumericType::F32),
        ),
        // tauri: "Laser state (1 = on, 0 = off) - stored as uint8": the producer can declare the labels
        ChannelDescriptor::laser_state(
            key(&["state"]),
            EnumLabels::Declared(vec![
                EnumLabel {
                    value: 0,
                    name: "off".to_string(),
                },
                EnumLabel {
                    value: 1,
                    name: "on".to_string(),
                },
            ]),
        ),
    ];
    // commanded: int16 on XY2-100E, int32 on SL2-100; 16 bits in offset-binary form on XY2 (requirement CT-DA-04)
    let (commanded_type, commanded_encoding) = match protocol {
        Protocol::Xy2100E => (NumericType::I16, CapturedEncoding::OffsetBinary),
        Protocol::Sl2100 => (NumericType::I32, CapturedEncoding::TwosComplement),
    };
    for (axis, name) in [(Axis::X, "x"), (Axis::Y, "y"), (Axis::Z, "z")] {
        channels.push(position(
            true,
            axis,
            name,
            commanded_type,
            laser.commanded_bits,
            commanded_encoding,
        ));
    }
    // actual: always int32, X and Y on both protocols, and Z on SL2-100 only
    let axes: &[(Axis, &str)] = match protocol {
        Protocol::Xy2100E => &[(Axis::X, "x"), (Axis::Y, "y")],
        Protocol::Sl2100 => &[(Axis::X, "x"), (Axis::Y, "y"), (Axis::Z, "z")],
    };
    for (axis, name) in axes {
        channels.push(position(
            false,
            *axis,
            name,
            NumericType::I32,
            laser.actual_bits,
            CapturedEncoding::TwosComplement,
        ));
    }
    channels
}

/// One laser in one layer, as tauri has it when the layer ends: the samples it stored, and the window and period it knows. The times are the integers tauri
/// writes, with no unit (decision D02-19).
pub fn laser_layer(samples: u64, window: (i64, i64), sample_period: f32) -> LaserLayerMeta {
    let axis = TimeAxis::new()
        .with_accuracy(Accuracy::Nominal)
        .with_stored_samples(samples)
        .with_window(TimeWindow {
            start: TimeValue::from_file(window.0, None),
            end: TimeValue::from_file(window.1, None),
        })
        // tauri's `sample_period` is a bare number; its comment says microseconds, and the file records no unit
        .with_period(Quantity::from_file(f64::from(sample_period), None));
    LaserLayerMeta::new(Presence::from_count(samples)).with_time_axis(axis)
}

/// The metadata of a new build, as tauri could hand it over before the first layer: identity, lasers with their channels, and nothing recorded yet.
pub fn build_meta(config: &TauriConfig) -> BuildMeta {
    let mut meta = BuildMeta::new(build_identity(config));
    // tauri writes `Layer_Quantity` as 0 when it starts and updates it when it closes the file: the CDM holds what the file declares, not "planned" or "committed"
    meta.layers_declared = Some(0);
    meta.layer_thickness = Some(Quantity::from_file(f64::from(config.layer_thickness), None));
    for laser_config in &config.lasers {
        let mut laser = LaserMeta::new(LaserKey::new(
            laser_config.number.to_string(),
            laser_config.number,
        ));
        laser.channels = channels_for(config.protocol, laser_config);
        meta.add_laser(laser);
    }
    meta
}

/// Adds one recorded layer to every laser (what tauri knows when a layer ends). Every laser gets the same window, as in tauri's streaming path, where the laser
/// times are the layer's capture window (`start_time * 1000` and one end time taken after the layer's data was read).
pub fn add_layer(meta: &mut BuildMeta, layer: LayerId, samples: u64, window: (i64, i64)) {
    let numbers: Vec<u32> = meta.lasers().iter().map(|l| l.key.number).collect();
    for number in numbers {
        let mut laser = meta
            .lasers()
            .iter()
            .find(|l| l.key.number == number)
            .expect("a laser")
            .clone();
        laser
            .layers
            .insert(layer, laser_layer(samples, window, 10.0));
        let presence: ByLayer<Presence> = laser
            .layers
            .iter()
            .map(|(id, l)| (id, l.presence))
            .collect();
        for channel in &mut laser.channels {
            channel.presence = presence.clone();
        }
        meta.add_laser(laser);
    }
    meta.synthesise_missing_layers();
}

fn main() {
    for protocol in [Protocol::Xy2100E, Protocol::Sl2100] {
        let config = example_config(protocol);
        let mut meta = build_meta(&config);
        add_layer(
            &mut meta,
            LayerId(1),
            727_922,
            (1_792_165_086_845_000, 1_792_165_087_746_592),
        );
        println!(
            "{protocol:?}: build {} with {} lasers and {} layer(s)",
            meta.identity.build_id,
            meta.lasers().len(),
            meta.layers_present()
        );
        for laser in meta.lasers() {
            let names: Vec<String> = laser
                .channels
                .iter()
                .map(|c| format!("{:?}", c.kind))
                .collect();
            println!(
                "  laser {}: {} channels: {}",
                laser.key,
                laser.channels.len(),
                names.join(", ")
            );
        }
    }
}
