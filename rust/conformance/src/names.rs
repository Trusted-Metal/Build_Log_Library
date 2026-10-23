//! The words of the expectation files and the CDM types they stand for, in both directions.

use build_log_model::{
    Accuracy, Axis, ChannelDType, ChannelKind, ClockClass, EnumLabels, NumericType, Presence,
};

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
