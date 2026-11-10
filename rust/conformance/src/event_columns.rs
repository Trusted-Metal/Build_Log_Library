//! The fingerprint of the columns of an event place, and the check of a column source against the fingerprints of an expectation (IP-06 phase 6F, task T06.6,
//! decision D06-12).
//!
//! A V17 expectation file carries, for each place where an OPC UA series was recorded (a layer, or the bucket that no layer owns), a **fingerprint of its columns**, written by the
//! independent script from the miniature alone: the count, the first and the last timestamp, and the SHA-256 of the timestamps (their little-endian bytes), of the values (each as
//! UTF-8 followed by one zero byte) and, for a task, of its start and its stop. [`check_event_columns`] reads each of those places from a [`ColumnSource`] in full and says every way in
//! which what it got is not what the fingerprint says, and every way in which the typed view disagrees with the type the series declares.
//!
//! * The fingerprint is computed here from an [`EventColumns`] **in the same way the script computes it from the file**; a test holds both to the same literals.
//! * A place that holds no values column has no hash of values, and a source that serves one for it (or none for a place that has it) is reported.
//! * The expectation holds no value and no time but the first and the last, so that the events stay bulk data.

use build_log_model::{
    ColumnSource, EventColumns, EventDataType, EventKind, EventPlace, EventRequest, LayerId,
    ReadContext, TypedValues,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::expectation::{array, field, opt_text, text};
use crate::names::{event_kind_from, event_kind_to, event_type_from};
use crate::Expectation;

/// How many timestamps are hashed at a time.
const HASH_BLOCK: usize = 8192;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of numbers of nanoseconds as little-endian bytes, whatever machine this runs on.
fn sha_of_times(values: &[u64]) -> String {
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity(HASH_BLOCK * 8);
    for block in values.chunks(HASH_BLOCK) {
        bytes.clear();
        for value in block {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        hasher.update(&bytes);
    }
    hex(&hasher.finalize())
}

/// The SHA-256 of values as text: each as its UTF-8 bytes followed by one zero byte, in order.
fn sha_of_texts(values: &[String]) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.as_bytes());
        hasher.update([0u8]);
    }
    hex(&hasher.finalize())
}

/// What the columns of one place of an event series hold, in a few numbers and some hashes.
#[derive(Debug, Clone, PartialEq)]
pub struct EventFingerprint {
    /// The number of timestamps, which every column of the place shares.
    pub count: u64,
    pub first: Option<u64>,
    pub last: Option<u64>,
    pub timestamps_sha256: String,
    /// `None` for a place that holds no values column (a task's).
    pub values_sha256: Option<String>,
    /// `None` for a place with no interval.
    pub start_sha256: Option<String>,
    pub stop_sha256: Option<String>,
}

impl EventFingerprint {
    /// The fingerprint of the columns a read returned.
    pub fn of(columns: &EventColumns) -> EventFingerprint {
        let times = columns.timestamps.as_slice();
        EventFingerprint {
            count: times.len() as u64,
            first: times.first().copied(),
            last: times.last().copied(),
            timestamps_sha256: sha_of_times(times),
            values_sha256: columns
                .values
                .as_ref()
                .map(|values| sha_of_texts(values.as_slice())),
            start_sha256: columns
                .start
                .as_ref()
                .map(|start| sha_of_times(start.as_slice())),
            stop_sha256: columns
                .stop
                .as_ref()
                .map(|stop| sha_of_times(stop.as_slice())),
        }
    }

    /// The fingerprint as an expectation file holds it, for a test that builds an expectation of its own.
    pub fn to_json(&self) -> Value {
        json!({
            "count": self.count,
            "first": self.first,
            "last": self.last,
            "timestamps_sha256": self.timestamps_sha256,
            "values_sha256": self.values_sha256,
            "start_sha256": self.start_sha256,
            "stop_sha256": self.stop_sha256,
        })
    }

    fn from_json(value: &Value) -> Result<EventFingerprint, String> {
        let number = |key: &str| -> Result<Option<u64>, String> {
            match value.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_u64()
                    .map(Some)
                    .ok_or_else(|| format!("`{key}` is not a whole number or null")),
            }
        };
        Ok(EventFingerprint {
            count: number("count")?.ok_or("the fingerprint has no `count`")?,
            first: number("first")?,
            last: number("last")?,
            timestamps_sha256: opt_text(value, "timestamps_sha256")?
                .ok_or("the fingerprint has no `timestamps_sha256`")?,
            values_sha256: opt_text(value, "values_sha256")?,
            start_sha256: opt_text(value, "start_sha256")?,
            stop_sha256: opt_text(value, "stop_sha256")?,
        })
    }
}

/// One place of an event series that an expectation describes: which series and where, what it declares, and the fingerprint of its columns.
#[derive(Debug, Clone, PartialEq)]
pub struct EventExpectation {
    pub kind: EventKind,
    pub path: Vec<String>,
    pub place: EventPlace,
    pub data_type: Option<EventDataType>,
    pub interval: bool,
    pub fingerprint: EventFingerprint,
}

impl EventExpectation {
    /// Which place, for a message: `sensor Actuators/node/variable in layer 2`, or `... in the bucket that no layer owns`.
    fn whose(&self) -> String {
        let place = match self.place {
            EventPlace::Layer(LayerId(id)) => format!("layer {id}"),
            _ => "the bucket that no layer owns".to_string(),
        };
        format!(
            "event {} {} in {place}",
            event_kind_to(&self.kind),
            self.path.join("/")
        )
    }
}

/// Every place that an expectation describes. **Every place of a series must carry its fingerprint**: one that does not is an error, so that a check cannot pass by having
/// nothing to check. An expectation of a version with no events describes none.
pub fn event_expectations(expectation: &Expectation) -> Result<Vec<EventExpectation>, String> {
    let mut found = Vec::new();
    for series in array(expectation.root(), "events")? {
        let kind = event_kind_from(text(series, "kind")?)?;
        let path: Vec<String> = array(series, "path")?
            .iter()
            .map(|p| {
                p.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "a part of a path is not text".to_string())
            })
            .collect::<Result<_, _>>()?;
        let data_type = opt_text(series, "data_type")?.map(|declared| event_type_from(&declared));
        let interval = series
            .get("interval")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        for part in array(series, "parts")? {
            let place = match field(part, "assignment")? {
                Value::String(word) if word == "unassigned" => EventPlace::NoLayer,
                other => EventPlace::Layer(LayerId(other.as_u64().ok_or_else(|| {
                    "an assignment is neither `unassigned` nor a layer id".to_string()
                })? as u32)),
            };
            let who = format!("event {} {}", event_kind_to(&kind), path.join("/"));
            let columns = match part.get("columns") {
                Some(columns) if !columns.is_null() => columns,
                _ => {
                    return Err(format!(
                        "{who}: the expectation has no fingerprint for a place of a series"
                    ))
                }
            };
            found.push(EventExpectation {
                kind: kind.clone(),
                path: path.clone(),
                place,
                data_type: data_type.clone(),
                interval,
                fingerprint: EventFingerprint::from_json(columns)
                    .map_err(|e| format!("{who}: {e}"))?,
            });
        }
    }
    Ok(found)
}

fn differences(
    who: &str,
    actual: &EventFingerprint,
    wanted: &EventFingerprint,
    found: &mut Vec<String>,
) {
    let mut say = |what: &str, got: String, want: String| {
        found.push(format!("{who}: {what} is {got}, expected {want}"));
    };
    let short = |hash: &Option<String>| {
        hash.as_ref().map_or("none".to_string(), |h| {
            h.chars().take(16).collect::<String>() + "..."
        })
    };
    if actual.count != wanted.count {
        say(
            "the count of records",
            actual.count.to_string(),
            wanted.count.to_string(),
        );
    }
    if actual.first != wanted.first {
        say(
            "the first timestamp",
            format!("{:?}", actual.first),
            format!("{:?}", wanted.first),
        );
    }
    if actual.last != wanted.last {
        say(
            "the last timestamp",
            format!("{:?}", actual.last),
            format!("{:?}", wanted.last),
        );
    }
    if actual.timestamps_sha256 != wanted.timestamps_sha256 {
        say(
            "the SHA-256 of the timestamps",
            short(&Some(actual.timestamps_sha256.clone())),
            short(&Some(wanted.timestamps_sha256.clone())),
        );
    }
    for (what, got, want) in [
        (
            "the SHA-256 of the values",
            &actual.values_sha256,
            &wanted.values_sha256,
        ),
        (
            "the SHA-256 of the starts",
            &actual.start_sha256,
            &wanted.start_sha256,
        ),
        (
            "the SHA-256 of the stops",
            &actual.stop_sha256,
            &wanted.stop_sha256,
        ),
    ] {
        if got != want {
            say(what, short(got), short(want));
        }
    }
}

/// What the typed view must be for a series that declares `declared`: numbers for `Double`, flags for `Boolean`, and none for anything else.
fn typed_problem(expected: &EventExpectation, columns: &EventColumns) -> Option<String> {
    // a place with no values has nothing to type
    let Some(values) = &columns.values else {
        return columns
            .typed
            .as_ref()
            .map(|_| "there is a typed view and no values".to_string());
    };
    let held = Some(values.len());
    match (&expected.data_type, &columns.typed) {
        (Some(EventDataType::Double), Some(TypedValues::Doubles { .. }))
        | (Some(EventDataType::Boolean), Some(TypedValues::Booleans { .. })) => {
            let typed = columns.typed.as_ref().map_or(0, |t| t.len());
            (held != Some(typed))
                .then(|| format!("the typed view holds {typed} values for {held:?} texts"))
        }
        (Some(EventDataType::Double), other) => Some(format!(
            "the series declares Double and the typed view is {}",
            if other.is_some() {
                "of flags"
            } else {
                "missing"
            }
        )),
        (Some(EventDataType::Boolean), other) => Some(format!(
            "the series declares Boolean and the typed view is {}",
            if other.is_some() {
                "of numbers"
            } else {
                "missing"
            }
        )),
        (_, Some(_)) => {
            Some("the series declares no type this library names and has a typed view".to_string())
        }
        (_, None) => None,
    }
}

/// Every way in which what `source` serves is not what the fingerprints of `expectation` say, as lines that name the series and the place.
///
/// Each described place is read in full with a context that never cancels; one that cannot be read is reported with the error. The result must also be **aligned** (the expectation
/// files hold no series whose columns differ in length) and its typed view must be the one the series' declared type asks for. An expectation that cannot be read as one that
/// describes events is a single line.
pub fn check_event_columns(source: &dyn ColumnSource, expectation: &Expectation) -> Vec<String> {
    let places = match event_expectations(expectation) {
        Ok(places) => places,
        Err(problem) => return vec![problem],
    };
    let context = ReadContext::new();
    let mut found = Vec::new();
    for place in &places {
        let who = place.whose();
        let request = EventRequest::new(place.kind.clone(), place.path.clone(), place.place);
        match source.read_events(&request, &context) {
            Err(error) => found.push(format!("{who}: the place was not read: {error}")),
            Ok(columns) => {
                differences(
                    &who,
                    &EventFingerprint::of(&columns),
                    &place.fingerprint,
                    &mut found,
                );
                if !columns.is_aligned() {
                    found.push(format!(
                        "{who}: the columns are not as long as the timestamps (the values are {} short)",
                        columns.values_missing()
                    ));
                }
                if let Some(problem) = typed_problem(place, &columns) {
                    found.push(format!("{who}: {problem}"));
                }
            }
        }
    }
    found
}
