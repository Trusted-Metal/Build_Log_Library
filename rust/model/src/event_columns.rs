//! The columns of an OPC UA event series: the times and the values of one place of one series, as a column accessor hands them over (IP-06 phase 6F; decision D06-12;
//! requirement BL-CD-15).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** telemetry.
//! * **Constructed by:** caller options (`EventRequest`); library (`EventColumns`); the enums `EventPlace` and `TypedValues`.
//! * **Requirement:** BL-CD-15 (the events), BL-CD-03 (bulk data only through a column accessor).
//!
//! # What this module claims, and what it does not
//!
//! * **A series is described by the metadata and read through [`ColumnSource::read_events`](crate::ColumnSource::read_events).** [`EventSeries`](crate::EventSeries) says what the
//!   series is and where it was recorded; a request names one **place** of it (a layer, or the bucket that no layer owns) and says which records it wants.
//! * **The values are text, as stored, and always.** A V17 file holds each value as text and says the type of the series in an attribute. The result carries the text, and, **beside
//!   it**, a typed view when the series declares a type that this library names (`Double`, `Boolean`): the numbers or the flags, each with a validity flag. **A value that does not
//!   parse is invalid, never zero** (Vision's DA-11); what the typed view holds in its place is meaningless, and is not zero either (NaN for a number, `false` for a flag).
//! * **A column that is shorter than the timestamps is reported and never padded.** tauri can write fewer values than timestamps (CT-DA-09). The result holds the values that
//!   exist, and says how many records have none ([`EventColumns::values_missing`], [`EventColumns::is_aligned`]).
//! * **Nothing here puts an event on a laser's sample grid.** That is processing (Vision's DA-10), not reading.
//!
//! The policy guards in `tests/policy_guards.rs` scan this file: every enum is `#[non_exhaustive]`, every public struct says who builds it, and the types that hold bulk data
//! say `// bulk-container:`.

use std::ops::Range;

use crate::column::{ColumnRange, Shared};
use crate::error::Location;
use crate::events::{EventDataType, EventKind};
use crate::identity::{ChannelKey, LayerId};

/// Where a series is read: inside a layer, or in the bucket that no layer owns (what happened before the build, between layers and after it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EventPlace {
    Layer(LayerId),
    NoLayer,
}

/// A request for the columns of one place of an event series: the series (its kind and its path, as in `BuildMeta.events`), the place, and which records.
// constructed-by: caller-options
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EventRequest {
    pub kind: EventKind,
    pub path: Vec<String>,
    pub place: EventPlace,
    /// Which records: every one, or the records at these indices. **A time range is not read from events** in the files that exist.
    pub range: ColumnRange,
}

impl EventRequest {
    /// A request for every record of the series in a place.
    pub fn new(kind: EventKind, path: Vec<String>, place: EventPlace) -> Self {
        EventRequest {
            kind,
            path,
            place,
            range: ColumnRange::All,
        }
    }

    pub fn with_range(mut self, range: ColumnRange) -> Self {
        self.range = range;
        self
    }

    /// Where this request points, for an error that must say which series it was about: the layer if the place is one, the series as a channel (its kind, then its path) and
    /// the operation `read_events`.
    pub fn location(&self) -> Location {
        let mut location = Location::new().with_operation("read_events");
        if let EventPlace::Layer(layer) = self.place {
            location = location.with_layer(layer);
        }
        let mut parts = vec![self.kind.name().to_string()];
        parts.extend(self.path.iter().cloned());
        location.with_channel(ChannelKey::new(parts))
    }
}

/// The typed view of the values of a series that declares a type this library names. The text as stored is always in [`EventColumns::values`]; this is beside it.
///
/// **`valid[i]` says whether the text of record `i` parsed.** Where it did not, `values[i]` means nothing: it is NaN for a number and `false` for a flag, which are not a reading
/// and must not be used as one.
// bulk-container: the typed values of one place of an event series
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TypedValues {
    /// A series that declares `Double`: the number each text parses to, as Rust reads a decimal (so `0.013`, `-1e3`, `inf` and `NaN` parse, and an empty text, a text with a
    /// space in it or a word do not).
    Doubles {
        values: Shared<f64>,
        valid: Shared<bool>,
    },
    /// A series that declares `Boolean`: `True` and `False` as the servers write them, and `true` and `false`. Nothing else parses (not `1`, not `yes`).
    Booleans {
        values: Shared<bool>,
        valid: Shared<bool>,
    },
}

/// A decimal number as text, or `None` for what is not one.
fn parse_double(text: &str) -> Option<f64> {
    text.parse::<f64>().ok()
}

/// A flag as text, or `None` for what is not one.
fn parse_boolean(text: &str) -> Option<bool> {
    match text {
        "True" | "true" => Some(true),
        "False" | "false" => Some(false),
        _ => None,
    }
}

impl TypedValues {
    /// The typed view of `texts` for a series that declares `declared`, or `None` for a type that this library does not name (the text is all there is).
    pub fn of(declared: &EventDataType, texts: &[String]) -> Option<TypedValues> {
        match declared {
            EventDataType::Double => {
                let (values, valid): (Vec<f64>, Vec<bool>) = texts
                    .iter()
                    .map(|text| match parse_double(text) {
                        Some(number) => (number, true),
                        None => (f64::NAN, false),
                    })
                    .unzip();
                Some(TypedValues::Doubles {
                    values: Shared::new(values),
                    valid: Shared::new(valid),
                })
            }
            EventDataType::Boolean => {
                let (values, valid): (Vec<bool>, Vec<bool>) = texts
                    .iter()
                    .map(|text| match parse_boolean(text) {
                        Some(flag) => (flag, true),
                        None => (false, false),
                    })
                    .unzip();
                Some(TypedValues::Booleans {
                    values: Shared::new(values),
                    valid: Shared::new(valid),
                })
            }
            _ => None,
        }
    }

    /// How many values the view holds, which is how many texts it was made from.
    pub fn len(&self) -> usize {
        match self {
            TypedValues::Doubles { valid, .. } | TypedValues::Booleans { valid, .. } => valid.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The validity flags: which records parsed.
    pub fn valid(&self) -> &[bool] {
        match self {
            TypedValues::Doubles { valid, .. } | TypedValues::Booleans { valid, .. } => {
                valid.as_slice()
            }
        }
    }

    /// The values `range`, sharing this view's allocation. `None` if the range is outside it.
    pub fn slice(&self, range: Range<usize>) -> Option<TypedValues> {
        Some(match self {
            TypedValues::Doubles { values, valid } => TypedValues::Doubles {
                values: values.slice(range.clone())?,
                valid: valid.slice(range)?,
            },
            TypedValues::Booleans { values, valid } => TypedValues::Booleans {
                values: values.slice(range.clone())?,
                valid: valid.slice(range)?,
            },
        })
    }
}

impl PartialEq for TypedValues {
    /// Two views are equal when they hold the same flags and the same values **where they are valid**; the bits of a double are compared, so that the NaN of an invalid value
    /// equals itself.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                TypedValues::Doubles {
                    values: a,
                    valid: va,
                },
                TypedValues::Doubles {
                    values: b,
                    valid: vb,
                },
            ) => {
                va.as_slice() == vb.as_slice()
                    && a.as_slice()
                        .iter()
                        .zip(b.as_slice())
                        .zip(va.as_slice())
                        .all(|((x, y), valid)| !valid || x.to_bits() == y.to_bits())
            }
            (
                TypedValues::Booleans {
                    values: a,
                    valid: va,
                },
                TypedValues::Booleans {
                    values: b,
                    valid: vb,
                },
            ) => va.as_slice() == vb.as_slice() && a.as_slice() == b.as_slice(),
            _ => false,
        }
    }
}

/// The columns of one place of an event series, for the records that were asked for.
///
/// `timestamps` always holds the source times, in nanoseconds, as stored. The other columns are there when the place holds them: the values (text) for a sensor and the run
/// state, and `start` and `stop` for a task. **Every column is index-aligned with the timestamps from the first record of the range**; a column that ends before the
/// timestamps do is shorter, is reported ([`EventColumns::values_missing`], [`EventColumns::is_aligned`]) and is **never padded**.
// constructed-by: library
// bulk-container: the typed, shared columns of one place of an event series
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EventColumns {
    /// The times the source stamped each record with, in nanoseconds since the epoch, as stored.
    pub timestamps: Shared<u64>,
    /// The values as stored (text), or `None` when the place holds no values column (a task holds none).
    pub values: Option<Shared<String>>,
    /// The values parsed by the type the series declares: the view of `values`, with a validity flag for each. `None` for a series that declares no type or one this library
    /// does not name.
    pub typed: Option<TypedValues>,
    /// When each record of a task started, in nanoseconds, or `None` for a place with no interval.
    pub start: Option<Shared<u64>>,
    /// When each record of a task stopped, in nanoseconds, or `None` for a place with no interval.
    pub stop: Option<Shared<u64>>,
}

impl EventColumns {
    /// The columns of a place that holds timestamps and nothing else yet.
    pub fn new(timestamps: Shared<u64>) -> Self {
        EventColumns {
            timestamps,
            values: None,
            typed: None,
            start: None,
            stop: None,
        }
    }

    /// Adds the values as stored, and their typed view when `declared` names a type this library knows.
    pub fn with_values(mut self, values: Shared<String>, declared: Option<&EventDataType>) -> Self {
        self.typed = declared.and_then(|declared| TypedValues::of(declared, values.as_slice()));
        self.values = Some(values);
        self
    }

    /// Adds the interval of a task.
    pub fn with_interval(mut self, start: Shared<u64>, stop: Shared<u64>) -> Self {
        self.start = Some(start);
        self.stop = Some(stop);
        self
    }

    /// How many records the result holds.
    pub fn len(&self) -> usize {
        self.timestamps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    /// How many records of this result have **no value** because the values column ends before the timestamps do. It is 0 when every record has one, and also when the place
    /// holds no values column at all (`values` is `None`): a task has none, and that is not a shortfall.
    pub fn values_missing(&self) -> u64 {
        self.values.as_ref().map_or(0, |values| {
            self.timestamps.len().saturating_sub(values.len()) as u64
        })
    }

    /// Whether every column that is present is as long as the timestamps. A place where tauri wrote fewer values than timestamps is not aligned, and the values that exist are
    /// the first ones: the result is **never padded**.
    pub fn is_aligned(&self) -> bool {
        let n = self.timestamps.len();
        self.values.as_ref().is_none_or(|v| v.len() == n)
            && self.start.as_ref().is_none_or(|v| v.len() == n)
            && self.stop.as_ref().is_none_or(|v| v.len() == n)
    }

    /// The records `range` of this result, sharing its allocations. A column that ends before the range does is cut where it ends, and a column that ends before the range
    /// starts is empty: nothing is padded. `None` if the range is outside the timestamps.
    pub fn range(&self, range: Range<usize>) -> Option<EventColumns> {
        let timestamps = self.timestamps.slice(range.clone())?;
        let cut = |held: usize| range.start.min(held)..range.end.min(held);
        Some(EventColumns {
            timestamps,
            values: match &self.values {
                Some(values) => Some(values.slice(cut(values.len()))?),
                None => None,
            },
            typed: match &self.typed {
                Some(typed) => Some(typed.slice(cut(typed.len()))?),
                None => None,
            },
            start: match &self.start {
                Some(start) => Some(start.slice(cut(start.len()))?),
                None => None,
            },
            stop: match &self.stop {
                Some(stop) => Some(stop.slice(cut(stop.len()))?),
                None => None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn shared_texts(items: &[&str]) -> Shared<String> {
        Shared::new(texts(items))
    }

    #[test]
    fn a_double_parses_as_rust_reads_a_decimal_and_what_does_not_parse_is_invalid_and_not_zero() {
        let typed = TypedValues::of(
            &EventDataType::Double,
            &texts(&["0.013", "-1e3", "0", "", " 1", "abc", "1,5", "NaN", "inf"]),
        )
        .expect("a typed view of a Double");
        let TypedValues::Doubles { values, valid } = &typed else {
            panic!("numbers");
        };
        assert_eq!(
            valid.as_slice(),
            [true, true, true, false, false, false, false, true, true]
        );
        // where it is valid the number is what the text says
        assert_eq!(&values.as_slice()[..3], [0.013, -1000.0, 0.0]);
        // where it is not, the value is NaN: it is not the zero of the text "0" two places before
        for at in [3, 4, 5, 6] {
            assert!(values.as_slice()[at].is_nan(), "record {at}");
        }
        assert_eq!(typed.len(), 9);
        assert_eq!(typed.valid(), valid.as_slice());
    }

    #[test]
    fn a_boolean_is_the_two_words_in_the_two_spellings_and_nothing_else() {
        let typed = TypedValues::of(
            &EventDataType::Boolean,
            &texts(&[
                "True", "False", "true", "false", "1", "0", "yes", "", "TRUE",
            ]),
        )
        .expect("a typed view of a Boolean");
        let TypedValues::Booleans { values, valid } = &typed else {
            panic!("flags");
        };
        assert_eq!(
            valid.as_slice(),
            [true, true, true, true, false, false, false, false, false]
        );
        assert_eq!(&values.as_slice()[..4], [true, false, true, false]);
        // an invalid value is `false` in the view, and the validity flag is what says it is not a reading
        assert!(values.as_slice()[4..].iter().all(|v| !v));
    }

    #[test]
    fn a_type_this_library_does_not_name_has_no_typed_view() {
        assert!(TypedValues::of(&EventDataType::Other("Int32".into()), &texts(&["1"])).is_none());
        // the columns hold the text and no typed view, whatever the series declares
        let columns = EventColumns::new(Shared::new(vec![1])).with_values(
            shared_texts(&["1"]),
            Some(&EventDataType::Other("Int32".into())),
        );
        assert!(columns.typed.is_none());
        assert_eq!(columns.values.as_ref().unwrap().as_slice(), ["1"]);
        let none = EventColumns::new(Shared::new(vec![1])).with_values(shared_texts(&["x"]), None);
        assert!(none.typed.is_none());
    }

    #[test]
    fn two_typed_views_with_an_invalid_double_are_equal_when_they_hold_the_same() {
        let a = TypedValues::of(&EventDataType::Double, &texts(&["1", "x"])).unwrap();
        let b = TypedValues::of(&EventDataType::Double, &texts(&["1", "y"])).unwrap();
        let c = TypedValues::of(&EventDataType::Double, &texts(&["2", "x"])).unwrap();
        let d = TypedValues::of(&EventDataType::Double, &texts(&["1", "2"])).unwrap();
        // the invalid texts differ and the views are equal: the NaN of an invalid value is not compared
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(
            a, d,
            "a value that is valid in one and not in the other differs"
        );
        let flags = TypedValues::of(&EventDataType::Boolean, &texts(&["True", "?"])).unwrap();
        assert_ne!(a, flags);
    }

    #[test]
    fn a_values_column_shorter_than_the_timestamps_is_reported_and_not_padded() {
        let columns = EventColumns::new(Shared::new(vec![10, 20, 30, 40]))
            .with_values(shared_texts(&["1", "2"]), Some(&EventDataType::Double));
        assert_eq!(columns.len(), 4);
        assert_eq!(columns.values.as_ref().unwrap().len(), 2);
        assert_eq!(columns.typed.as_ref().unwrap().len(), 2);
        assert_eq!(columns.values_missing(), 2);
        assert!(!columns.is_aligned());
        // a place with all its values is aligned, and one with no values column is neither short nor aligned by a value
        let whole =
            EventColumns::new(Shared::new(vec![1, 2])).with_values(shared_texts(&["a", "b"]), None);
        assert!(whole.is_aligned() && whole.values_missing() == 0);
        let task = EventColumns::new(Shared::new(vec![1, 2]))
            .with_interval(Shared::new(vec![0, 1]), Shared::new(vec![1, 2]));
        assert!(task.is_aligned() && task.values_missing() == 0 && task.values.is_none());
        // an interval that is short makes the result not aligned too
        let short = EventColumns::new(Shared::new(vec![1, 2]))
            .with_interval(Shared::new(vec![0]), Shared::new(vec![1, 2]));
        assert!(!short.is_aligned());
    }

    #[test]
    fn a_range_cuts_every_column_at_the_same_records_and_never_past_where_a_column_ends() {
        let columns = EventColumns::new(Shared::new(vec![10, 20, 30, 40, 50]))
            .with_values(shared_texts(&["1", "2", "3"]), Some(&EventDataType::Double))
            .with_interval(
                Shared::new(vec![9, 19, 29, 39, 49]),
                Shared::new(vec![11, 21, 31, 41, 51]),
            );
        let part = columns.range(1..4).expect("inside");
        assert_eq!(part.timestamps.as_slice(), [20, 30, 40]);
        assert_eq!(part.values.as_ref().unwrap().as_slice(), ["2", "3"]);
        assert_eq!(part.typed.as_ref().unwrap().len(), 2);
        assert_eq!(part.start.as_ref().unwrap().as_slice(), [19, 29, 39]);
        assert_eq!(part.values_missing(), 1);
        // a range that starts after the values end has no values, and the timestamps are all there
        let late = columns.range(3..5).expect("inside");
        assert_eq!(late.timestamps.as_slice(), [40, 50]);
        assert_eq!(late.values.as_ref().unwrap().len(), 0);
        assert_eq!(late.values_missing(), 2);
        // the part shares the allocation, and a range past the timestamps is not a range
        assert!(columns.range(2..9).is_none());
        assert_eq!(columns.range(2..2).expect("empty").len(), 0);
    }

    #[test]
    fn a_request_says_where_it_points_for_an_error() {
        let in_a_layer = EventRequest::new(
            EventKind::Sensor,
            texts(&["Actuators", "node", "current_velocity"]),
            EventPlace::Layer(LayerId(3)),
        );
        let location = in_a_layer.location();
        let said = format!("{location}");
        assert!(said.contains("read_events"), "{said}");
        assert!(said.contains("layer 3"), "{said}");
        assert!(
            said.contains("sensor") && said.contains("current_velocity"),
            "{said}"
        );
        let in_the_bucket =
            EventRequest::new(EventKind::Run, texts(&["RunState"]), EventPlace::NoLayer);
        assert!(!format!("{}", in_the_bucket.location()).contains("layer"));
        // the default range is every record, and a range can be set
        assert_eq!(in_the_bucket.range, ColumnRange::All);
        assert_eq!(
            in_the_bucket.with_range(ColumnRange::Indices(2..5)).range,
            ColumnRange::Indices(2..5)
        );
    }

    // ---------------------------------------------------------------- reading a place from a source

    use crate::column::{
        CancelToken, ColumnData, ColumnRequest, ColumnSource, InMemorySource, ReadContext,
    };
    use crate::error::{Error, ErrorKind};

    fn series_path() -> Vec<String> {
        texts(&["Actuators", "node", "current_velocity"])
    }

    /// A source that holds one sensor series in layer 2 and the run state in the bucket of no layer.
    fn source_with_events() -> InMemorySource {
        let mut source = InMemorySource::new();
        source.insert_events(
            EventKind::Sensor,
            series_path(),
            EventPlace::Layer(LayerId(2)),
            EventColumns::new(Shared::new(vec![100, 200, 300, 400])).with_values(
                shared_texts(&["0.5", "x", "1.5"]),
                Some(&EventDataType::Double),
            ),
        );
        source.insert_events(
            EventKind::Run,
            texts(&["RunState"]),
            EventPlace::NoLayer,
            EventColumns::new(Shared::new(vec![7])).with_values(shared_texts(&["started"]), None),
        );
        source
    }

    fn sensor(place: EventPlace) -> EventRequest {
        EventRequest::new(EventKind::Sensor, series_path(), place)
    }

    fn kind_of(error: &Error) -> &ErrorKind {
        error.kind()
    }

    #[test]
    fn a_source_serves_a_place_whole_or_by_range_and_the_series_of_one_place_is_not_the_series_of_another(
    ) {
        let source = source_with_events();
        let context = ReadContext::new();
        let whole = source
            .read_events(&sensor(EventPlace::Layer(LayerId(2))), &context)
            .expect("the place");
        assert_eq!(whole.timestamps.as_slice(), [100, 200, 300, 400]);
        assert_eq!(whole.values_missing(), 1);
        // the typed view says which values parsed: the second is not a number, and is not zero
        let TypedValues::Doubles { values, valid } = whole.typed.as_ref().expect("a Double") else {
            panic!("numbers");
        };
        assert_eq!(valid.as_slice(), [true, false, true]);
        assert_eq!((values.as_slice()[0], values.as_slice()[2]), (0.5, 1.5));
        assert!(values.as_slice()[1].is_nan());

        let part = source
            .read_events(
                &sensor(EventPlace::Layer(LayerId(2))).with_range(ColumnRange::Indices(1..3)),
                &context,
            )
            .expect("a range");
        assert_eq!(part.timestamps.as_slice(), [200, 300]);
        assert_eq!(part.values.as_ref().unwrap().as_slice(), ["x", "1.5"]);
        assert_eq!(part.typed.as_ref().unwrap().valid(), [false, true]);

        // the same series in a place it was not recorded in is not there, and neither is a series nobody recorded
        let elsewhere = source
            .read_events(&sensor(EventPlace::Layer(LayerId(3))), &context)
            .unwrap_err();
        assert!(matches!(kind_of(&elsewhere), ErrorKind::NotPresent { .. }));
        let bucket = source
            .read_events(&sensor(EventPlace::NoLayer), &context)
            .unwrap_err();
        assert!(matches!(kind_of(&bucket), ErrorKind::NotPresent { .. }));
        // the bucket holds its own series, with no layer in its location
        let run = source
            .read_events(
                &EventRequest::new(EventKind::Run, texts(&["RunState"]), EventPlace::NoLayer),
                &context,
            )
            .expect("the run state");
        assert_eq!(run.values.as_ref().unwrap().as_slice(), ["started"]);
        assert!(run.typed.is_none());
    }

    #[test]
    fn a_range_outside_the_records_a_time_range_and_a_cancelled_token_are_typed_errors_that_say_which_series(
    ) {
        let source = source_with_events();
        let place = EventPlace::Layer(LayerId(2));
        let outside = source
            .read_events(
                &sensor(place).with_range(ColumnRange::Indices(2..9)),
                &ReadContext::new(),
            )
            .unwrap_err();
        assert!(matches!(kind_of(&outside), ErrorKind::InvalidIndex { .. }));
        let said = outside.to_string();
        assert!(
            said.contains("current_velocity")
                && said.contains("layer 2")
                && said.contains("read_events"),
            "{said}"
        );

        let by_time = source
            .read_events(
                &sensor(place).with_range(ColumnRange::Time {
                    from: crate::time::TimeValue::from_file(0, None),
                    to: crate::time::TimeValue::from_file(1, None),
                }),
                &ReadContext::new(),
            )
            .unwrap_err();
        assert!(matches!(
            kind_of(&by_time),
            ErrorKind::UnsupportedInVersion { .. }
        ));

        let token = CancelToken::new();
        token.cancel();
        let cancelled = source
            .read_events(&sensor(place), &ReadContext::new().with_cancel(token))
            .unwrap_err();
        assert!(matches!(kind_of(&cancelled), ErrorKind::Cancelled));
    }

    #[test]
    fn a_source_that_has_no_event_columns_says_so_and_its_other_reads_are_unchanged() {
        // a source written before events were readable: it implements `read` and nothing else
        struct Bare;
        impl ColumnSource for Bare {
            fn read(&self, _: &ColumnRequest, _: &ReadContext) -> Result<ColumnData, Error> {
                Ok(ColumnData::U8(Shared::new(vec![1])))
            }
        }
        let error = Bare
            .read_events(&sensor(EventPlace::NoLayer), &ReadContext::new())
            .unwrap_err();
        assert!(matches!(
            kind_of(&error),
            ErrorKind::UnsupportedInVersion { .. }
        ));
        assert!(error.to_string().contains("event series"), "{error}");
        let read = Bare
            .read(
                &ColumnRequest::new(LayerId(1), None, ChannelKey::new(["x"])),
                &ReadContext::new(),
            )
            .expect("an ordinary read");
        assert_eq!(read.len(), 1);
    }
}
