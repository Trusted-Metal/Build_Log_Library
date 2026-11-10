//! The fingerprints of event places and the check of a column source against them (IP-06 phase 6F, task T06.6, decision D06-12).
//!
//! The expectation files of the two V17 miniatures that hold events carry a fingerprint for every place where a series was recorded. The check is proved here on a source made of
//! synthetic places: a source that serves them as stored passes, and each way of serving one wrongly is reported, on that place and on no other.

use build_log_conformance::{
    check_event_columns, event_expectations, load_all, EventExpectation, EventFingerprint,
    Expectation,
};
use build_log_model::{
    EventColumns, EventDataType, EventKind, EventPlace, InMemorySource, LayerId, Shared,
    TypedValues,
};
use serde_json::{json, Value};

// ---------------------------------------------------------------- known answers

// Worked out with the standard library alone in Python (`struct` and `hashlib`); the test of the oracle holds the same literals, so that the two languages agree on what the bytes are.
const HASH_OF_NOTHING: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const TIMES_1_AND_MAX: &str = "db0550d553e2a146e34164d19cd55f006c38d700d8f9a4e3ba2c889a1d7c26b2";
const VALUES_A_BC: &str = "aa795aa4bbb6117911ef062e271bcb05ccfd58ea439da7d46a44e3a3fcefa790";
const VALUES_E_ACUTE_AND_EMPTY: &str =
    "b11c5140a383a7fa5046a4b9d3073ff392724073205d4a0f0de3768d859b1475";
const TIMES_6_8_10: &str = "48662064a1b134cc4336005d5a69c9484e9bc34ce923a0fa4ad50365e8354b06";
const START_4_6_8: &str = "ce937d177b77dcb137ed66c209fd01b3482b951268dc961c9e9adfc003e1953c";
const STOP_5_7_9: &str = "1b2622c980704343ce7b3ee89bb7b0ad01f5f44cca6cfcce1dd7a08a92b7ba1e";

fn texts(items: &[&str]) -> Shared<String> {
    Shared::new(items.iter().map(|s| s.to_string()).collect())
}

#[test]
fn the_fingerprint_of_known_places_is_what_hashlib_gives() {
    let sensor =
        EventColumns::new(Shared::new(vec![1, u64::MAX])).with_values(texts(&["a", "bc"]), None);
    assert_eq!(
        EventFingerprint::of(&sensor),
        EventFingerprint {
            count: 2,
            first: Some(1),
            last: Some(u64::MAX),
            timestamps_sha256: TIMES_1_AND_MAX.to_string(),
            values_sha256: Some(VALUES_A_BC.to_string()),
            start_sha256: None,
            stop_sha256: None,
        }
    );
    // a value in UTF-8 is hashed as its bytes, and an empty value is only the zero byte
    let accented =
        EventColumns::new(Shared::new(vec![1, 2])).with_values(texts(&["\u{e9}", ""]), None);
    assert_eq!(
        EventFingerprint::of(&accented).values_sha256.as_deref(),
        Some(VALUES_E_ACUTE_AND_EMPTY)
    );
    // a task has no values and an interval
    let task = EventColumns::new(Shared::new(vec![6, 8, 10]))
        .with_interval(Shared::new(vec![4, 6, 8]), Shared::new(vec![5, 7, 9]));
    assert_eq!(
        EventFingerprint::of(&task),
        EventFingerprint {
            count: 3,
            first: Some(6),
            last: Some(10),
            timestamps_sha256: TIMES_6_8_10.to_string(),
            values_sha256: None,
            start_sha256: Some(START_4_6_8.to_string()),
            stop_sha256: Some(STOP_5_7_9.to_string()),
        }
    );
    // a place of no record has no first and no last, and the hash of nothing
    let empty = EventColumns::new(Shared::new(Vec::new())).with_values(texts(&[]), None);
    assert_eq!(
        EventFingerprint::of(&empty),
        EventFingerprint {
            count: 0,
            first: None,
            last: None,
            timestamps_sha256: HASH_OF_NOTHING.to_string(),
            values_sha256: Some(HASH_OF_NOTHING.to_string()),
            start_sha256: None,
            stop_sha256: None,
        }
    );
}

// ---------------------------------------------------------------- a synthetic build: four series in several places

/// One place of one synthetic series.
#[derive(Clone)]
struct Place {
    kind: EventKind,
    path: &'static [&'static str],
    declared: Option<EventDataType>,
    interval: bool,
    place: EventPlace,
}

impl Place {
    fn path(&self) -> Vec<String> {
        self.path.iter().map(|p| p.to_string()).collect()
    }

    /// The beginning of every line the check says about this place.
    fn who(&self) -> String {
        let place = match self.place {
            EventPlace::Layer(LayerId(id)) => format!("layer {id}"),
            _ => "the bucket that no layer owns".to_string(),
        };
        let kind = match self.kind {
            EventKind::Sensor => "sensor",
            EventKind::Task => "task",
            _ => "run",
        };
        format!("event {kind} {} in {place}", self.path.join("/"))
    }
}

fn places() -> Vec<Place> {
    let velocity = |place| Place {
        kind: EventKind::Sensor,
        path: &["Actuators", "node", "velocity"],
        declared: Some(EventDataType::Double),
        interval: false,
        place,
    };
    vec![
        velocity(EventPlace::NoLayer),
        velocity(EventPlace::Layer(LayerId(1))),
        velocity(EventPlace::Layer(LayerId(2))),
        Place {
            kind: EventKind::Sensor,
            path: &["Valves", "node", "open"],
            declared: Some(EventDataType::Boolean),
            interval: false,
            place: EventPlace::Layer(LayerId(1)),
        },
        Place {
            kind: EventKind::Task,
            path: &["ExposeCommand", "scanner", "State"],
            declared: None,
            interval: true,
            place: EventPlace::Layer(LayerId(1)),
        },
        Place {
            kind: EventKind::Run,
            path: &["RunState"],
            declared: None,
            interval: false,
            place: EventPlace::NoLayer,
        },
    ]
}

/// The columns of a place: different for each, with a spread that a changed value can hide in.
fn columns_of(place: &Place) -> EventColumns {
    let seed = match place.place {
        EventPlace::Layer(LayerId(id)) => 1_013 * id as u64,
        _ => 17,
    } + place.path.len() as u64 * 7;
    let n = 12 + (seed % 5) as usize;
    let times: Vec<u64> = (0..n as u64)
        .map(|i| 1_700_000_000_000_000_000 + seed * 1_000 + i * 250_000_000 + (i * i) % 13)
        .collect();
    let columns = EventColumns::new(Shared::new(times.clone()));
    match (&place.kind, &place.declared) {
        (EventKind::Task, _) => columns.with_interval(
            Shared::new(times.iter().map(|t| t - 1_000).collect()),
            Shared::new(times.iter().map(|t| t - 10).collect()),
        ),
        (_, Some(EventDataType::Boolean)) => columns.with_values(
            Shared::new(
                (0..n)
                    .map(|i| {
                        if (i + seed as usize) % 3 == 0 {
                            "True"
                        } else {
                            "False"
                        }
                        .to_string()
                    })
                    .collect(),
            ),
            place.declared.as_ref(),
        ),
        (_, Some(EventDataType::Double)) => columns.with_values(
            Shared::new(
                (0..n)
                    .map(|i| format!("{}.{}", (i * 7 + seed as usize) % 40, i % 10))
                    .collect(),
            ),
            place.declared.as_ref(),
        ),
        _ => columns.with_values(
            Shared::new(
                (0..n)
                    .map(|i| ["started", "paused", "resumed"][i % 3].to_string())
                    .collect(),
            ),
            None,
        ),
    }
}

/// The source that serves every place as stored; `change` may serve one differently, or not at all (`None`).
fn source_with(change: impl Fn(&Place, EventColumns) -> Option<EventColumns>) -> InMemorySource {
    let mut source = InMemorySource::new();
    for place in places() {
        if let Some(columns) = change(&place, columns_of(&place)) {
            source.insert_events(place.kind.clone(), place.path(), place.place, columns);
        }
    }
    source
}

/// An expectation that describes the places as stored; `described` says whether each carries its fingerprint.
fn expectation(described: bool) -> Expectation {
    let mut series: Vec<Value> = Vec::new();
    for place in places() {
        let part = {
            let columns = columns_of(&place);
            let mut part = json!({
                "assignment": match place.place {
                    EventPlace::Layer(LayerId(id)) => json!(id),
                    _ => json!("unassigned"),
                },
                "records": columns.len(),
                "height": null,
            });
            if described {
                part["columns"] = EventFingerprint::of(&columns).to_json();
            }
            part
        };
        let kind = match place.kind {
            EventKind::Sensor => "sensor",
            EventKind::Task => "task",
            _ => "run",
        };
        match series
            .iter_mut()
            .find(|s| s["path"] == json!(place.path) && s["kind"] == kind)
        {
            Some(existing) => existing["parts"].as_array_mut().unwrap().push(part),
            None => series.push(json!({
                "kind": kind,
                "path": place.path,
                "data_type": place.declared.as_ref().map(|d| match d {
                    EventDataType::Double => "Double",
                    _ => "Boolean",
                }),
                "interval": place.interval,
                "parts": [part],
            })),
        }
    }
    Expectation::parse(
        &json!({"expectations_version": 1, "file_version": 17, "events": series}).to_string(),
    )
    .expect("an expectation")
}

/// Serves every place as stored except the one whose lines start with `target`, which is changed by `change`; says what the check reports. Every line must be about that place.
fn broken(target: &Place, change: impl Fn(EventColumns) -> Option<EventColumns>) -> Vec<String> {
    let source = source_with(|place, columns| {
        if place.who() == target.who() {
            change(columns)
        } else {
            Some(columns)
        }
    });
    let found = check_event_columns(&source, &expectation(true));
    assert!(
        !found.is_empty(),
        "nothing was reported for {}",
        target.who()
    );
    for line in &found {
        assert!(
            line.starts_with(&target.who()),
            "a line about another place: {line}"
        );
    }
    found
}

fn has(found: &[String], what: &str) -> bool {
    found.iter().any(|line| line.contains(what))
}

fn place(path: &str, place: EventPlace) -> Place {
    places()
        .into_iter()
        .find(|p| p.path.join("/").ends_with(path) && p.place == place)
        .expect("a place of the synthetic build")
}

#[test]
fn the_synthetic_source_with_events_passes_and_a_changed_one_fails() {
    // the source that serves every place as stored passes
    let expected = expectation(true);
    assert_eq!(
        check_event_columns(&source_with(|_, c| Some(c)), &expected),
        Vec::<String>::new()
    );
    assert_eq!(event_expectations(&expected).expect("six places").len(), 6);

    let velocity = |p| place("velocity", p);
    let in_layer_1 = velocity(EventPlace::Layer(LayerId(1)));

    // a timestamp that differs by one nanosecond, in the middle, is found by the hash and by nothing else
    let found = broken(&in_layer_1, |mut c| {
        let mut times = c.timestamps.as_slice().to_vec();
        times[5] += 1;
        c.timestamps = Shared::new(times);
        Some(c)
    });
    assert_eq!(found.len(), 1);
    assert!(has(&found, "the SHA-256 of the timestamps"));

    // the first and the last timestamp, and the count
    let found = broken(&in_layer_1, |mut c| {
        let mut times = c.timestamps.as_slice().to_vec();
        times[0] += 1;
        c.timestamps = Shared::new(times);
        Some(c)
    });
    assert!(has(&found, "the first timestamp") && has(&found, "the SHA-256 of the timestamps"));
    let found = broken(&in_layer_1, |c| c.range(0..c.len() - 1));
    assert!(has(&found, "the count of records") && has(&found, "the last timestamp"));

    // a value that differs, a values column that is not there, and one that is shorter than the timestamps
    let found = broken(&in_layer_1, |mut c| {
        let mut values = c.values.as_ref().unwrap().as_slice().to_vec();
        values[3].push('0');
        c.values = Some(Shared::new(values));
        Some(c)
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(has(&found, "the SHA-256 of the values"));
    let found = broken(&in_layer_1, |mut c| {
        c.values = None;
        c.typed = None;
        Some(c)
    });
    assert!(has(&found, "the SHA-256 of the values is none"));
    let found = broken(&in_layer_1, |mut c| {
        let values = c.values.as_ref().unwrap().as_slice().to_vec();
        c.values = Some(Shared::new(values[..values.len() - 1].to_vec()));
        Some(c)
    });
    assert!(has(
        &found,
        "the columns are not as long as the timestamps (the values are 1 short)"
    ));

    // the typed view: missing for a Double series, of the wrong kind, and for a series that declares nothing
    let found = broken(&in_layer_1, |mut c| {
        c.typed = None;
        Some(c)
    });
    assert_eq!(found.len(), 1);
    assert!(has(
        &found,
        "the series declares Double and the typed view is missing"
    ));
    let found = broken(&in_layer_1, |mut c| {
        c.typed = TypedValues::of(
            &EventDataType::Boolean,
            c.values.as_ref().unwrap().as_slice(),
        );
        Some(c)
    });
    assert!(has(
        &found,
        "the series declares Double and the typed view is of flags"
    ));
    let found = broken(&place("RunState", EventPlace::NoLayer), |c| {
        let values = c.values.clone().unwrap();
        Some(c.with_values(values, Some(&EventDataType::Double)))
    });
    assert!(has(
        &found,
        "declares no type this library names and has a typed view"
    ));

    // a task: its start, its stop, and an interval that is not there
    let task = place("State", EventPlace::Layer(LayerId(1)));
    let found = broken(&task, |mut c| {
        let mut start = c.start.as_ref().unwrap().as_slice().to_vec();
        start[2] += 1;
        c.start = Some(Shared::new(start));
        Some(c)
    });
    assert_eq!(found.len(), 1);
    assert!(has(&found, "the SHA-256 of the starts"));
    let found = broken(&task, |mut c| {
        c.stop = None;
        Some(c)
    });
    assert!(has(&found, "the SHA-256 of the stops is none"));
    // a task that is served with values it does not hold
    let found = broken(&task, |c| {
        let values = Shared::new(vec!["x".to_string(); c.len()]);
        Some(c.with_values(values, None))
    });
    assert!(has(&found, "the SHA-256 of the values"));

    // a place that is not served, and one that is served with the columns of another place of the same series
    let found = broken(&in_layer_1, |_| None);
    assert_eq!(found.len(), 1);
    assert!(has(&found, "the place was not read"));
    let other = columns_of(&velocity(EventPlace::Layer(LayerId(2))));
    let found = broken(&in_layer_1, |_| Some(other.clone()));
    assert!(
        has(&found, "the SHA-256 of the timestamps") && has(&found, "the SHA-256 of the values")
    );
}

#[test]
fn an_expectation_that_does_not_describe_a_place_is_an_error_and_not_a_pass() {
    let bare = expectation(false);
    let error = event_expectations(&bare).unwrap_err();
    assert!(error.contains("no fingerprint"), "{error}");
    // the check says so as one line, and does not pass by having nothing to check
    let lines = check_event_columns(&source_with(|_, c| Some(c)), &bare);
    assert_eq!(lines.len(), 1, "{lines:?}");
    // a fingerprint that is not what the format says is an error naming the series
    let mut root: Value = json!({"expectations_version": 1, "file_version": 17, "events": [{
        "kind": "sensor", "path": ["a"], "data_type": "Double", "interval": false,
        "parts": [{"assignment": "unassigned", "records": 1, "height": null, "columns": {"count": "one"}}]
    }]});
    let error = event_expectations(&Expectation::parse(&root.to_string()).unwrap()).unwrap_err();
    assert!(
        error.contains("sensor a") && error.contains("count"),
        "{error}"
    );
    root["events"][0]["parts"][0]["assignment"] = json!([1]);
    assert!(event_expectations(&Expectation::parse(&root.to_string()).unwrap()).is_err());
    // an expectation with no events describes none
    let none = Expectation::parse(
        &json!({"expectations_version": 1, "file_version": 16, "events": []}).to_string(),
    )
    .unwrap();
    assert!(event_expectations(&none).unwrap().is_empty());
    assert!(check_event_columns(&InMemorySource::new(), &none).is_empty());
}

#[test]
fn the_miniatures_describe_every_event_place_with_its_fingerprint() {
    let all = load_all().expect("the expectation files");
    let mut described: Vec<(String, Vec<EventExpectation>)> = Vec::new();
    for expectation in &all {
        described.push((
            expectation.miniature().to_string(),
            event_expectations(expectation).expect("every place carries its fingerprint"),
        ));
    }
    let with_events: Vec<(&str, usize)> = described
        .iter()
        .filter(|(_, p)| !p.is_empty())
        .map(|(name, p)| (name.as_str(), p.len()))
        .collect();
    // the gate file holds 11 places and the 2026-08-18 file 111; no other file holds events
    assert_eq!(
        with_events,
        [
            ("v17_2026_08_18_images.h5", 111),
            ("v17_triggered_2026_10_19.h5", 11)
        ]
    );
    for (_, list) in &described {
        for place in list {
            let f = &place.fingerprint;
            let who = format!(
                "{:?} {} {:?}",
                place.kind,
                place.path.join("/"),
                place.place
            );
            assert_eq!(f.first.is_none(), f.count == 0, "{who}");
            // a task has its interval, and only a task
            assert_eq!(f.start_sha256.is_some(), place.interval, "{who}");
            assert_eq!(f.stop_sha256.is_some(), place.interval, "{who}");
            assert!(f.values_sha256.is_some() || place.interval, "{who}");
        }
    }
}
