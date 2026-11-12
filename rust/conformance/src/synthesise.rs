//! The synthetic builder: a `BuildMeta` made straight from an expectation file.
//!
//! Until adapters exist this is the only producer of a CDM, and it is how the assertions are tested: a CDM built from an expectation must pass its assertions,
//! and the same CDM with one thing changed must fail them. It builds with the same public calls that an adapter uses.

use build_log_model::{
    BuildIdentity, BuildMeta, ByLayer, ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind,
    ClockClass, EnumLabels, EventPart, EventSeries, ImageFormat, LaserKey, LaserLayerMeta,
    LaserMeta, LayerId, LayerMeta, Presence, Quantity, TimeAxis, TimeValue, TimeWindow, Unit,
    UnitSource,
};
use serde_json::Value;

use crate::expectation::{
    array, field, float, int, opt_bool, opt_int, opt_text, text, Expectation,
};
use crate::names::{
    accuracy_from, clock_from, event_kind_from, event_type_from, join_status_from, kind_from,
    numeric_from, origin_from, outcome_from,
};

/// Builds the CDM that an expectation file describes.
pub fn synthesise(expectation: &Expectation) -> Result<BuildMeta, String> {
    let root = expectation.root();
    let build = field(root, "build")?;
    let identity = field(build, "identity")?;
    let mut meta = BuildMeta::new(BuildIdentity {
        build_id: opt_text(identity, "build_id")?,
        date: opt_text(identity, "date")?,
        username: opt_text(identity, "username")?,
        alloy: opt_text(identity, "alloy")?,
        part_count: opt_int(identity, "part_count")?,
        project_id: opt_text(identity, "project_id")?,
        source_mcf_name: opt_text(identity, "source_mcf_name")?,
        source_mcf_hash: opt_text(identity, "source_mcf_hash")?,
    });
    meta.layers_declared = opt_int(build, "layers_declared")?;
    meta.quality.outcome = outcome_from(build.get("outcome"))?;
    let provenance = field(build, "provenance")?;
    meta.provenance.opcua_join_status = join_status_from(provenance.get("opcua_join_status"))?;
    meta.provenance.opcua_join_version = opt_text(provenance, "opcua_join_version")?;
    crate::context::synthesise(build.get("context"), &mut meta)?;
    if let Some(thickness) = build.get("layer_thickness").filter(|v| !v.is_null()) {
        let spelling = opt_text(field(thickness, "unit")?, "spelling")?;
        meta.layer_thickness = Some(Quantity::from_file(
            float(thickness, "value")?,
            spelling.as_deref(),
        ));
    }

    for layer in array(root, "layers")? {
        // a layer with no group in the file is built from its lasers, below
        if layer.get("origin").and_then(Value::as_str) == Some("Inferred") {
            continue;
        }
        let id = LayerId(int(layer, "id")? as u32);
        let mut meta_layer = LayerMeta::new(id);
        meta_layer.start = opt_int(layer, "start")?.map(|v| TimeValue::from_file(v, None));
        meta_layer.end = opt_int(layer, "end")?.map(|v| TimeValue::from_file(v, None));
        // the two thicknesses, each on its own: absent, a placeholder that is not recorded, or a real value
        for (key, set) in [
            (
                "actual_thickness",
                LayerMeta::set_actual_thickness_from_file
                    as fn(&mut LayerMeta, f64, Option<&str>, &[f64]),
            ),
            (
                "commanded_thickness",
                LayerMeta::set_commanded_thickness_from_file,
            ),
        ] {
            match layer.get(key).filter(|v| !v.is_null()) {
                None => {}
                Some(thickness)
                    if thickness.get("origin").and_then(Value::as_str)
                        == Some("PlaceholderDropped") =>
                {
                    let raw: f64 = text(thickness, "raw")?
                        .parse()
                        .map_err(|_| "a placeholder thickness is not a number".to_string())?;
                    set(&mut meta_layer, raw, None, &[raw]);
                }
                Some(thickness) => set(&mut meta_layer, float(thickness, "value")?, None, &[]),
            }
        }
        meta.layers.insert(id, meta_layer);
    }

    for laser in array(root, "lasers")? {
        meta.add_laser(laser_from(laser)?);
    }
    meta.synthesise_missing_layers();

    for sensor in array(root, "sensors")? {
        meta.sensors.push(sensor_from(sensor)?);
    }
    for series in array(root, "events")? {
        meta.events.push(series_from(series)?);
    }
    Ok(meta)
}

fn presence_of(entry: &Value) -> Result<Presence, String> {
    match text(entry, "state")? {
        "recorded" | "recorded_empty" => Ok(Presence::from_count(int(entry, "samples")? as u64)),
        "not_recorded" => Ok(Presence::NotRecorded),
        other => Err(format!("`{other}` is not a presence")),
    }
}

fn laser_from(laser: &Value) -> Result<LaserMeta, String> {
    let key = LaserKey::new(text(laser, "key")?, int(laser, "number")? as u32);
    let mut meta = LaserMeta::new(key);
    meta.parameters = crate::parameters::synthesise(laser.get("parameters"))?;

    // (the layer, the channel's name, the name of its origin)
    let mut channel_origins: Vec<(LayerId, String, String)> = Vec::new();
    for channel in array(laser, "channels")? {
        let kind = kind_from(
            text(channel, "kind")?,
            opt_text(channel, "axis")?.as_deref(),
        )?;
        let source_name = text(channel, "source_name")?;
        let mut descriptor = if kind == ChannelKind::LaserState {
            ChannelDescriptor::laser_state(ChannelKey::new([source_name]), EnumLabels::Unknown)
        } else {
            let number = field(field(channel, "dtype")?, "Numeric")?
                .as_str()
                .ok_or_else(|| "a numeric channel names its type".to_string())?;
            ChannelDescriptor::new(
                ChannelKey::new([source_name]),
                kind,
                clock_from(text(channel, "clock_class")?)?,
                ChannelDType::Numeric(numeric_from(number)?),
            )
        };
        if let Some(unit) = channel.get("unit").filter(|v| !v.is_null()) {
            // declared by the adapter: no spelling in the file, only the id
            let id = opt_text(unit, "unit_id")?;
            descriptor = descriptor.with_unit(
                Unit {
                    id,
                    spelling: opt_text(unit, "spelling")?,
                },
                UnitSource::DeclaredByAdapter,
            );
        }
        let mut presence = ByLayer::new();
        for entry in array(channel, "layers")? {
            let id = LayerId(int(entry, "layer")? as u32);
            presence.insert(id, presence_of(entry)?);
            // D07-2: a channel that is empty because its dataspace is null says so, on the laser's entry for the layer (made below)
            if let Some(origin) = opt_text(entry, "origin")? {
                channel_origins.push((id, source_name.to_string(), origin));
            }
        }
        meta.channels.push(descriptor.with_presence(presence));
    }

    for entry in array(laser, "layers")? {
        let presence = presence_of(entry)?;
        if presence == Presence::NotRecorded {
            continue;
        }
        let mut layer = LaserLayerMeta::new(presence);
        layer.complete = opt_bool(entry, "complete")?;
        // D07-1: the origin of the stored samples where the channels of the stream differ in length
        if let Some(name) = opt_text(entry, "stored_samples_origin")? {
            layer.origins.record("stored_samples", origin_from(&name)?);
        }
        for (id, channel, name) in &channel_origins {
            if i64::from(id.0) == int(entry, "layer")? {
                layer.record_channel_origin(channel, origin_from(name)?);
            }
        }
        if let Some(inputs) = entry.get("time_axis_inputs").filter(|v| !v.is_null()) {
            let mut axis =
                TimeAxis::new().with_accuracy(accuracy_from(text(entry, "expected_accuracy")?)?);
            axis = axis.with_stored_samples(int(inputs, "stored_samples")? as u64);
            if let (Some(start), Some(end)) = (
                opt_int(inputs, "window_start")?,
                opt_int(inputs, "window_end")?,
            ) {
                axis = axis.with_window(TimeWindow {
                    start: TimeValue::from_file(start, None),
                    end: TimeValue::from_file(end, None),
                });
            }
            if let Some(period) = inputs.get("sample_period").filter(|v| !v.is_null()) {
                let spelling = opt_text(field(period, "unit")?, "spelling")?;
                axis = axis.with_period(Quantity::from_file(
                    float(period, "value")?,
                    spelling.as_deref(),
                ));
            }
            layer = layer.with_time_axis(axis);
        }
        meta.layers
            .insert(LayerId(int(entry, "layer")? as u32), layer);
    }
    Ok(meta)
}

fn sensor_from(sensor: &Value) -> Result<ChannelDescriptor, String> {
    let clock = clock_from(text(sensor, "clock_class")?)?;
    let kind = if clock == ClockClass::Image {
        ChannelKind::Image
    } else {
        ChannelKind::Sensor
    };
    let key = ChannelKey::new([text(sensor, "name")?]);
    let mut descriptor = match sensor.get("image").filter(|v| !v.is_null()) {
        // a camera channel that says its frame (IP-13 phase 13F)
        Some(image) => ChannelDescriptor::image(
            key,
            ImageFormat::new(
                int(image, "rows")? as u64,
                int(image, "columns")? as u64,
                numeric_from(text(image, "pixel")?)?,
            ),
        ),
        // the expectation says a sensor's identity, presence, unit and clock class and nothing else (decision D02-16), so the type is any
        None => ChannelDescriptor::new(
            key,
            kind,
            clock,
            ChannelDType::Numeric(build_log_model::NumericType::F64),
        ),
    };
    let unit = field(sensor, "unit")?;
    if let Some(spelling) = opt_text(unit, "spelling")? {
        let parsed = Unit::parse(&spelling);
        let source = if parsed.is_known() {
            UnitSource::FromFile
        } else {
            UnitSource::Unknown
        };
        descriptor = descriptor.with_unit(parsed, source);
    }
    let mut presence = ByLayer::new();
    for layer in array(sensor, "layers")? {
        let id = layer
            .as_i64()
            .ok_or_else(|| "a layer id is not an integer".to_string())?;
        // the expectation names the layers a sensor was recorded in, not how many samples: one stands for "recorded"
        presence.insert(LayerId(id as u32), Presence::from_count(1));
    }
    Ok(descriptor.with_presence(presence))
}

/// An event series as an expectation describes it, with the places it was recorded.
fn series_from(series: &Value) -> Result<EventSeries, String> {
    let path: Vec<String> = array(series, "path")?
        .iter()
        .map(|p| {
            p.as_str()
                .map(str::to_string)
                .ok_or_else(|| "a part of a path is not text".to_string())
        })
        .collect::<Result<_, _>>()?;
    let mut built = EventSeries::new(event_kind_from(text(series, "kind")?)?, path).with_interval(
        series
            .get("interval")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    );
    if let Some(declared) = opt_text(series, "data_type")? {
        built = built.with_data_type(event_type_from(&declared));
    }
    for place in array(series, "parts")? {
        let mut part = EventPart::new(int(place, "records")? as u64);
        if let Some(height) = place.get("height").and_then(Value::as_f64) {
            part = part.with_height(Quantity::from_file(height, None));
        }
        match field(place, "assignment")? {
            Value::String(word) if word == "unassigned" => built.no_layer = Some(part),
            other => {
                let id = other.as_u64().ok_or_else(|| {
                    "an assignment is neither `unassigned` nor a layer id".to_string()
                })?;
                built.layers.insert(LayerId(id as u32), part);
            }
        }
    }
    Ok(built)
}
