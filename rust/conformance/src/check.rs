//! The assertions: what a `BuildMeta` must show to conform to an expectation file.
//!
//! [`check`] returns every disagreement, as a line that says what was found and what was expected, and an empty list when the CDM conforms. It says what
//! the CDM must show, not how an adapter reads it, so any adapter of any version is held to the same facts.

use build_log_model::{
    BuildMeta, ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind, LaserMeta, LayerId,
    Origin, Presence, Quantity, Renderability, TimeAxis, TimeValue, Unit, UnitSource,
};
use serde_json::Value;

use crate::expectation::{array, field, int, opt_bool, opt_int, opt_text, text, Expectation};
use crate::names::{
    accuracy_to, dtype_to, event_kind_to, event_type_to, join_status_from, kind_from, kind_to,
    numeric_from, origin_from, outcome_from, presence_to,
};

/// One part of the whole check. An adapter that is built in steps asserts the parts it implements so far (IP-13 decision D13-9); [`check`] is all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Part {
    /// The identity of the build, the declared layer count, the build's layer thickness, its outcome and its provenance.
    Build,
    /// The build's context: the machine, the material, the starting height and the build plate, each value by name, the placeholders that are not recorded, and the plate vocabulary.
    Context,
    /// The layers the file holds: their ids, times and thickness.
    Layers,
    /// The lasers: their presence in each layer, `complete` flags, time axes and channels.
    Lasers,
    /// The parameter snapshot of each laser: its values by name, its equation and whether the power limits are not recorded as numbers.
    Parameters,
    /// The build-level sensors and the camera images.
    Sensors,
    /// The OPC UA event series: their kinds, paths and types, and where each was recorded, with the number of records.
    Events,
}

impl Part {
    /// Every part, in the order that [`check`] reports them.
    pub const ALL: [Part; 7] = [
        Part::Build,
        Part::Context,
        Part::Layers,
        Part::Lasers,
        Part::Parameters,
        Part::Sensors,
        Part::Events,
    ];
}

/// Every way in which `meta` does not conform to `expectation`. Empty when it conforms.
pub fn check(meta: &BuildMeta, expectation: &Expectation) -> Vec<String> {
    check_parts(meta, expectation, &Part::ALL)
}

/// The same for the parts named only, reported in the order of [`Part::ALL`] whatever order they are given in. The parts together are the whole: the lines
/// of [`check`] are the lines of each part, one after the other.
pub fn check_parts(meta: &BuildMeta, expectation: &Expectation, parts: &[Part]) -> Vec<String> {
    let mut found = Vec::new();
    if let Err(problem) = run(meta, expectation, parts, &mut found) {
        found.push(format!("the expectation file cannot be read: {problem}"));
    }
    found
}

/// Notes a disagreement when `actual` is not `expected`.
pub(crate) fn same<T: PartialEq + std::fmt::Debug>(
    found: &mut Vec<String>,
    what: &str,
    actual: T,
    expected: T,
) {
    if actual != expected {
        found.push(format!("{what}: found {actual:?}, expected {expected:?}"));
    }
}

fn run(
    meta: &BuildMeta,
    expectation: &Expectation,
    parts: &[Part],
    found: &mut Vec<String>,
) -> Result<(), String> {
    let root = expectation.root();
    if parts.contains(&Part::Build) {
        build_level(meta, field(root, "build")?, found)?;
    }
    if parts.contains(&Part::Context) {
        crate::context::check(meta, field(root, "build")?.get("context"), found)?;
    }
    if parts.contains(&Part::Layers) {
        layers(meta, array(root, "layers")?, found)?;
    }
    if parts.contains(&Part::Lasers) {
        lasers(meta, array(root, "lasers")?, found)?;
    }
    if parts.contains(&Part::Parameters) {
        for laser in array(root, "lasers")? {
            let number = int(laser, "number")? as u32;
            if let Some(actual) = meta.lasers().iter().find(|l| l.key.number == number) {
                crate::parameters::check_laser(actual, laser.get("parameters"), found)?;
            }
        }
    }
    if parts.contains(&Part::Sensors) {
        sensors(meta, array(root, "sensors")?, found)?;
    }
    if parts.contains(&Part::Events) {
        events(meta, array(root, "events")?, found)?;
    }
    Ok(())
}

/// The build level: the identity, the declared layer count, the build's layer thickness, its outcome and its provenance.
fn build_level(meta: &BuildMeta, build: &Value, found: &mut Vec<String>) -> Result<(), String> {
    identity(meta, field(build, "identity")?, found)?;
    same(
        found,
        "the build's outcome",
        meta.quality.outcome.clone(),
        outcome_from(build.get("outcome"))?,
    );
    let provenance = field(build, "provenance")?;
    same(
        found,
        "the status of the OPC UA join",
        meta.provenance.opcua_join_status.clone(),
        join_status_from(provenance.get("opcua_join_status"))?,
    );
    same(
        found,
        "the version of the OPC UA join",
        meta.provenance.opcua_join_version.clone(),
        opt_text(provenance, "opcua_join_version")?,
    );
    same(
        found,
        "the declared layer count",
        meta.layers_declared,
        opt_int(build, "layers_declared")?,
    );
    match build.get("layer_thickness").filter(|v| !v.is_null()) {
        None => same(
            found,
            "the build's layer thickness",
            meta.layer_thickness.is_some(),
            false,
        ),
        Some(expected) => match &meta.layer_thickness {
            None => found.push("the build's layer thickness: not there, expected one".to_string()),
            Some(thickness) => {
                same(
                    found,
                    "the build's layer thickness",
                    thickness.value,
                    field(expected, "value")?.as_f64().unwrap_or(f64::NAN),
                );
                unit(
                    found,
                    "the unit of the build's layer thickness",
                    &thickness.unit,
                    thickness.unit_source,
                    field(expected, "unit")?,
                )?;
            }
        },
    }
    Ok(())
}

fn identity(meta: &BuildMeta, expected: &Value, found: &mut Vec<String>) -> Result<(), String> {
    let id = &meta.identity;
    same(
        found,
        "the build id",
        Some(id.build_id.clone()),
        opt_text(expected, "build_id")?,
    );
    same(
        found,
        "the build date (a string, as written)",
        id.date.clone(),
        opt_text(expected, "date")?,
    );
    same(
        found,
        "the username",
        id.username.clone(),
        opt_text(expected, "username")?,
    );
    same(
        found,
        "the alloy",
        id.alloy.clone(),
        opt_text(expected, "alloy")?,
    );
    same(
        found,
        "the part count",
        id.part_count,
        opt_int(expected, "part_count")?,
    );
    same(
        found,
        "the project id",
        id.project_id.clone(),
        opt_text(expected, "project_id")?,
    );
    same(
        found,
        "the configuration name",
        id.source_mcf_name.clone(),
        opt_text(expected, "source_mcf_name")?,
    );
    same(
        found,
        "the configuration hash",
        id.source_mcf_hash.clone(),
        opt_text(expected, "source_mcf_hash")?,
    );
    Ok(())
}

/// A unit against `{spelling, unit_id, source}`.
pub(crate) fn unit(
    found: &mut Vec<String>,
    what: &str,
    actual: &Unit,
    source: UnitSource,
    expected: &Value,
) -> Result<(), String> {
    same(
        found,
        &format!("{what}, its spelling"),
        actual.spelling.clone(),
        opt_text(expected, "spelling")?,
    );
    same(
        found,
        &format!("{what}, its id"),
        actual.id.clone(),
        opt_text(expected, "unit_id")?,
    );
    same(
        found,
        &format!("{what}, where it came from"),
        format!("{source:?}"),
        text(expected, "source")?.to_string(),
    );
    Ok(())
}

/// A time as written: the integer, and no unit (decision D02-19).
fn time(found: &mut Vec<String>, what: &str, actual: Option<&TimeValue>, expected: Option<i64>) {
    same(found, what, actual.map(|t| t.value), expected);
    if let Some(t) = actual {
        if t.unit_source != UnitSource::Unknown || t.unit.spelling.is_some() {
            found.push(format!(
                "{what}: no file states its unit, but the CDM reports {:?}",
                t.unit_source
            ));
        }
    }
}

fn layers(meta: &BuildMeta, expected: &[Value], found: &mut Vec<String>) -> Result<(), String> {
    let wanted: Vec<u32> = expected
        .iter()
        .map(|l| int(l, "id").map(|v| v as u32))
        .collect::<Result<_, _>>()?;
    same(
        found,
        "the layers present",
        meta.layers.ids().map(|id| id.0).collect::<Vec<_>>(),
        wanted,
    );
    for layer in expected {
        let id = LayerId(int(layer, "id")? as u32);
        let Some(actual) = meta.layers.get(id) else {
            continue;
        };
        let what = |part: &str| format!("layer {id}, {part}");
        time(
            found,
            &what("its start"),
            actual.start.as_ref(),
            opt_int(layer, "start")?,
        );
        time(
            found,
            &what("its end"),
            actual.end.as_ref(),
            opt_int(layer, "end")?,
        );
        // the two thicknesses are two facts, each with its own origin
        thickness(
            found,
            &what("its actual thickness"),
            actual.actual_thickness.as_ref(),
            actual.origins.get("actual_thickness"),
            layer.get("actual_thickness"),
        )?;
        thickness(
            found,
            &what("its commanded thickness"),
            actual.commanded_thickness.as_ref(),
            actual.origins.get("commanded_thickness"),
            layer.get("commanded_thickness"),
        )?;
        let inferred = layer.get("origin").and_then(Value::as_str) == Some("Inferred");
        same(
            found,
            &what("whether it was built from its lasers"),
            actual.origins.get("layer") == Some(&Origin::Inferred),
            inferred,
        );
    }
    Ok(())
}

/// One of a layer's two thicknesses against its expectation: absent (the file has no such attribute), a placeholder that is not recorded (with its raw
/// value as its origin), or a real value with its unit.
fn thickness(
    found: &mut Vec<String>,
    what: &str,
    actual: Option<&Quantity>,
    origin: Option<&Origin>,
    expected: Option<&Value>,
) -> Result<(), String> {
    match expected.filter(|v| !v.is_null()) {
        None => {
            same(found, what, actual.is_some(), false);
            same(found, &format!("{what}, its origin"), origin.cloned(), None);
        }
        Some(e) if e.get("origin").and_then(Value::as_str) == Some("PlaceholderDropped") => {
            same(found, what, actual.is_some(), false);
            let raw = Origin::PlaceholderDropped {
                raw: text(e, "raw")?.to_string(),
            };
            same(
                found,
                &format!("{what}, its origin"),
                origin.cloned(),
                Some(raw),
            );
        }
        Some(e) => {
            same(
                found,
                what,
                actual.map(|t| t.value),
                e.get("value").and_then(Value::as_f64),
            );
            if let Some(t) = actual {
                unit(
                    found,
                    &format!("{what}, its unit"),
                    &t.unit,
                    t.unit_source,
                    field(e, "unit")?,
                )?;
            }
        }
    }
    Ok(())
}

fn lasers(meta: &BuildMeta, expected: &[Value], found: &mut Vec<String>) -> Result<(), String> {
    let wanted: Vec<(String, u32)> = expected
        .iter()
        .map(|l| Ok((text(l, "key")?.to_string(), int(l, "number")? as u32)))
        .collect::<Result<_, String>>()?;
    let actual: Vec<(String, u32)> = meta
        .lasers()
        .iter()
        .map(|l| (l.key.key.clone(), l.key.number))
        .collect();
    same(found, "the lasers, in numeric order", actual, wanted);
    for laser in expected {
        let number = int(laser, "number")? as u32;
        let Some(actual) = meta.lasers().iter().find(|l| l.key.number == number) else {
            continue;
        };
        laser_layers(actual, array(laser, "layers")?, found)?;
        channels(actual, array(laser, "channels")?, found)?;
    }
    Ok(())
}

fn laser_layers(
    laser: &LaserMeta,
    expected: &[Value],
    found: &mut Vec<String>,
) -> Result<(), String> {
    for entry in expected {
        let id = LayerId(int(entry, "layer")? as u32);
        let what = |part: &str| format!("laser {} in layer {id}, {part}", laser.key);
        let wanted_state = (
            text(entry, "state")?.to_string(),
            opt_int(entry, "samples")?.map(|v| v as u64),
        );
        let Some(actual) = laser.layers.get(id) else {
            same(
                found,
                &what("its presence"),
                ("not_recorded".to_string(), None),
                wanted_state,
            );
            continue;
        };
        same(
            found,
            &what("its presence"),
            presence_to(&actual.presence),
            wanted_state,
        );
        same(
            found,
            &what("its complete flag"),
            actual.complete,
            opt_bool(entry, "complete")?,
        );
        // D07-1: where the channels of the stream differ in length, its stored samples carry the origin `LongestChannel`; where they agree there is none
        same(
            found,
            &what("where its stored samples came from"),
            actual.origins.get("stored_samples").cloned(),
            opt_text(entry, "stored_samples_origin")?
                .map(|name| origin_from(&name))
                .transpose()?,
        );
        let Some(inputs) = entry.get("time_axis_inputs").filter(|v| !v.is_null()) else {
            continue;
        };
        let axis: &TimeAxis = &actual.time_axis;
        same(
            found,
            &what("its accuracy"),
            accuracy_to(axis.accuracy),
            text(entry, "expected_accuracy")?.to_string(),
        );
        same(
            found,
            &what("its stored samples"),
            axis.stored_samples,
            opt_int(inputs, "stored_samples")?.map(|v| v as u64),
        );
        time(
            found,
            &what("the start of its window"),
            axis.window.as_ref().map(|w| &w.start),
            opt_int(inputs, "window_start")?,
        );
        time(
            found,
            &what("the end of its window"),
            axis.window.as_ref().map(|w| &w.end),
            opt_int(inputs, "window_end")?,
        );
        match inputs.get("sample_period").filter(|v| !v.is_null()) {
            None => same(
                found,
                &what("its sample period"),
                axis.period.is_some(),
                false,
            ),
            Some(period) => match &axis.period {
                None => found.push(format!(
                    "{}: not there, expected one",
                    what("its sample period")
                )),
                Some(actual) => {
                    same(
                        found,
                        &what("its sample period"),
                        actual.value,
                        period
                            .get("value")
                            .and_then(Value::as_f64)
                            .unwrap_or(f64::NAN),
                    );
                    unit(
                        found,
                        &what("the unit of its sample period"),
                        &actual.unit,
                        actual.unit_source,
                        field(period, "unit")?,
                    )?;
                }
            },
        }
    }
    let extra: Vec<u32> = laser
        .layers
        .ids()
        .filter(|id| {
            !expected
                .iter()
                .any(|e| int(e, "layer").is_ok_and(|v| v == i64::from(id.0)))
        })
        .map(|id| id.0)
        .collect();
    same(
        found,
        &format!(
            "laser {}, layers that the expectation does not name",
            laser.key
        ),
        extra,
        Vec::new(),
    );
    Ok(())
}

fn channels(laser: &LaserMeta, expected: &[Value], found: &mut Vec<String>) -> Result<(), String> {
    let mut matched: Vec<&ChannelDescriptor> = Vec::new();
    for channel in expected {
        let kind = kind_from(
            text(channel, "kind")?,
            opt_text(channel, "axis")?.as_deref(),
        )?;
        let (name, axis) = kind_to(&kind);
        let what = |part: &str| {
            format!(
                "laser {}, channel {name}{}, {part}",
                laser.key,
                axis.as_ref().map(|a| format!(" {a}")).unwrap_or_default()
            )
        };
        let Some(actual) = laser.channels.iter().find(|c| c.kind == kind) else {
            found.push(format!(
                "laser {}, channel {name}: not described",
                laser.key
            ));
            continue;
        };
        matched.push(actual);
        same(
            found,
            &what("its clock class"),
            format!("{:?}", actual.clock_class),
            text(channel, "clock_class")?.to_string(),
        );
        let wanted_dtype = match channel.get("dtype") {
            Some(Value::String(s)) if s == "Enum" => "Enum(unknown labels)".to_string(),
            Some(d) => d
                .get("Numeric")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            None => "?".to_string(),
        };
        same(
            found,
            &what("its type"),
            dtype_to(&actual.dtype),
            wanted_dtype,
        );
        match channel.get("unit").filter(|v| !v.is_null()) {
            Some(expected_unit) => unit(
                found,
                &what("its unit"),
                &actual.unit,
                actual.unit_source,
                expected_unit,
            )?,
            None => same(
                found,
                &what("where its unit came from"),
                actual.unit_source,
                UnitSource::Unknown,
            ),
        }
        same(
            found,
            &what("its captured width"),
            actual.captured_width.is_some(),
            false,
        );
        same(
            found,
            &what("whether it can be drawn"),
            actual.renderability == Renderability::Renderable,
            true,
        );
        let wanted: Vec<(u32, String, Option<u64>)> = array(channel, "layers")?
            .iter()
            .map(|e| {
                Ok((
                    int(e, "layer")? as u32,
                    text(e, "state")?.to_string(),
                    opt_int(e, "samples")?.map(|v| v as u64),
                ))
            })
            .collect::<Result<_, String>>()?;
        let actual_presence: Vec<(u32, String, Option<u64>)> = actual
            .presence
            .iter()
            .map(|(id, p)| {
                let (state, samples) = presence_to(p);
                (id.0, state, samples)
            })
            .collect();
        same(
            found,
            &what("its presence by layer"),
            actual_presence,
            wanted,
        );
        // D07-2: a channel that is recorded and empty because its dataspace is null has the origin `NullDataspace`, on the laser's entry for the layer; every other channel has none there
        let source = text(channel, "source_name")?;
        for e in array(channel, "layers")? {
            let id = LayerId(int(e, "layer")? as u32);
            same(
                found,
                &what(&format!("where it came from in layer {id}")),
                laser
                    .layers
                    .get(id)
                    .and_then(|entry| entry.origins.get(&format!("channel {source}")).cloned()),
                opt_text(e, "origin")?
                    .map(|name| origin_from(&name))
                    .transpose()?,
            );
        }
    }
    for channel in &laser.channels {
        if !matched.iter().any(|m| std::ptr::eq(*m, channel)) {
            found.push(format!(
                "laser {}, channel {:?}: the expectation does not describe it",
                laser.key, channel.kind
            ));
        }
    }
    Ok(())
}

fn sensors(meta: &BuildMeta, expected: &[Value], found: &mut Vec<String>) -> Result<(), String> {
    let wanted: Vec<String> = expected
        .iter()
        .map(|s| text(s, "name").map(str::to_string))
        .collect::<Result<_, _>>()?;
    let actual: Vec<String> = meta.sensors.iter().map(|s| s.key.to_string()).collect();
    let mut sorted_actual = actual.clone();
    sorted_actual.sort();
    same(found, "the sensors (identity)", sorted_actual, wanted);
    for sensor in expected {
        let name = text(sensor, "name")?;
        let Some(actual) = meta
            .sensors
            .iter()
            .find(|s| s.key == ChannelKey::new([name]))
        else {
            continue;
        };
        let what = |part: &str| format!("sensor {name}, {part}");
        same(
            found,
            &what("its clock class"),
            format!("{:?}", actual.clock_class),
            text(sensor, "clock_class")?.to_string(),
        );
        unit(
            found,
            &what("its unit"),
            &actual.unit,
            actual.unit_source,
            field(sensor, "unit")?,
        )?;
        // a camera channel says the frame it holds: the rows, the columns and the type of a pixel (IP-13 phase 13F). A sensor whose expectation has no image
        // description says nothing about its type (decision D02-16).
        if let Some(image) = sensor.get("image").filter(|v| !v.is_null()) {
            same(
                found,
                &what("its kind"),
                actual.kind.clone(),
                ChannelKind::Image,
            );
            let format = match &actual.dtype {
                ChannelDType::Image(format) => Some((format.rows, format.columns, format.pixel)),
                _ => None,
            };
            same(
                found,
                &what("its image format (rows, columns, pixel)"),
                format,
                Some((
                    int(image, "rows")? as u64,
                    int(image, "columns")? as u64,
                    numeric_from(text(image, "pixel")?)?,
                )),
            );
        }
        let wanted_layers: Vec<u32> = array(sensor, "layers")?
            .iter()
            .filter_map(Value::as_u64)
            .map(|v| v as u32)
            .collect();
        let layers: Vec<u32> = actual.presence.ids().map(|id| id.0).collect();
        same(
            found,
            &what("the layers it was recorded in"),
            layers,
            wanted_layers,
        );
        same(
            found,
            &what("whether every layer holds samples"),
            actual
                .presence
                .iter()
                .all(|(_, p)| matches!(p, Presence::Recorded { .. })),
            true,
        );
    }
    Ok(())
}

/// The places an expectation gives for a series, as (where, records, height): `unassigned` for the bucket that no layer owns, and the layer's id otherwise.
fn expected_parts(series: &Value) -> Result<Vec<(String, u64, Option<f64>)>, String> {
    let mut parts = Vec::new();
    for part in array(series, "parts")? {
        let assignment = match field(part, "assignment")? {
            Value::String(text) => text.clone(),
            other => other.as_u64().map(|id| id.to_string()).ok_or_else(|| {
                "an assignment is neither `unassigned` nor a layer id".to_string()
            })?,
        };
        parts.push((
            assignment,
            int(part, "records")? as u64,
            part.get("height").and_then(Value::as_f64),
        ));
    }
    Ok(parts)
}

/// The OPC UA event series: the series themselves (by kind and path), then each one's declared type, its interval and the places it was recorded.
fn events(meta: &BuildMeta, expected: &[Value], found: &mut Vec<String>) -> Result<(), String> {
    let name_of = |kind: String, path: &[String]| format!("{kind} {}", path.join("/"));
    let mut wanted: Vec<String> = Vec::new();
    for series in expected {
        let path: Vec<String> = array(series, "path")?
            .iter()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect();
        wanted.push(name_of(text(series, "kind")?.to_string(), &path));
    }
    let mut actual: Vec<String> = meta
        .events
        .iter()
        .map(|s| name_of(event_kind_to(&s.kind), &s.path))
        .collect();
    actual.sort();
    wanted.sort();
    same(found, "the event series (kind and path)", actual, wanted);
    for series in expected {
        let path: Vec<String> = array(series, "path")?
            .iter()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect();
        let kind = text(series, "kind")?;
        let Some(actual) = meta
            .events
            .iter()
            .find(|s| event_kind_to(&s.kind) == kind && s.path == path)
        else {
            continue;
        };
        let what = |part: &str| format!("event {kind} {}, {part}", path.join("/"));
        same(
            found,
            &what("its declared type"),
            actual.data_type.as_ref().map(event_type_to),
            opt_text(series, "data_type")?,
        );
        same(
            found,
            &what("whether each record is an interval"),
            actual.interval,
            opt_bool(series, "interval")?.unwrap_or(false),
        );
        let mut places: Vec<(String, u64, Option<f64>)> = Vec::new();
        if let Some(part) = &actual.no_layer {
            places.push((
                "unassigned".to_string(),
                part.records,
                part.height.as_ref().map(|h| h.value),
            ));
        }
        for (id, part) in actual.layers.iter() {
            places.push((
                id.0.to_string(),
                part.records,
                part.height.as_ref().map(|h| h.value),
            ));
        }
        same(
            found,
            &what("where it was recorded, with the records and the height"),
            places,
            expected_parts(series)?,
        );
    }
    Ok(())
}
