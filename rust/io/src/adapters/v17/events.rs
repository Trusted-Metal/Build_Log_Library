//! The OPC UA events of a V17 build (IP-13 phase 13K; decisions D13-21, D13-26 and D13-29): which signals were listened to, where each was recorded, and how many
//! records it holds there.
//!
//! * **A series is a group that holds the source timestamps.** Below an events group (`sensor_events`, `task_events` or `run_events`) any number of groups name the
//!   series by the path the OPC UA server gave it; the first group on the way down that holds `Source_ts` is the series, and the names on the way are its path.
//!   The kind of the series is the events group it is in.
//! * **A series is recorded in places.** Inside a layer (`Layer_M/Global_Sensors`) and in the bucket that no layer owns (`/Global_Sensors/unassigned`, beside the mount,
//!   decision D13-19). The same series can be recorded in several places; it is one series, and each place says its records and, for a task, its height.
//! * **A series is described and its records are not read.** The count of a place is the length of its timestamps; the values, which are text, and the interval of a
//!   task (`Start_ns` and `Stop_ns`) are columns reached later (IP-06). A series has an interval when it holds both of those columns.
//! * **The declared type and the interval are the same in every place.** A place that differs is not described and says why (a series has one type).
//! * **A column that does not match its timestamps is reported, not repaired.** tauri can write fewer values than timestamps (CT-DA-09): the place is still
//!   described with its timestamps, and its origin `values` says how many values there are.
//! * **Whatever else the groups hold is kept**, by its path, in the layer's `extra` or the build's: the attributes of a series, a column that is not known, a group that
//!   is not an events group. A series that cannot be read is reported on its layer (or the build) and kept whole (decision D13-21).
//!
//! Every name comes from [`super::layout`].

use std::collections::BTreeMap;

use build_log_model::{
    BuildMeta, Error, ErrorKind, EventDataType, EventKind, EventPart, EventSeries, Extra, LayerId,
    LayerMeta, Origin, Origins, Quantity,
};
use hdf5::Group;

use super::layout;
use crate::hdf5::{
    describe_dataset, from_binding, keep_dataset_remainder, keep_group_remainder, keep_node,
    MAX_DEPTH,
};
use crate::toolkit::content::is_about_the_content;
use crate::toolkit::values::{optional_number, optional_text};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The events groups of an assignment, and the kind of series that each holds.
fn kinds() -> [(&'static str, EventKind); 3] {
    [
        (layout::SENSOR_EVENTS, EventKind::Sensor),
        (layout::TASK_EVENTS, EventKind::Task),
        (layout::RUN_EVENTS, EventKind::Run),
    ]
}

/// Where a place is: inside a layer, or in the bucket that no layer owns.
#[derive(Clone, Copy)]
enum Place {
    Layer(LayerId),
    Unassigned,
}

/// The series of a build, collected place by place.
pub struct Events {
    series: BTreeMap<(EventKind, Vec<String>), EventSeries>,
}

impl Events {
    pub fn new() -> Self {
        Events {
            series: BTreeMap::new(),
        }
    }

    /// The events inside a layer's group. Returns the names of the children of the layer that were dealt with, so that the caller does not keep them again.
    pub fn read_layer(
        &mut self,
        group: &Group,
        layer: &mut LayerMeta,
    ) -> Result<Vec<String>, Error> {
        if !group.link_exists(layout::GLOBAL_SENSORS) {
            return Ok(Vec::new());
        }
        let base = [layout::GLOBAL_SENSORS.to_string()];
        let place = Place::Layer(layer.id);
        let outcome = group
            .group(layout::GLOBAL_SENSORS)
            .map_err(|e| corrupt(format!("`{}` is not a group: {e}", layout::GLOBAL_SENSORS)))
            .and_then(|events| {
                self.read_place(&events, place, &base, &mut layer.extra, &mut layer.origins)
            });
        match outcome {
            Ok(()) => {}
            Err(error) if is_about_the_content(&error) => {
                layer.origins.record("events", Origin::not_read(&error));
                keep_node(group, layout::GLOBAL_SENSORS, &[], &mut layer.extra)?;
            }
            Err(error) => return Err(error),
        }
        Ok(vec![layout::GLOBAL_SENSORS.to_string()])
    }

    /// The events beside the mount: the bucket that no layer owns, below `Global_Sensors`. What else that group holds is kept in the build's `extra`, by a path that
    /// starts above the mount (decision D13-19).
    pub fn read_top(
        &mut self,
        beside: &Group,
        extra: &mut Extra,
        origins: &mut Origins,
    ) -> Result<(), Error> {
        let up = vec![String::from("..")];
        let base = [String::from(".."), layout::GLOBAL_SENSORS.to_string()];
        let outcome = beside
            .group(layout::GLOBAL_SENSORS)
            .map_err(|e| corrupt(format!("`{}` is not a group: {e}", layout::GLOBAL_SENSORS)))
            .and_then(|events| {
                let mut dealt_with = Vec::new();
                if events.link_exists(layout::UNASSIGNED) {
                    let bucket = events.group(layout::UNASSIGNED).map_err(|e| {
                        corrupt(format!("`{}` is not a group: {e}", layout::UNASSIGNED))
                    })?;
                    let mut inside = base.to_vec();
                    inside.push(layout::UNASSIGNED.to_string());
                    self.read_place(&bucket, Place::Unassigned, &inside, extra, origins)?;
                    dealt_with.push(layout::UNASSIGNED);
                }
                keep_group_remainder(&events, &dealt_with, &[], &base, extra)
            });
        match outcome {
            Ok(()) => Ok(()),
            Err(error) if is_about_the_content(&error) => {
                origins.record("events", Origin::not_read(&error));
                keep_node(beside, layout::GLOBAL_SENSORS, &up, extra)
            }
            Err(error) => Err(error),
        }
    }

    /// One place: its events groups, and what else it holds.
    fn read_place(
        &mut self,
        place_group: &Group,
        place: Place,
        base: &[String],
        extra: &mut Extra,
        origins: &mut Origins,
    ) -> Result<(), Error> {
        let mut dealt_with: Vec<&str> = Vec::new();
        for (name, kind) in kinds() {
            if !place_group.link_exists(name) {
                continue;
            }
            let holder = place_group
                .group(name)
                .map_err(|e| corrupt(format!("`{name}` is not a group: {e}")))?;
            let mut inside = base.to_vec();
            inside.push(name.to_string());
            self.walk(
                &holder,
                &kind,
                &mut Vec::new(),
                place,
                &inside,
                extra,
                origins,
            )?;
            dealt_with.push(name);
        }
        keep_group_remainder(place_group, &dealt_with, &[], base, extra)
    }

    /// The groups below an events group, down to the series.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        group: &Group,
        kind: &EventKind,
        path: &mut Vec<String>,
        place: Place,
        base: &[String],
        extra: &mut Extra,
        origins: &mut Origins,
    ) -> Result<(), Error> {
        if path.len() > MAX_DEPTH {
            return Err(corrupt(format!(
                "events nested deeper than {MAX_DEPTH} levels are not followed"
            )));
        }
        let names = group
            .member_names()
            .map_err(|e| from_binding("listing the members of an events group", e))?;
        let mut groups: Vec<String> = Vec::new();
        for name in &names {
            let Ok(child) = group.group(name) else {
                continue;
            };
            groups.push(name.clone());
            let mut inside = base.to_vec();
            inside.push(name.clone());
            path.push(name.clone());
            let outcome = if child.dataset(layout::SOURCE_TS).is_ok() {
                self.series(&child, kind, path, place, &inside, extra)
            } else {
                self.walk(&child, kind, path, place, &inside, extra, origins)
            };
            path.pop();
            match outcome {
                Ok(()) => {}
                Err(error) if is_about_the_content(&error) => {
                    // reported on itself: the series is not described in this place, and what its group holds is kept
                    origins.record(
                        format!("events {}", inside.join("/")),
                        Origin::not_read(&error),
                    );
                    let parent = base.to_vec();
                    keep_node(group, name, &parent, extra)?;
                }
                Err(error) => return Err(error),
            }
        }
        // what an events group holds that is not a group below it: a dataset, and the attributes of the group
        let below: Vec<&str> = groups.iter().map(String::as_str).collect();
        keep_group_remainder(group, &below, &[], base, extra)
    }

    /// One series in one place: its type, its interval, its records and its height.
    fn series(
        &mut self,
        group: &Group,
        kind: &EventKind,
        path: &[String],
        place: Place,
        base: &[String],
        extra: &mut Extra,
    ) -> Result<(), Error> {
        let times = describe_dataset(
            &group
                .dataset(layout::SOURCE_TS)
                .map_err(|e| from_binding("opening the timestamps of a series", e))?,
        )?;
        let [records] = times.shape.as_slice() else {
            return Err(corrupt(format!(
                "the timestamps of a series are a list, and these have {} dimensions",
                times.shape.len()
            )));
        };
        let records = *records as u64;
        let column = |name: &str| -> Result<Option<u64>, Error> {
            if group.dataset(name).is_err() {
                return Ok(None);
            }
            let shape = describe_dataset(
                &group
                    .dataset(name)
                    .map_err(|e| from_binding("opening a column of a series", e))?,
            )?
            .shape;
            match shape.as_slice() {
                [length] => Ok(Some(*length as u64)),
                other => Err(corrupt(format!(
                    "`{name}` is a list in a series, and this has {} dimensions",
                    other.len()
                ))),
            }
        };
        let values = column(layout::EVENT_VALUE)?;
        let start = column(layout::START_NS)?;
        let stop = column(layout::STOP_NS)?;
        let interval = start.is_some() && stop.is_some();
        let data_type = optional_text(group, layout::EVENT_DATA_TYPE)?
            .map(|declared| EventDataType::from_declared(&declared));
        let height = optional_number(group, layout::Z_HEIGHT)?;

        // a place that disagrees with the series about its type or its interval is not described
        let key = (kind.clone(), path.to_vec());
        if let Some(known) = self.series.get(&key) {
            if known.data_type != data_type || known.interval != interval {
                return Err(corrupt(
                    "the type or the interval of this series differs from the one it has in another place (a series has one)"
                        .to_string(),
                ));
            }
        }
        let mut part = EventPart::new(records);
        if let Some(height) = height {
            part = part.with_height(Quantity::from_file(height, None));
        }
        // columns that are not as long as the timestamps are reported and not repaired
        if let Some(held) = values.filter(|held| *held != records) {
            part.origins.record(
                "values",
                Origin::Other(format!(
                    "the values column holds {held} entries for {records} timestamps"
                )),
            );
        }
        for (what, length) in [("start", start), ("stop", stop)] {
            if let Some(held) = length.filter(|held| *held != records) {
                part.origins.record(
                    what,
                    Origin::Other(format!(
                        "the {what} column holds {held} entries for {records} timestamps"
                    )),
                );
            }
        }
        if start.is_some() != stop.is_some() {
            part.origins.record(
                "interval",
                Origin::Other("only one of the start and the stop is there".into()),
            );
        }

        let series = self.series.entry(key).or_insert_with(|| {
            let mut series = EventSeries::new(kind.clone(), path.to_vec()).with_interval(interval);
            series.data_type = data_type;
            series
        });
        match place {
            Place::Layer(id) => {
                series.layers.insert(id, part);
            }
            Place::Unassigned => series.no_layer = Some(part),
        }

        // what else the series holds: the attributes of its group and of its columns, and any group or dataset that is not known
        let known = [
            layout::SOURCE_TS,
            layout::EVENT_VALUE,
            layout::START_NS,
            layout::STOP_NS,
        ];
        for name in known {
            if group.link_exists(name) {
                if let Ok(dataset) = group.dataset(name) {
                    let mut at = base.to_vec();
                    at.push(name.to_string());
                    keep_dataset_remainder(&dataset, &[], &at, extra)?;
                }
            }
        }
        keep_group_remainder(
            group,
            &known,
            &[layout::EVENT_DATA_TYPE, layout::Z_HEIGHT],
            base,
            extra,
        )
    }

    /// Adds the series to the build, in kind and path order.
    pub fn finish(self, build: &mut BuildMeta) {
        build.events.extend(self.series.into_values());
    }
}

impl Default for Events {
    fn default() -> Self {
        Events::new()
    }
}
