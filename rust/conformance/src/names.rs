//! The words of the expectation files and the CDM types they stand for, in both directions.

use build_log_model::{
    Accuracy, Axis, ChannelDType, ChannelKind, ClockClass, EnumLabels, EquationKind, EventDataType,
    EventKind, GalvoProtocol, JoinStatus, NumericType, Origin, Outcome, Presence,
};
use serde_json::Value;

use crate::expectation::opt_text;

pub(crate) fn axis_from(name: &str) -> Result<Axis, String> {
    match name {
        "X" => Ok(Axis::X),
        "Y" => Ok(Axis::Y),
        "Z" => Ok(Axis::Z),
        other => Err(format!("`{other}` is not an axis")),
    }
}

pub(crate) fn kind_from(kind: &str, axis: Option<&str>) -> Result<ChannelKind, String> {
    let axis = || {
        axis.ok_or_else(|| format!("the kind `{kind}` needs an axis"))
            .and_then(axis_from)
    };
    match kind {
        "LaserPower" => Ok(ChannelKind::LaserPower),
        "LaserState" => Ok(ChannelKind::LaserState),
        "ActualPosition" => Ok(ChannelKind::ActualPosition { axis: axis()? }),
        "CommandedPosition" => Ok(ChannelKind::CommandedPosition { axis: axis()? }),
        other => Err(format!(
            "`{other}` is not a channel kind of the expectation files"
        )),
    }
}

/// The kind and the axis, as the expectation files spell them.
pub(crate) fn kind_to(kind: &ChannelKind) -> (String, Option<String>) {
    match kind {
        ChannelKind::LaserPower => ("LaserPower".to_string(), None),
        ChannelKind::LaserState => ("LaserState".to_string(), None),
        ChannelKind::ActualPosition { axis } => {
            ("ActualPosition".to_string(), Some(format!("{axis:?}")))
        }
        ChannelKind::CommandedPosition { axis } => {
            ("CommandedPosition".to_string(), Some(format!("{axis:?}")))
        }
        ChannelKind::Sensor => ("Sensor".to_string(), None),
        ChannelKind::Image => ("Image".to_string(), None),
        ChannelKind::Other(name) => (format!("Other({name})"), None),
        _ => ("an unknown kind".to_string(), None),
    }
}

/// The origin that an expectation names (IP-07 phase 7A): the two that a laser's stream can carry, by the names of the library's variants.
pub(crate) fn origin_from(name: &str) -> Result<Origin, String> {
    match name {
        "LongestChannel" => Ok(Origin::LongestChannel),
        "NullDataspace" => Ok(Origin::NullDataspace),
        other => Err(format!(
            "`{other}` is not an origin that an expectation names"
        )),
    }
}

pub(crate) fn numeric_from(name: &str) -> Result<NumericType, String> {
    Ok(match name {
        "I8" => NumericType::I8,
        "I16" => NumericType::I16,
        "I32" => NumericType::I32,
        "I64" => NumericType::I64,
        "U8" => NumericType::U8,
        "U16" => NumericType::U16,
        "U32" => NumericType::U32,
        "U64" => NumericType::U64,
        "F32" => NumericType::F32,
        "F64" => NumericType::F64,
        other => return Err(format!("`{other}` is not a number type")),
    })
}

pub(crate) fn clock_from(name: &str) -> Result<ClockClass, String> {
    Ok(match name {
        "LaserSynchronous" => ClockClass::LaserSynchronous,
        "Timestamped" => ClockClass::Timestamped,
        "Event" => ClockClass::Event,
        "Static" => ClockClass::Static,
        "Image" => ClockClass::Image,
        other => return Err(format!("`{other}` is not a clock class")),
    })
}

pub(crate) fn accuracy_from(name: &str) -> Result<Accuracy, String> {
    Ok(match name {
        "nominal" => Accuracy::Nominal,
        "measured" => Accuracy::Measured,
        "unknown" => Accuracy::Unknown,
        other => return Err(format!("`{other}` is not an accuracy")),
    })
}

pub(crate) fn accuracy_to(accuracy: Accuracy) -> String {
    format!("{accuracy:?}").to_lowercase()
}

/// The type of a channel as the files spell it: `{"Numeric": "F32"}` is `F32` here, and an enum with unknown labels is `Enum(unknown labels)`.
pub(crate) fn dtype_to(dtype: &ChannelDType) -> String {
    match dtype {
        ChannelDType::Numeric(n) => format!("{n:?}"),
        ChannelDType::Bool => "Bool".to_string(),
        ChannelDType::Enum {
            labels: EnumLabels::Unknown,
        } => "Enum(unknown labels)".to_string(),
        ChannelDType::Enum { .. } => "Enum(declared labels)".to_string(),
        ChannelDType::Text => "Text".to_string(),
        _ => "an unknown type".to_string(),
    }
}

/// A presence as the files spell it: the state and, where there is one, the number of samples.
pub(crate) fn presence_to(presence: &Presence) -> (String, Option<u64>) {
    match presence {
        Presence::Recorded { samples } => ("recorded".to_string(), Some(samples.get())),
        Presence::RecordedEmpty => ("recorded_empty".to_string(), Some(0)),
        Presence::NotRecorded => ("not_recorded".to_string(), None),
        _ => ("an unknown state".to_string(), None),
    }
}

/// The build's outcome as an expectation file says it: `null` is not recorded; otherwise the name the oracle has for the status text, or the text as written.
pub(crate) fn outcome_from(status: Option<&Value>) -> Result<Outcome, String> {
    let Some(status) = status.filter(|v| !v.is_null()) else {
        return Ok(Outcome::NotRecorded);
    };
    match (
        opt_text(status, "named")?.as_deref(),
        opt_text(status, "other")?,
    ) {
        (Some("Complete"), _) => Ok(Outcome::Complete),
        (Some("InProgress"), _) => Ok(Outcome::InProgress),
        (Some(other), _) => Err(format!("`{other}` is not an outcome")),
        (None, Some(text)) => Ok(Outcome::Other(text)),
        (None, None) => Err("a status has neither a name nor a text".to_string()),
    }
}

/// The status of the OPC UA join as an expectation file says it: `null` is none.
pub(crate) fn join_status_from(status: Option<&Value>) -> Result<Option<JoinStatus>, String> {
    let Some(status) = status.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    match (
        opt_text(status, "named")?.as_deref(),
        opt_text(status, "other")?,
    ) {
        (Some("Complete"), _) => Ok(Some(JoinStatus::Complete)),
        (Some(other), _) => Err(format!("`{other}` is not a join status")),
        (None, Some(text)) => Ok(Some(JoinStatus::Other(text))),
        (None, None) => Err("a status has neither a name nor a text".to_string()),
    }
}

/// A galvo's protocol as an expectation file says it: the name the oracle has for the text, or the text as written.
pub(crate) fn protocol_from(
    named: Option<String>,
    other: Option<String>,
) -> Result<GalvoProtocol, String> {
    match (named.as_deref(), other) {
        (Some("Sl2100"), _) => Ok(GalvoProtocol::Sl2100),
        (Some("Xy2100"), _) => Ok(GalvoProtocol::Xy2100),
        (Some(name), _) => Err(format!("`{name}` is not a protocol")),
        (None, Some(text)) => Ok(GalvoProtocol::Other(text)),
        (None, None) => Err("a protocol has neither a name nor a text".to_string()),
    }
}

/// The kind of an equation as an expectation file says it.
pub(crate) fn equation_kind_from(
    kind: &str,
    other: Option<String>,
) -> Result<EquationKind, String> {
    match (kind, other) {
        ("Linear", _) => Ok(EquationKind::Linear),
        ("Polynomial", _) => Ok(EquationKind::Polynomial),
        ("Other", Some(text)) => Ok(EquationKind::Other(text)),
        (kind, _) => Err(format!("`{kind}` is not a kind of equation")),
    }
}

/// The kind of an event series as an expectation file spells it.
pub(crate) fn event_kind_from(kind: &str) -> Result<EventKind, String> {
    match kind {
        "sensor" => Ok(EventKind::Sensor),
        "task" => Ok(EventKind::Task),
        "run" => Ok(EventKind::Run),
        other => Err(format!("`{other}` is not a kind of event")),
    }
}

/// The kind of an event series as an expectation file spells it.
pub(crate) fn event_kind_to(kind: &EventKind) -> String {
    match kind {
        EventKind::Sensor => "sensor".to_string(),
        EventKind::Task => "task".to_string(),
        EventKind::Run => "run".to_string(),
        EventKind::Other(name) => format!("other: {name}"),
        _ => "an unknown kind".to_string(),
    }
}

/// The type a series declares, as an expectation file spells it: the word that is known, or the text as written.
pub(crate) fn event_type_from(declared: &str) -> EventDataType {
    match declared {
        "Double" => EventDataType::Double,
        "Boolean" => EventDataType::Boolean,
        other => EventDataType::Other(other.to_string()),
    }
}

pub(crate) fn event_type_to(declared: &EventDataType) -> String {
    match declared {
        EventDataType::Double => "Double".to_string(),
        EventDataType::Boolean => "Boolean".to_string(),
        EventDataType::Other(text) => text.clone(),
        _ => "an unknown type".to_string(),
    }
}
