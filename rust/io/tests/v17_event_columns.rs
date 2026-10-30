//! The columns of the OPC UA series of V17 (IP-06 phase 6F, task T06.6; decision D06-12): the times, the values as stored (text), the typed view of them, and a task's start and stop.
//!
//! * `every_event_place_of_the_two_files_matches_its_fingerprint`: each place where a series was recorded in the two miniatures that hold events (the gate file's 11, in the bucket that no
//!   layer owns, and the 2026-08-18 file's 111, in the bucket and in layers), read through the source in full, equals the fingerprint that the independent script wrote.
//! * The typed view: a `Double` series gives numbers and a `Boolean` one gives flags, each with a validity flag, **an unparsable value is invalid and never zero**, and a series of another
//!   type, or of none, has the text only.
//! * **A values column shorter than its timestamps is reported and never padded**; a task has its start and its stop; a range of records equals the slice of the whole read.
//! * What V17 says about a request: a series or a place that the metadata does not have is `NotPresent`, a range outside the records is `InvalidIndex`, a time range is unsupported, a read can be
//!   cancelled, and every error names the series. The numeric columns are cached.

mod common;

use build_log_conformance::{check_event_columns, event_expectations, load_all};
use build_log_io::adapters::v17::source::V17Source;
use build_log_io::adapters::v17::{self, layout};
use build_log_io::dispatch::{open, production};
use build_log_model::{
    BuildMeta, BuildSource, CancelToken, ColumnData, ColumnRange, ColumnRequest, ColumnSource,
    Direction, Error, ErrorKind, EventColumns, EventKind, EventPlace, EventRequest, Feature,
    LayerId, ReadContext, TimeValue, TypedValues,
};
use build_log_testkit::{miniature, miniature_path};
use common::{monitoring, pick_any, put_text, temp_file, Rng};
use hdf5::types::VarLenUnicode;
use hdf5::{File, Group};

/// A source seen as the column source it is, so that the conformance check can hold it to the fingerprints.
struct Through<'a>(&'a dyn BuildSource);

impl ColumnSource for Through<'_> {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        self.0.read(request, context)
    }

    fn read_events(
        &self,
        request: &EventRequest,
        context: &ReadContext,
    ) -> Result<EventColumns, Error> {
        self.0.read_events(request, context)
    }
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

// ---------------------------------------------------------------- the miniatures

const FILES: [&str; 2] = [
    miniature::V17_TRIGGERED_2026_10_19,
    miniature::V17_2026_08_18_IMAGES,
];

#[test]
fn every_event_place_of_the_two_files_matches_its_fingerprint() {
    let expectations = load_all().expect("the expectation files");
    let mut places = 0;
    for (name, count) in FILES.into_iter().zip([11, 111]) {
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let expectation = expectations
            .iter()
            .find(|e| e.miniature() == name)
            .unwrap_or_else(|| panic!("no expectation file for {name}"));
        let lines = check_event_columns(&Through(opened.source()), expectation);
        assert!(lines.is_empty(), "{name}:\n  {}", lines.join("\n  "));
        let described = event_expectations(expectation).expect("the places").len();
        assert_eq!(described, count, "{name}");
        places += described;
    }
    assert_eq!(places, 122);
}

#[test]
fn a_places_timestamps_and_values_read_as_stored() {
    // the gate file records its events in the bucket that no layer owns, and the first values of one of them are known from the file
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("the gate file opens");
    let velocity = path(&["Actuators", "optical_axis::drive::node", "current_velocity"]);
    let columns = opened
        .source()
        .read_events(
            &EventRequest::new(EventKind::Sensor, velocity, EventPlace::NoLayer),
            &ReadContext::new(),
        )
        .expect("the place");
    assert_eq!(columns.len(), 45);
    assert_eq!(columns.timestamps.as_slice()[0], 1_792_441_907_943_842_200);
    let values = columns.values.as_ref().expect("a sensor has values");
    // the text is exactly what the file holds: `0` and `0.013`, not `0.0` or `0.013000`
    assert_eq!(&values.as_slice()[..3], ["0.013", "0", "0.013"]);
    assert!(columns.is_aligned() && columns.start.is_none() && columns.stop.is_none());

    // the state of the run has no declared type, so it has the text and no typed view
    let run = opened
        .source()
        .read_events(
            &EventRequest::new(EventKind::Run, path(&["RunState"]), EventPlace::NoLayer),
            &ReadContext::new(),
        )
        .expect("the run state");
    assert_eq!(run.len(), 11);
    assert_eq!(run.values.as_ref().unwrap().as_slice()[0], "started");
    assert!(run.typed.is_none());

    // the 2026-08-18 file records the same series inside each layer, and a layer's place is not the bucket's
    let images = open(
        production(),
        miniature_path(miniature::V17_2026_08_18_IMAGES),
    )
    .expect("the images file opens");
    let series = path(&["Actuators", "optical_axis::drive::node", "current_velocity"]);
    let read = |place| {
        images
            .source()
            .read_events(
                &EventRequest::new(EventKind::Sensor, series.clone(), place),
                &ReadContext::new(),
            )
            .expect("a place")
    };
    let in_layer = read(EventPlace::Layer(LayerId(1)));
    assert_eq!(in_layer.len(), 72);
    assert_eq!(
        in_layer.values.as_ref().unwrap().as_slice()[..3],
        ["0", "0.013", "0"]
    );
    assert_ne!(
        in_layer.timestamps.as_slice(),
        read(EventPlace::NoLayer).timestamps.as_slice()
    );
}

#[test]
fn a_task_has_its_start_and_its_stop() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_2026_08_18_IMAGES),
    )
    .expect("the images file opens");
    let build = opened.source().build().expect("the metadata");
    let mut tasks = 0;
    for series in build.events.iter().filter(|s| s.kind == EventKind::Task) {
        assert!(series.interval, "{:?}", series.path);
        for (id, _) in series.layers.iter() {
            let columns = opened
                .source()
                .read_events(
                    &EventRequest::new(EventKind::Task, series.path.clone(), EventPlace::Layer(id)),
                    &ReadContext::new(),
                )
                .expect("a task's place");
            let (start, stop) = (
                columns.start.as_ref().expect("a start"),
                columns.stop.as_ref().expect("a stop"),
            );
            assert_eq!((start.len(), stop.len()), (columns.len(), columns.len()));
            assert!(columns.is_aligned());
            assert!(
                start
                    .as_slice()
                    .iter()
                    .zip(stop.as_slice())
                    .all(|(start, stop)| start <= stop),
                "{:?} layer {id}",
                series.path
            );
            tasks += 1;
        }
    }
    assert!(tasks > 0, "the file holds task events");
    // the one that is known from the file: a task that holds no values at all
    let snapshot = path(&["CameraSnapshotCommand::1", "_", "State"]);
    let columns = opened
        .source()
        .read_events(
            &EventRequest::new(EventKind::Task, snapshot, EventPlace::Layer(LayerId(1))),
            &ReadContext::new(),
        )
        .expect("the snapshot task");
    assert_eq!(columns.timestamps.as_slice(), [1_787_072_601_721_279_400]);
    assert_eq!(
        columns.start.unwrap().as_slice(),
        [1_787_072_600_794_544_700]
    );
    assert_eq!(
        columns.stop.unwrap().as_slice(),
        [1_787_072_601_721_249_700]
    );
}

#[test]
fn the_declared_type_gives_the_typed_view_of_every_series_in_the_files() {
    // every place of a `Double` series in the two files holds numbers, and every place of a `Boolean` one flags, and none holds a value that does not parse
    let (mut doubles, mut booleans) = (0, 0);
    for name in FILES {
        let opened = open(production(), miniature_path(name)).expect("the file opens");
        let build = opened.source().build().expect("the metadata");
        for series in &build.events {
            let places: Vec<EventPlace> = series
                .no_layer
                .iter()
                .map(|_| EventPlace::NoLayer)
                .chain(series.layers.ids().map(EventPlace::Layer))
                .collect();
            for place in places {
                let columns = opened
                    .source()
                    .read_events(
                        &EventRequest::new(series.kind.clone(), series.path.clone(), place),
                        &ReadContext::new(),
                    )
                    .expect("a place");
                let who = format!("{name} {:?} {place:?}", series.path);
                match (&series.data_type, &columns.typed) {
                    (
                        Some(build_log_model::EventDataType::Double),
                        Some(TypedValues::Doubles { values, valid }),
                    ) => {
                        assert!(valid.as_slice().iter().all(|v| *v), "{who}");
                        assert_eq!(values.len(), columns.len(), "{who}");
                        doubles += 1;
                    }
                    (
                        Some(build_log_model::EventDataType::Boolean),
                        Some(TypedValues::Booleans { valid, .. }),
                    ) => {
                        assert!(valid.as_slice().iter().all(|v| *v), "{who}");
                        booleans += 1;
                    }
                    (None, None) => {}
                    other => panic!("{who}: {other:?}"),
                }
            }
        }
    }
    // 91 places of numbers and 25 of flags in the 122, and the rest (the run state and the tasks) declare nothing
    assert_eq!((doubles, booleans), (91, 25));
}

#[test]
fn a_ranged_read_of_a_place_equals_a_slice_of_the_whole_read() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_2026_08_18_IMAGES),
    )
    .expect("the images file opens");
    let build = opened.source().build().expect("the metadata");
    let mut rng = Rng(0x5eed_u64);
    let mut checked = 0;
    for series in &build.events {
        let places: Vec<EventPlace> = series
            .no_layer
            .iter()
            .map(|_| EventPlace::NoLayer)
            .chain(series.layers.ids().map(EventPlace::Layer))
            .collect();
        for place in places {
            let request = EventRequest::new(series.kind.clone(), series.path.clone(), place);
            let whole = opened
                .source()
                .read_events(&request, &ReadContext::new())
                .expect("the whole place");
            for _ in 0..3 {
                let range = pick_any(&mut rng, whole.len());
                let part = opened
                    .source()
                    .read_events(
                        &request
                            .clone()
                            .with_range(ColumnRange::Indices(range.start as u64..range.end as u64)),
                        &ReadContext::new(),
                    )
                    .expect("a range");
                assert_eq!(
                    part,
                    whole.range(range.clone()).expect("a slice"),
                    "{:?} {place:?} {range:?}",
                    series.path
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 300, "{checked} ranges");
}

// ---------------------------------------------------------------- files made at test time

fn put_times(group: &Group, name: &str, values: &[u64]) {
    group
        .new_dataset::<u64>()
        .shape(values.len())
        .create(name)
        .expect("a dataset")
        .write_raw(values)
        .expect("its values");
}

fn put_strings(group: &Group, name: &str, values: &[&str]) {
    let text: Vec<VarLenUnicode> = values.iter().map(|v| v.parse().expect("text")).collect();
    group
        .new_dataset::<VarLenUnicode>()
        .shape(text.len())
        .create(name)
        .expect("a dataset")
        .write_raw(&text)
        .expect("its values");
}

/// A series in a place (a layer's `Global_Sensors` group, or the bucket that no layer owns): the groups down to `path` inside the events group `kind`, its columns, and the type
/// it declares.
fn series_in(
    place: &Group,
    kind: &str,
    path: &[&str],
    declared: Option<&str>,
    times: &[u64],
    values: Option<&[&str]>,
) -> Group {
    let mut group = place
        .group(kind)
        .or_else(|_| place.create_group(kind))
        .expect("the events group");
    for part in path {
        group = group
            .group(part)
            .or_else(|_| group.create_group(part))
            .expect("a group of the path");
    }
    put_times(&group, layout::SOURCE_TS, times);
    if let Some(values) = values {
        put_strings(&group, layout::EVENT_VALUE, values);
    }
    if let Some(declared) = declared {
        put_text(&group, layout::EVENT_DATA_TYPE, declared);
    }
    group
}

/// A series in the bucket that no layer owns, beside the mount.
fn bucket_series(
    root: &Group,
    kind: &str,
    path: &[&str],
    declared: Option<&str>,
    times: &[u64],
    values: Option<&[&str]>,
) -> Group {
    let bucket = root
        .group(layout::GLOBAL_SENSORS)
        .or_else(|_| root.create_group(layout::GLOBAL_SENSORS))
        .and_then(|events| {
            events
                .group(layout::UNASSIGNED)
                .or_else(|_| events.create_group(layout::UNASSIGNED))
        })
        .expect("the bucket");
    series_in(&bucket, kind, path, declared, times, values)
}

/// A source over a file made at test time: `write` fills the root of the file (the events of the bucket sit there, beside the mount), and the build is read from it.
fn source_over(label: &str, write: impl FnOnce(&Group)) -> (common::TempH5, File, V17Source) {
    let on_disk = temp_file(label, |file| {
        let mount = file.create_group(layout::MOUNT).expect("the mount");
        put_text(&mount, layout::BUILD_ID, "a build");
        monitoring(&mount);
        write(&file.group("/").expect("the root"));
    });
    let file = File::open(&on_disk.0).expect("the file opens");
    let mount = file.group(layout::MOUNT).expect("the mount");
    let build = v17::read::read_build(&mount).expect("the build reads");
    (on_disk, file, V17Source::new(build, mount))
}

fn in_the_bucket(kind: EventKind, parts: &[&str]) -> EventRequest {
    EventRequest::new(kind, path(parts), EventPlace::NoLayer)
}

#[test]
fn a_double_series_reads_as_numbers_and_an_unparsable_value_is_invalid_not_zero() {
    let texts = ["0.5", "abc", "", "-2e3", " 7", "0", "1,5"];
    let (_disk, _file, source) = source_over("events-double", |root| {
        bucket_series(
            root,
            layout::SENSOR_EVENTS,
            &["A", "node", "var"],
            Some("Double"),
            &[10, 20, 30, 40, 50, 60, 70],
            Some(&texts),
        );
    });
    let columns = source
        .read_events(
            &in_the_bucket(EventKind::Sensor, &["A", "node", "var"]),
            &ReadContext::new(),
        )
        .expect("the place");
    // the text is as stored, whatever it says
    assert_eq!(columns.values.as_ref().unwrap().as_slice(), texts);
    let Some(TypedValues::Doubles { values, valid }) = &columns.typed else {
        panic!("numbers: {:?}", columns.typed);
    };
    assert_eq!(
        valid.as_slice(),
        [true, false, false, true, false, true, false]
    );
    assert_eq!(
        (
            values.as_slice()[0],
            values.as_slice()[3],
            values.as_slice()[5]
        ),
        (0.5, -2000.0, 0.0)
    );
    // what does not parse is not the zero of the `0` that does
    for at in [1, 2, 4, 6] {
        assert!(values.as_slice()[at].is_nan(), "record {at}");
    }
}

#[test]
fn a_boolean_series_reads_as_flags() {
    let (_disk, _file, source) = source_over("events-boolean", |root| {
        bucket_series(
            root,
            layout::SENSOR_EVENTS,
            &["B", "node", "flag"],
            Some("Boolean"),
            &[1, 2, 3, 4, 5],
            Some(&["True", "False", "true", "maybe", "1"]),
        );
    });
    let columns = source
        .read_events(
            &in_the_bucket(EventKind::Sensor, &["B", "node", "flag"]),
            &ReadContext::new(),
        )
        .expect("the place");
    let Some(TypedValues::Booleans { values, valid }) = &columns.typed else {
        panic!("flags: {:?}", columns.typed);
    };
    assert_eq!(valid.as_slice(), [true, true, true, false, false]);
    assert_eq!(&values.as_slice()[..3], [true, false, true]);
}

#[test]
fn a_series_of_another_type_has_the_text_only() {
    let (_disk, _file, source) = source_over("events-other", |root| {
        bucket_series(
            root,
            layout::SENSOR_EVENTS,
            &["C", "node", "count"],
            Some("Int32"),
            &[1, 2],
            Some(&["7", "8"]),
        );
        bucket_series(
            root,
            layout::SENSOR_EVENTS,
            &["D", "node", "untyped"],
            None,
            &[1, 2],
            Some(&["1.5", "x"]),
        );
    });
    for (series, texts) in [
        (["C", "node", "count"], ["7", "8"]),
        (["D", "node", "untyped"], ["1.5", "x"]),
    ] {
        let columns = source
            .read_events(
                &in_the_bucket(EventKind::Sensor, &series),
                &ReadContext::new(),
            )
            .expect("the place");
        assert_eq!(columns.values.as_ref().unwrap().as_slice(), texts);
        assert!(columns.typed.is_none(), "{series:?}");
    }
}

#[test]
fn a_values_column_shorter_than_its_timestamps_is_reported_and_not_padded() {
    // tauri can write fewer values than timestamps (CT-DA-09): five records, and the values of three of them
    let (_disk, _file, source) = source_over("events-short", |root| {
        bucket_series(
            root,
            layout::SENSOR_EVENTS,
            &["S", "node", "var"],
            Some("Double"),
            &[10, 20, 30, 40, 50],
            Some(&["1", "2", "3"]),
        );
    });
    let request = in_the_bucket(EventKind::Sensor, &["S", "node", "var"]);
    let read = |range: Option<std::ops::Range<u64>>| {
        let request = match range {
            Some(range) => request.clone().with_range(ColumnRange::Indices(range)),
            None => request.clone(),
        };
        source
            .read_events(&request, &ReadContext::new())
            .expect("a read")
    };
    let whole = read(None);
    assert_eq!(whole.len(), 5);
    assert_eq!(
        whole.values.as_ref().unwrap().as_slice(),
        ["1", "2", "3"],
        "the values that exist, and no more"
    );
    assert_eq!(whole.typed.as_ref().unwrap().len(), 3);
    assert_eq!(whole.values_missing(), 2);
    assert!(!whole.is_aligned());
    // a range that holds some of the values has them and says how many records have none; one past where the values end has none
    let middle = read(Some(1..4));
    assert_eq!(middle.timestamps.as_slice(), [20, 30, 40]);
    assert_eq!(middle.values.as_ref().unwrap().as_slice(), ["2", "3"]);
    assert_eq!(middle.values_missing(), 1);
    let late = read(Some(3..5));
    assert_eq!(late.timestamps.as_slice(), [40, 50]);
    assert_eq!(late.values.as_ref().unwrap().len(), 0);
    assert_eq!(late.values_missing(), 2);
    assert!(read(Some(0..3)).is_aligned());
    // and the metadata said so too, in the origin of the place
    let build = source.build().expect("the metadata");
    let part = build.events[0].no_layer.as_ref().expect("the bucket");
    assert!(part.origins.get("values").is_some());
}

#[test]
fn a_start_or_a_stop_column_shorter_than_the_timestamps_is_cut_at_its_end_and_not_padded_too() {
    // a task of five records whose start column holds four and whose stop column holds two: each is cut at its own end, and the result is not aligned
    let (_disk, _file, source) = source_over("events-short-interval", |root| {
        let task = bucket_series(
            root,
            layout::TASK_EVENTS,
            &["Cmd", "scanner", "State"],
            None,
            &[10, 20, 30, 40, 50],
            None,
        );
        put_times(&task, layout::START_NS, &[9, 19, 29, 39]);
        put_times(&task, layout::STOP_NS, &[11, 21]);
    });
    let request = in_the_bucket(EventKind::Task, &["Cmd", "scanner", "State"]);
    let whole = source
        .read_events(&request, &ReadContext::new())
        .expect("the place");
    assert_eq!(whole.len(), 5);
    assert_eq!(whole.start.as_ref().unwrap().as_slice(), [9, 19, 29, 39]);
    assert_eq!(whole.stop.as_ref().unwrap().as_slice(), [11, 21]);
    assert!(!whole.is_aligned());
    // a range that starts where the stop column has ended has no stop, and the start it has is cut at four
    let late = source
        .read_events(
            &request.clone().with_range(ColumnRange::Indices(3..5)),
            &ReadContext::new(),
        )
        .expect("a range past the end of a column");
    assert_eq!(late.timestamps.as_slice(), [40, 50]);
    assert_eq!(late.start.as_ref().unwrap().as_slice(), [39]);
    assert_eq!(late.stop.as_ref().unwrap().len(), 0);
}

// ---------------------------------------------------------------- what V17 says about a request

#[test]
fn what_a_request_gets_wrong_is_a_typed_error_that_says_which_series() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_2026_08_18_IMAGES),
    )
    .expect("the images file opens");
    let source = opened.source();
    let velocity = path(&["Actuators", "optical_axis::drive::node", "current_velocity"]);
    let ask = |kind: EventKind, series: Vec<String>, place: EventPlace| {
        EventRequest::new(kind, series, place)
    };
    let context = ReadContext::new();

    // a series that is not there, a kind that it does not have, and a place where it was not recorded
    let nobody = source
        .read_events(
            &ask(
                EventKind::Sensor,
                path(&["No", "such", "series"]),
                EventPlace::NoLayer,
            ),
            &context,
        )
        .unwrap_err();
    assert!(
        matches!(nobody.kind(), ErrorKind::NotPresent { .. }),
        "{nobody}"
    );
    let task = source
        .read_events(
            &ask(EventKind::Task, velocity.clone(), EventPlace::NoLayer),
            &context,
        )
        .unwrap_err();
    assert!(
        matches!(task.kind(), ErrorKind::NotPresent { .. }),
        "{task}"
    );
    let elsewhere = source
        .read_events(
            &ask(
                EventKind::Sensor,
                velocity.clone(),
                EventPlace::Layer(LayerId(99)),
            ),
            &context,
        )
        .unwrap_err();
    assert!(
        matches!(elsewhere.kind(), ErrorKind::NotPresent { .. }),
        "{elsewhere}"
    );

    // a range outside the records says how many there are, and a time range is not read from events in these files
    let place = ask(
        EventKind::Sensor,
        velocity.clone(),
        EventPlace::Layer(LayerId(1)),
    );
    let outside = source
        .read_events(
            &place.clone().with_range(ColumnRange::Indices(0..9_999)),
            &context,
        )
        .unwrap_err();
    assert!(
        matches!(outside.kind(), ErrorKind::InvalidIndex { .. }),
        "{outside}"
    );
    assert!(outside.to_string().contains("72"), "{outside}");
    let by_time = source
        .read_events(
            &place.clone().with_range(ColumnRange::Time {
                from: TimeValue::from_file(0, None),
                to: TimeValue::from_file(1, None),
            }),
            &context,
        )
        .unwrap_err();
    match by_time.kind() {
        ErrorKind::UnsupportedInVersion { version, .. } => assert_eq!(version, "17"),
        other => panic!("{other:?}"),
    }

    // a cancelled read is `Cancelled`
    let token = CancelToken::new();
    token.cancel();
    let cancelled = source
        .read_events(&place, &ReadContext::new().with_cancel(token))
        .unwrap_err();
    assert!(
        matches!(cancelled.kind(), ErrorKind::Cancelled),
        "{cancelled}"
    );

    // every one of them names the series, the layer if it has one, and the operation
    for error in [&outside, &by_time, &cancelled] {
        let said = error.to_string();
        assert!(
            said.contains("current_velocity")
                && said.contains("layer 1")
                && said.contains("read_events"),
            "{said}"
        );
    }
    assert!(nobody.to_string().contains("read_events") && !nobody.to_string().contains("layer"));
}

#[test]
fn the_bucket_and_a_layer_of_the_same_series_do_not_share_what_the_cache_holds() {
    // the bucket is named for the cache with layer 0, so a layer that is numbered 0 holds the very same series and must not be taken for it
    let (_disk, _file, source) = source_over("events-cache-keys", |root| {
        let layers = root
            .group(layout::MOUNT)
            .and_then(|mount| mount.group(layout::MONITORING_DATA))
            .expect("the layers");
        let zero = common::layer(&layers, 0)
            .create_group(layout::GLOBAL_SENSORS)
            .expect("the events of layer 0");
        series_in(
            &zero,
            layout::SENSOR_EVENTS,
            &["A", "node", "var"],
            Some("Double"),
            &[1, 2, 3],
            Some(&["1", "2", "3"]),
        );
        bucket_series(
            root,
            layout::SENSOR_EVENTS,
            &["A", "node", "var"],
            Some("Double"),
            &[100, 200],
            Some(&["10", "20"]),
        );
    });
    let kept = ReadContext::new().with_cache_budget(1024 * 1024);
    let ask = |place| {
        source
            .read_events(
                &EventRequest::new(EventKind::Sensor, path(&["A", "node", "var"]), place),
                &kept,
            )
            .expect("a place")
    };
    // each is itself the first time and the second, whichever is read first
    for place in [
        EventPlace::Layer(LayerId(0)),
        EventPlace::NoLayer,
        EventPlace::Layer(LayerId(0)),
    ] {
        let columns = ask(place);
        let wanted: &[u64] = if place == EventPlace::NoLayer {
            &[100, 200]
        } else {
            &[1, 2, 3]
        };
        assert_eq!(columns.timestamps.as_slice(), wanted, "{place:?}");
    }
}

#[test]
fn a_series_the_metadata_has_and_the_file_lacks_is_corrupt() {
    // the metadata is read from a file that has the series, and the source reads another, whose mount and bucket do not hold it
    let with = source_over("events-with", |root| {
        bucket_series(
            root,
            layout::SENSOR_EVENTS,
            &["A", "node", "var"],
            Some("Double"),
            &[1, 2],
            Some(&["1", "2"]),
        );
    });
    let without = source_over("events-without", |_| {});
    let build: BuildMeta = (*with.2.build().expect("the metadata")).clone();
    let mount = without.1.group(layout::MOUNT).expect("the mount");
    let source = V17Source::new(build, mount);
    let error = source
        .read_events(
            &in_the_bucket(EventKind::Sensor, &["A", "node", "var"]),
            &ReadContext::new(),
        )
        .unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Corrupt { .. }), "{error}");
    assert!(
        error.to_string().contains("the bucket that no layer owns"),
        "{error}"
    );
}

#[test]
fn a_second_read_of_a_place_is_served_from_the_cache_when_the_budget_allows() {
    let opened = open(
        production(),
        miniature_path(miniature::V17_TRIGGERED_2026_10_19),
    )
    .expect("the gate file opens");
    let request = in_the_bucket(
        EventKind::Sensor,
        &["Actuators", "optical_axis::drive::node", "current_velocity"],
    );
    let stats = || opened.source().cache_stats().expect("the cache counts");

    // with no budget nothing is kept, so a second read finds nothing
    let none = ReadContext::new();
    opened.source().read_events(&request, &none).unwrap();
    opened.source().read_events(&request, &none).unwrap();
    assert_eq!(stats().hits, 0);

    // with a budget the timestamps are kept, and the second read is a hit
    let kept = ReadContext::new().with_cache_budget(64 * 1024 * 1024);
    let first = opened.source().read_events(&request, &kept).unwrap();
    let before = stats();
    let second = opened.source().read_events(&request, &kept).unwrap();
    let after = stats();
    assert_eq!(first, second);
    assert!(after.hits > before.hits, "{before:?} then {after:?}");
    assert_eq!(after.misses, before.misses);
}

#[test]
fn event_columns_are_declared_in_the_capabilities() {
    let set = production().sets().next().expect("the V17 adapter set");
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::EventColumns));
    // reading is declared and writing is not: the writer arrives with IP-08
    assert!(!set
        .capabilities()
        .supports(Direction::Write, Feature::EventColumns));
    // the series themselves are still described under `Events`
    assert!(set
        .capabilities()
        .supports(Direction::Read, Feature::Events));
}
