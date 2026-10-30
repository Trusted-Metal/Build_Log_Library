//! OPC UA event series, described: what each series is, how it is recorded, and where (IP-13 phase 13K; requirement BL-CD-15).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** telemetry.
//! * **Constructed by:** library (`EventSeries`, `EventPart`); the enums `EventKind` and `EventDataType`.
//! * **Requirement:** BL-CD-15.
//!
//! # What is here so far
//!
//! A **series** is one signal that the capture listened to: a sensor (a value now and then), a task (a command and its state, each record an interval with a start
//! and a stop and a height) or the state of the run. It is named by the path the OPC UA server gave it, in parts. A series is **recorded in places**: inside
//! layers, and in the **bucket that no layer owns** (what happened before the build, between layers and after it). Each place says how many records it holds and,
//! for a task, its height.
//!
//! **The series is described here and its records are not.** The values and the timestamps are columns, bulk data, reached through a column accessor (BL-CD-03):
//! `ColumnSource::read_events`, with the request and the result of the module `event_columns` (IP-06 phase 6F). Nothing here decodes a value: a V17 file holds them as
//! text and the library keeps them so.
//!
//! The pre-cleanup `EventRecord` (module `sensor`) keeps its name until phase 2T retires it, so the new types are named for the series.
//!
//! The policy guards in `tests/policy_guards.rs` scan this file from now on: every enum is `#[non_exhaustive]` or says `// closed: <reason>`, every public struct
//! says `// constructed-by: producer|library|caller-options` and follows that rule, and none holds bulk data.

use crate::identity::ByLayer;
use crate::provenance::Origins;
use crate::units::Quantity;

/// What an event series records.
// open-domain: files and servers may bring kinds of event that nobody has listed, and they are kept as the file names them
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum EventKind {
    /// A signal that was sampled when it changed: a value and the time it was taken.
    Sensor,
    /// A command and its state: each record has a start and a stop, and a height.
    Task,
    /// The state of the run (started, finished).
    Run,
    /// A kind that this library has no name for, as the file names it.
    Other(String),
}

impl EventKind {
    /// The kind as a word: `sensor`, `task` or `run`, and for a kind this library has no name for, the name the file gives it. It names the series in an error.
    pub fn name(&self) -> &str {
        match self {
            EventKind::Sensor => "sensor",
            EventKind::Task => "task",
            EventKind::Run => "run",
            EventKind::Other(name) => name,
        }
    }
}

/// The type that a series declares for its values, which the file holds as text. Only the words that are known are named, and they are matched exactly.
// open-domain: a server declares a type that nobody has listed, and it is kept as the file spells it
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventDataType {
    Double,
    Boolean,
    /// A type that this library has no name for, exactly as the file spells it.
    Other(String),
}

impl EventDataType {
    /// The type that a series declares, from the text the file gives. Only `Double` and `Boolean` are named, and they are matched exactly: any other text, in any
    /// case, is kept as written.
    pub fn from_declared(declared: &str) -> Self {
        match declared {
            "Double" => EventDataType::Double,
            "Boolean" => EventDataType::Boolean,
            other => EventDataType::Other(other.to_string()),
        }
    }
}

/// A place where a series was recorded: how many records it holds there and, for a task, its height. Nothing is read to say so: the count is the length of the
/// series' timestamps.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EventPart {
    /// How many records the series holds in this place.
    pub records: u64,
    /// The height the file gives for the series in this place (a task's), with a unit only if the file states one.
    pub height: Option<Quantity>,
    /// How a value came to be what it is, when that is not simply "the file said so", by the name of the value.
    pub origins: Origins,
}

impl EventPart {
    pub fn new(records: u64) -> Self {
        EventPart {
            records,
            height: None,
            origins: Origins::new(),
        }
    }

    pub fn with_height(mut self, height: Quantity) -> Self {
        self.height = Some(height);
        self
    }
}

/// One signal of the capture, described: its kind and path, the type it declares, whether each record is an interval, and where it was recorded.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EventSeries {
    pub kind: EventKind,
    /// The name the server gave the signal, in parts, from the outermost: such as a category, a node and a variable. It is the key of the series.
    pub path: Vec<String>,
    /// The type the series declares for its values; `None` when it declares none (the run state).
    pub data_type: Option<EventDataType>,
    /// Whether each record has a start and a stop (a task's), and not only the time it was taken.
    pub interval: bool,
    /// Where the series was recorded inside layers.
    pub layers: ByLayer<EventPart>,
    /// Where it was recorded in the bucket that no layer owns: before the build, between layers, after it.
    pub no_layer: Option<EventPart>,
}

impl EventSeries {
    /// A series with no data type, no interval and no places yet.
    pub fn new(kind: EventKind, path: Vec<String>) -> Self {
        EventSeries {
            kind,
            path,
            data_type: None,
            interval: false,
            layers: ByLayer::new(),
            no_layer: None,
        }
    }

    pub fn with_data_type(mut self, data_type: EventDataType) -> Self {
        self.data_type = Some(data_type);
        self
    }

    pub fn with_interval(mut self, interval: bool) -> Self {
        self.interval = interval;
        self
    }

    /// How many records the series holds in all, in every place.
    pub fn records(&self) -> u64 {
        self.layers
            .iter()
            .map(|(_, part)| part.records)
            .sum::<u64>()
            + self.no_layer.as_ref().map_or(0, |part| part.records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::LayerId;

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn a_series_counts_its_records_in_every_place_and_the_bucket_of_no_layer_is_its_own() {
        let mut series = EventSeries::new(EventKind::Sensor, path(&["a", "node", "variable"]))
            .with_data_type(EventDataType::Double);
        assert_eq!(series.records(), 0);
        series.layers.insert(LayerId(1), EventPart::new(4));
        series.layers.insert(LayerId(2), EventPart::new(6));
        series.no_layer = Some(EventPart::new(10));
        assert_eq!(series.records(), 20);
        // the bucket is not a layer: dropping it leaves the layers' records
        series.no_layer = None;
        assert_eq!(series.records(), 10);
        assert!(!series.interval);
    }

    #[test]
    fn a_task_keeps_its_interval_and_its_height_in_the_place_it_was_recorded() {
        let mut task = EventSeries::new(EventKind::Task, path(&["command", "node", "state"]))
            .with_interval(true);
        task.layers.insert(
            LayerId(7),
            EventPart::new(1).with_height(Quantity::from_file(0.5, None)),
        );
        assert!(task.interval);
        let part = task.layers.get(LayerId(7)).expect("the layer");
        assert_eq!(part.height.as_ref().map(|h| h.value), Some(0.5));
    }

    #[test]
    fn a_declared_type_is_named_only_for_the_two_words_and_exactly() {
        assert_eq!(
            EventDataType::from_declared("Double"),
            EventDataType::Double
        );
        assert_eq!(
            EventDataType::from_declared("Boolean"),
            EventDataType::Boolean
        );
        assert_eq!(
            EventDataType::from_declared("double"),
            EventDataType::Other("double".into())
        );
        assert_eq!(
            EventDataType::from_declared("Int32"),
            EventDataType::Other("Int32".into())
        );
    }

    #[test]
    fn a_kind_or_a_type_with_no_name_is_kept_as_the_file_spells_it() {
        assert_ne!(EventKind::Other("sensor".into()), EventKind::Sensor);
        assert_ne!(EventDataType::Other("double".into()), EventDataType::Double);
        // kinds order the series: sensors, then tasks, then the run
        assert!(EventKind::Sensor < EventKind::Task && EventKind::Task < EventKind::Run);
    }
}
