//! Where the columns of an OPC UA series are stored in V17, and how they are read (IP-06 phase 6F, task T06.6; decision D06-12).
//!
//! The metadata described the series in phase 13K (`super::events`); this file reads their **records**.
//!
//! * **A place is a group.** A series recorded in a layer is in `Layer_M/Global_Sensors/<events group>/<path>`, and in the bucket that no layer owns in
//!   `Global_Sensors/unassigned/<events group>/<path>` beside the mount; the events group is the kind of the series (decision D13-19). The group holds `Source_ts`, the times; `Value`, the
//!   values as text, for a sensor and the run state; and `Start_ns` and `Stop_ns` for a task.
//! * **The metadata answers first** (D06-9): a series that `BuildMeta.events` does not have, or has not recorded in the place asked for, is `NotPresent` and the file is not touched. A series that the
//!   metadata says is recorded and the file does not hold is `Corrupt`.
//! * **The numeric columns go through the column engine** (`crate::columns`): the same blocks, cancel and progress rules, and the same cache, so that a second read of a place costs nothing. The
//!   engine names each column for its cache by an [`Identity`] made here, different for each column of each place.
//! * **The values are text** and are read by IP-03's `read_strings`, as stored, **never padded**: a values column shorter than the timestamps (tauri can write fewer values, CT-DA-09) gives the
//!   values that exist, and the result says how many records have none. The range asked for is a range of records; each column is cut at its own end.
//! * **Nothing is typed here**: the typed view of the values is the Model's, made from the text and the type the series declares.
//!
//! Every name comes from [`super::layout`].

use build_log_model::{
    ChannelKey, ColumnData, ColumnRange, Error, ErrorKind, EventColumns, EventDataType, EventKind,
    EventPlace, EventRequest, LayerId, ReadContext, Shared,
};
use hdf5::dataset::Dataset;

use super::columns::V17Locator;
use super::layout;
use super::read::parent_of;
use crate::columns::{ColumnReader, Identity};
use crate::hdf5::{from_binding, read_strings};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

fn not_present(what: &str) -> Error {
    Error::new(ErrorKind::NotPresent {
        what: what.to_string(),
    })
}

/// The datasets that hold the columns of one place of a series.
struct Datasets {
    timestamps: Dataset,
    values: Option<Dataset>,
    start: Option<Dataset>,
    stop: Option<Dataset>,
    /// The type the series declares for its values, from the metadata.
    declared: Option<EventDataType>,
}

impl V17Locator {
    /// The datasets of the columns of the place a request names. The metadata says whether the series is recorded there; the file says where.
    fn locate_events(&self, request: &EventRequest) -> Result<Datasets, Error> {
        let build = self.build()?;
        let series = build
            .events
            .iter()
            .find(|series| series.kind == request.kind && series.path == request.path)
            .ok_or_else(|| not_present("the series"))?;
        let (recorded, place) = match request.place {
            EventPlace::Layer(LayerId(id)) => {
                (series.layers.contains(LayerId(id)), format!("layer {id}"))
            }
            EventPlace::NoLayer => (
                series.no_layer.is_some(),
                "the bucket that no layer owns".to_string(),
            ),
            _ => (false, "a place that V17 does not have".to_string()),
        };
        if !recorded {
            return Err(not_present("the series in this place"));
        }
        let kind = match request.kind {
            EventKind::Sensor => layout::SENSOR_EVENTS,
            EventKind::Task => layout::TASK_EVENTS,
            EventKind::Run => layout::RUN_EVENTS,
            _ => return Err(not_present("a kind of event that V17 has")),
        };
        let missing = || {
            corrupt(format!(
                "the metadata says the series is recorded in {place}, and the file has no group for it"
            ))
        };
        let holder = match request.place {
            EventPlace::Layer(LayerId(id)) => self
                .mount
                .group(layout::MONITORING_DATA)
                .and_then(|layers| layers.group(&format!("{}{id}", layout::LAYER_PREFIX)))
                .and_then(|layer| layer.group(layout::GLOBAL_SENSORS)),
            _ => parent_of(&self.mount)?
                .group(layout::GLOBAL_SENSORS)
                .and_then(|events| events.group(layout::UNASSIGNED)),
        }
        .map_err(|_| missing())?;
        let mut group = holder.group(kind).map_err(|_| missing())?;
        for part in &request.path {
            group = group.group(part).map_err(|_| missing())?;
        }
        let timestamps = group.dataset(layout::SOURCE_TS).map_err(|_| missing())?;
        let optional = |name: &str| -> Result<Option<Dataset>, Error> {
            if !group.link_exists(name) {
                return Ok(None);
            }
            group
                .dataset(name)
                .map(Some)
                .map_err(|e| from_binding("opening a column of a series", e))
        };
        Ok(Datasets {
            timestamps,
            values: optional(layout::EVENT_VALUE)?,
            start: optional(layout::START_NS)?,
            stop: optional(layout::STOP_NS)?,
            declared: series.data_type.clone(),
        })
    }
}

/// How the cache names a column of a place: the layer (none for the bucket, which has its own first word so that it cannot be taken for a layer), then the kind, the path and the column.
fn identity(request: &EventRequest, column: &str) -> Identity {
    let (layer, first) = match request.place {
        EventPlace::Layer(layer) => (layer, "events"),
        _ => (LayerId(0), "events-no-layer"),
    };
    let mut parts = vec![first.to_string(), request.kind.name().to_string()];
    parts.extend(request.path.iter().cloned());
    parts.push(column.to_string());
    Identity {
        layer,
        laser: None,
        channel: ChannelKey::new(parts),
    }
}

/// The number of values of a one-dimensional dataset.
fn length_of(dataset: &Dataset) -> Result<usize, Error> {
    match dataset.shape().as_slice() {
        [length] => Ok(*length),
        other => Err(corrupt(format!(
            "a column of a series is a list, and this one has {} dimensions",
            other.len()
        ))),
    }
}

/// The nanoseconds a column was read as: they are stored as `uint64`, and a column of another type is not one.
fn nanoseconds(data: ColumnData) -> Result<Shared<u64>, Error> {
    match data {
        ColumnData::U64(values) => Ok(values),
        _ => Err(corrupt(
            "the times of a series are `uint64` nanoseconds, and this column is stored as another type"
                .to_string(),
        )),
    }
}

/// The records `range` that a column of `length` entries holds: the range, cut where the column ends. Nothing is padded.
fn within(range: &std::ops::Range<usize>, length: usize) -> std::ops::Range<usize> {
    range.start.min(length)..range.end.min(length)
}

/// Reads the columns of the place `request` names: the times, the values as text, and a task's start and stop, for the records asked for.
pub(super) fn read_events(
    reader: &ColumnReader<V17Locator>,
    request: &EventRequest,
    context: &ReadContext,
) -> Result<EventColumns, Error> {
    context.check_cancelled()?;
    let found = reader.locator().locate_events(request)?;
    let total = length_of(&found.timestamps)?;
    let timestamps = nanoseconds(reader.read_dataset(
        &found.timestamps,
        identity(request, "timestamps"),
        &request.range,
        context,
    )?)?;
    // the records asked for, as indices: the read of the timestamps has checked the range against them, and a time range has been refused
    let range = match &request.range {
        ColumnRange::Indices(records) => records.start as usize..records.end as usize,
        _ => 0..total,
    };
    let mut columns = EventColumns::new(timestamps);

    // the numeric columns of an interval, each cut at its own end
    for (dataset, name) in [(&found.start, "start"), (&found.stop, "stop")] {
        let Some(dataset) = dataset else { continue };
        context.check_cancelled()?;
        let part = within(&range, length_of(dataset)?);
        let values = nanoseconds(reader.read_dataset(
            dataset,
            identity(request, name),
            &ColumnRange::Indices(part.start as u64..part.end as u64),
            context,
        )?)?;
        if name == "start" {
            columns.start = Some(values);
        } else {
            columns.stop = Some(values);
        }
    }

    // the values, as text, cut where they end
    if let Some(dataset) = &found.values {
        context.check_cancelled()?;
        let part = within(&range, length_of(dataset)?);
        let texts = read_strings(dataset, part)?;
        columns = columns.with_values(Shared::new(texts), found.declared.as_ref());
    }
    Ok(columns)
}
