//! The lasers of a build, read layer by layer from a layout value (IP-12 phase 12A, decision D12-1; the reader of IP-13 phase 13D, with the stream rule of IP-07 phase 7A): each laser in each
//! layer, its presence, its time axis and its channels.
//!
//! **What a version says is data, in a [`LaserLayout`]**: the prefix of a laser's group, the channels it holds (where each is and what it is), the attributes that hold a laser's window and
//! its completeness flag (a version with none says so), the attributes it types, and how to find the laser's sample period. **What is the same for every version is here.**
//!
//! * **A laser is a group named for it in a layer** (the prefix and a number). The lasers of a build are the lasers of all its layers, in numeric order; a layer
//!   in which a laser has no group has no entry for it, which is "not recorded" (decision D13-4).
//! * **A laser's stream is as long as its longest channel** (IP-07 phase 7A, decision D07-1): a stream with samples is recorded, one whose channels have none, or that
//!   has no channel at all, is recorded and empty. No one channel decides it: the rule was the power's length, which hid every sample of a laser whose power was not written, and a
//!   writer may omit an empty channel. A channel that is not there is not a difference; **where the channels that are there differ in length, the stored samples carry the origin
//!   `LongestChannel`**, and nothing is cut or padded: each channel keeps its own count. No sample is read: the lengths come from the datasets' descriptors.
//! * **A dataset with a null dataspace is a recorded, empty channel** (decision D07-2), and its origin on the laser's entry for the layer is `NullDataspace`. HDF5 allows a
//!   dataset with a type, no data and no dimension; a dataset of length zero is empty too, with no origin. Neither removes the laser or the channel.
//! * **The time axis is the laser's own**: its window is the start and end times of the laser **as written** (a version or a file may have none, so there is no
//!   window), its stored samples are those of the stream (above), and its sample period is what the layout says it is, with the unit the file gave it. The accuracy is nominal when there is a
//!   period and unknown when there is none (decisions D13-5 and D13-18).
//! * **A channel is a dataset the layout lists**: its type is the type it is stored as; its unit is declared by the layout (J7); the laser state is an enum whose labels nobody has
//!   declared. A channel is described once for the laser, with its presence in each layer where its dataset exists.
//! * **Whatever else a laser's group holds is kept** in the extra of the laser in the layer, and whatever else a layer holds in the layer's.
//! * **A laser that cannot be read is reported on itself and the build still opens** (decision D13-21): the laser in that layer is not recorded, with an origin
//!   that says why, and what its group holds is kept in the layer's extra. A channel that cannot be read is left out of that layer with an origin that says
//!   why. Only damage to the file itself (an I/O error) stops the open.

use std::collections::BTreeMap;

use build_log_model::{
    Accuracy, BuildMeta, ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind, ClockClass,
    EnumLabels, Error, ErrorKind, LaserKey, LaserLayerMeta, LaserMeta, LayerId, LayerMeta, Origin,
    Presence, Quantity, TimeAxis, TimeValue, TimeWindow, Unit, UnitSource,
};
use hdf5::{Dataset, Group};

use super::content::{is_about_the_content, numeric_type};
use super::values::optional_integer;
use crate::hdf5::{
    describe_shape, from_binding, keep_dataset_remainder, keep_group_remainder_named, keep_node,
    keep_other_attributes, keep_other_nodes, Class,
};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// How a version finds a laser's sample period: from the mount and the laser's number. `None` if the file has none.
pub type PeriodFn = fn(&Group, u32) -> Result<Option<Quantity>, Error>;

/// A channel dataset of a version: where it is under a laser's group, and what the adapter says it is.
pub struct ChannelSpec {
    /// The group of channels, under a laser's group, that holds the dataset.
    pub group: &'static str,
    /// The dataset's own name, which is also the channel's key.
    pub dataset: &'static str,
    /// What the channel records.
    pub kind: ChannelKind,
    /// The unit of the stored values, declared by the adapter where every file of the version says the same thing (J7).
    pub unit: Option<&'static str>,
}

/// What a version says about where its lasers and their channels are. Built by the version's adapter; every name in it comes from the adapter's layout module.
pub struct LaserLayout {
    /// How the version is called in a message ("V17"), so that an error names it without this file doing so.
    pub label: &'static str,
    /// The group of the mount that holds the layers.
    pub layers_group: &'static str,
    /// The prefix of a layer's group name, followed by the layer's number.
    pub layer_prefix: &'static str,
    /// The prefix of a laser's group name, followed by the laser's number.
    pub laser_prefix: &'static str,
    /// The channels, in the order that a laser's channels are listed.
    pub channels: Vec<ChannelSpec>,
    /// The attributes of a laser's group that hold its window: the start and the end, as written. A version with none says `None`.
    pub window: Option<(&'static str, &'static str)>,
    /// The attribute that holds the laser's completeness flag in a layer. A version with none says `None`.
    pub completeness: Option<&'static str>,
    /// The attributes of a laser's group that are typed: the rest are kept in the laser's `extra`.
    pub typed_attributes: &'static [&'static str],
    /// How to find a laser's sample period.
    pub period: PeriodFn,
}

impl LaserLayout {
    /// The groups of channels under a laser's group, each once, in the order of the channels.
    fn groups(&self) -> Vec<&'static str> {
        let mut groups: Vec<&'static str> = Vec::new();
        for spec in &self.channels {
            if !groups.contains(&spec.group) {
                groups.push(spec.group);
            }
        }
        groups
    }
}

/// The number of the laser that a group name stands for: the laser prefix and nothing but digits after it. `None` for any other name.
fn laser_number_of(prefix: &str, name: &str) -> Result<Option<u32>, Error> {
    let Some(digits) = name.strip_prefix(prefix) else {
        return Ok(None);
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }
    digits.parse::<u32>().map(Some).map_err(|_| {
        Error::new(ErrorKind::OutOfWidth {
            value: digits.to_string(),
            stored_as: "a laser number (a 32-bit unsigned number)".to_string(),
        })
    })
}

// ---------------------------------------------------------------- the channels of a layout

/// The dataset of a channel under a laser's group, or `None` if the laser has no such group or dataset in this layer.
pub fn channel_dataset(laser: &Group, spec: &ChannelSpec) -> Result<Option<Dataset>, Error> {
    if !laser.link_exists(spec.group) {
        return Ok(None);
    }
    let group = laser
        .group(spec.group)
        .map_err(|e| corrupt(format!("`{}` is not a group: {e}", spec.group)))?;
    if !group.link_exists(spec.dataset) {
        return Ok(None);
    }
    group
        .dataset(spec.dataset)
        .map(Some)
        .map_err(|e| corrupt(format!("`{}` is not a dataset: {e}", spec.dataset)))
}

/// One of the groups of channels of a laser in a layer, **opened once and its names listed once** (IP-07 phase 7E): a channel is there if its name is, so no lookup is made for a
/// channel that is not, and the remainder of the group is what its names hold besides the channels.
struct Holder {
    name: &'static str,
    group: Group,
    names: Vec<String>,
}

/// The group of channels called `name` under a laser, or `None` if the laser has none.
fn open_holder(laser: &Group, name: &'static str) -> Result<Option<Holder>, Error> {
    if !laser.link_exists(name) {
        return Ok(None);
    }
    let group = laser
        .group(name)
        .map_err(|e| corrupt(format!("`{name}` is not a group: {e}")))?;
    let names = group
        .member_names()
        .map_err(|e| from_binding("listing the members of a group", e))?;
    Ok(Some(Holder { name, group, names }))
}

/// The dataset of a channel in the groups that were opened, or `None` if the laser has no such group or dataset in this layer. The same answer as [`channel_dataset`], from names that
/// are already known.
fn holder_dataset(
    holders: &[Option<Holder>],
    spec: &ChannelSpec,
) -> Result<Option<Dataset>, Error> {
    let Some(holder) = holders.iter().flatten().find(|h| h.name == spec.group) else {
        return Ok(None);
    };
    if !holder.names.iter().any(|name| name == spec.dataset) {
        return Ok(None);
    }
    holder
        .group
        .dataset(spec.dataset)
        .map(Some)
        .map_err(|e| corrupt(format!("`{}` is not a dataset: {e}", spec.dataset)))
}

/// What a channel's dataset says: how it is typed, how many samples it holds, and whether its dataspace is null.
struct Facts {
    dtype: ChannelDType,
    samples: u64,
    null: bool,
}

/// What a channel's dataset says. It has to be a one-dimensional dataset of numbers, **or a dataset with a null dataspace, which holds no value** (D07-2).
fn channel_facts(dataset: &Dataset, spec: &ChannelSpec) -> Result<Facts, Error> {
    // only the class and the shape are needed: the full description of a dataset is for the datasets that are kept untyped
    let descriptor = describe_shape(dataset)?;
    if !descriptor.null && descriptor.shape.len() != 1 {
        return Err(corrupt(format!(
            "a channel of {} dimensions: a stream of samples has one",
            descriptor.shape.len()
        )));
    }
    let samples = descriptor.shape.first().map_or(0, |&n| n as u64);
    let dtype = if spec.kind == ChannelKind::LaserState {
        // an enum of whole numbers whose labels the file does not declare (Q2-2); the width is whatever it is stored as
        match descriptor.class {
            Class::Integer { .. } => ChannelDType::Enum {
                labels: EnumLabels::Unknown,
            },
            other => {
                return Err(corrupt(format!(
                    "the laser state stored as {other:?}, which is not a whole number"
                )))
            }
        }
    } else {
        ChannelDType::Numeric(numeric_type(&descriptor.class)?)
    };
    Ok(Facts {
        dtype,
        samples,
        null: descriptor.null,
    })
}

fn descriptor_for(spec: &ChannelSpec, dtype: ChannelDType) -> ChannelDescriptor {
    let key = ChannelKey::new([spec.dataset]);
    let descriptor = match dtype {
        ChannelDType::Enum { labels } => ChannelDescriptor::laser_state(key, labels),
        other => {
            ChannelDescriptor::new(key, spec.kind.clone(), ClockClass::LaserSynchronous, other)
        }
    };
    match spec.unit {
        Some(id) => descriptor.with_unit(
            Unit {
                id: Some(id.to_string()),
                spelling: None,
            },
            UnitSource::DeclaredByAdapter,
        ),
        None => descriptor,
    }
}

// ---------------------------------------------------------------- the lasers of a build

/// The lasers of a build, collected layer by layer.
pub struct Lasers {
    mount: Group,
    layout: LaserLayout,
    lasers: BTreeMap<u32, LaserMeta>,
    periods: BTreeMap<u32, Option<Quantity>>,
}

impl Lasers {
    pub fn new(mount: &Group, layout: LaserLayout) -> Self {
        Lasers {
            mount: mount.clone(),
            layout,
            lasers: BTreeMap::new(),
            periods: BTreeMap::new(),
        }
    }

    /// Makes sure laser `number` is known, and reads its period the first time.
    fn know(&mut self, name: &str, number: u32) -> Result<(), Error> {
        if self.lasers.contains_key(&number) {
            return Ok(());
        }
        let mut laser = LaserMeta::new(LaserKey::new(name, number));
        let period = match (self.layout.period)(&self.mount, number) {
            Ok(period) => period,
            Err(error) if is_about_the_content(&error) => {
                laser
                    .origins
                    .record("sample_period", Origin::not_read(&error));
                None
            }
            Err(error) => return Err(error),
        };
        self.periods.insert(number, period);
        self.lasers.insert(number, laser);
        Ok(())
    }

    /// Reads the lasers of one layer's group into the collection. Returns the names of the lasers' groups, so that the caller can keep what else the layer's group
    /// holds (the images, the events) without keeping them again.
    pub fn read_layer(
        &mut self,
        group: &Group,
        layer: &mut LayerMeta,
    ) -> Result<Vec<String>, Error> {
        let names = group
            .member_names()
            .map_err(|e| from_binding("listing the members of a layer", e))?;
        let mut here: Vec<(String, u32)> = Vec::new();
        for name in &names {
            if let Some(number) = laser_number_of(self.layout.laser_prefix, name)? {
                here.push((name.clone(), number));
            }
        }
        for (name, number) in &here {
            self.know(name, *number)?;
            let period = self.periods.get(number).cloned().flatten();
            let laser = self
                .lasers
                .get_mut(number)
                .expect("the laser was just made known");
            let read = group
                .group(name)
                .map_err(|e| corrupt(format!("`{name}` is not a group: {e}")))
                .and_then(|laser_group| {
                    read_laser_in_layer(
                        &laser_group,
                        layer.id,
                        period.as_ref(),
                        laser,
                        &self.layout,
                    )
                });
            let entry = match read {
                Ok(entry) => entry,
                Err(error) if is_about_the_content(&error) => {
                    // reported on itself: not recorded, with the reason, and what the group holds is kept
                    let mut entry = LaserLayerMeta::new(Presence::NotRecorded);
                    entry.origins.record("laser", Origin::not_read(&error));
                    keep_node(group, name, &[], &mut layer.extra)?;
                    entry
                }
                Err(error) => return Err(error),
            };
            if laser.layers.insert(layer.id, entry).is_some() {
                return Err(corrupt(format!(
                    "two groups are named for the laser {number} in the layer {}",
                    layer.id
                )));
            }
        }
        Ok(here.into_iter().map(|(name, _)| name).collect())
    }

    /// Adds the lasers to the build, in numeric order, with their channels in the order of the layout.
    pub fn finish(self, build: &mut BuildMeta) {
        let Lasers { layout, lasers, .. } = self;
        for (_, mut laser) in lasers {
            laser
                .channels
                .sort_by_key(|c| layout.channels.iter().position(|s| s.kind == c.kind));
            build.add_laser(laser);
        }
    }
}

/// One laser in one layer: its presence, its time axis, its channels (merged into the laser's) and, in `extra`, what else its group holds.
fn read_laser_in_layer(
    group: &Group,
    layer: LayerId,
    period: Option<&Quantity>,
    laser: &mut LaserMeta,
    layout: &LaserLayout,
) -> Result<LaserLayerMeta, Error> {
    let (start, end) = match layout.window {
        Some((start, end)) => (
            optional_integer(group, start)?,
            optional_integer(group, end)?,
        ),
        None => (None, None),
    };
    let flag = read_complete(group, layout.completeness)?;
    let all = &layout.channels;
    // what each channel says is read first, because no one channel decides the length of the stream (D07-1): it is that of the longest of them, and a stream with no channel is empty.
    // A channel that cannot be read is left out of the length, with the reason (D13-21); a channel that is not there is not a difference.
    let group_names = layout.groups();
    let mut holders = Vec::new();
    for &name in &group_names {
        holders.push(open_holder(group, name)?);
    }
    let mut read: Vec<(&ChannelSpec, Dataset, Result<Facts, Error>)> = Vec::new();
    for spec in all {
        let Some(dataset) = holder_dataset(&holders, spec)? else {
            continue;
        };
        let facts = match channel_facts(&dataset, spec) {
            Ok(facts) => Ok(facts),
            Err(error) if is_about_the_content(&error) => Err(error),
            Err(error) => return Err(error),
        };
        read.push((spec, dataset, facts));
    }
    let counts: Vec<u64> = read
        .iter()
        .filter_map(|(_, _, facts)| facts.as_ref().ok().map(|f| f.samples))
        .collect();
    let samples = counts.iter().copied().max().unwrap_or(0);
    let mut axis = TimeAxis::new()
        .with_accuracy(if period.is_some() {
            Accuracy::Nominal
        } else {
            Accuracy::Unknown
        })
        .with_stored_samples(samples);
    if let (Some(start), Some(end)) = (start, end) {
        axis = axis.with_window(TimeWindow {
            start: TimeValue::from_file(start, None),
            end: TimeValue::from_file(end, None),
        });
    }
    if let Some(period) = period {
        axis = axis.with_period(period.clone());
    }
    let mut entry = LaserLayerMeta::new(Presence::from_count(samples))
        .with_time_axis(axis)
        .with_complete(flag);
    // where the channels that are there differ in length, the stream's samples are those of the longest, and the origin says so (D07-1)
    if counts.iter().any(|&count| count != samples) {
        entry
            .origins
            .record("stored_samples", Origin::LongestChannel);
    }

    // what each channel says is merged into the laser only when the whole group has been read, so that a laser that turns out to be unreadable leaves nothing of itself in the
    // laser's channels
    let mut found: Vec<(&ChannelSpec, ChannelDType, u64)> = Vec::new();
    for (spec, dataset, facts) in read {
        match facts {
            Ok(facts) => {
                // a dataset with a null dataspace is recorded and empty, and says so (D07-2)
                if facts.null {
                    entry.record_channel_origin(spec.dataset, Origin::NullDataspace);
                }
                found.push((spec, facts.dtype, facts.samples));
            }
            Err(error) => {
                entry.record_channel_origin(spec.dataset, Origin::not_read(&error));
            }
        }
        keep_dataset_remainder(
            &dataset,
            &[],
            &[spec.group.to_string(), spec.dataset.to_string()],
            &mut entry.extra,
        )?;
    }

    // what else the group holds: its attributes, its children that are not the groups of channels, and what the groups of channels hold besides
    // the channels
    keep_other_attributes(group, layout.typed_attributes, &mut entry.extra)?;
    keep_other_nodes(group, &group_names, &[], &mut entry.extra)?;
    for holder in holders.iter().flatten() {
        let typed: Vec<&str> = all
            .iter()
            .filter(|s| s.group == holder.name)
            .map(|s| s.dataset)
            .collect();
        keep_group_remainder_named(
            &holder.group,
            &holder.names,
            &typed,
            &[],
            &[holder.name.to_string()],
            &mut entry.extra,
        )?;
    }
    for (spec, dtype, count) in found {
        match merge(laser, spec, dtype, count, layer) {
            Ok(()) => {}
            Err(error) if is_about_the_content(&error) => {
                entry.record_channel_origin(spec.dataset, Origin::not_read(&error));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(entry)
}

/// The laser's `complete` flag in a layer, as written: 1 is true and 0 is false, and a laser without the attribute (or a version without one) has none. It is never worked out from the
/// samples (decision J9). Any other value is not a flag, and is not guessed at: it is a fault in the laser's group.
fn read_complete(group: &Group, attribute: Option<&str>) -> Result<Option<bool>, Error> {
    let Some(attribute) = attribute else {
        return Ok(None);
    };
    match optional_integer(group, attribute)? {
        None => Ok(None),
        Some(0) => Ok(Some(false)),
        Some(1) => Ok(Some(true)),
        Some(other) => Err(corrupt(format!(
            "the completeness flag of a laser holds {other}, and only 0 and 1 mean anything"
        ))),
    }
}

/// Adds a channel's presence in one layer to the laser's description of it, describing the channel the first time it is seen. A channel that is stored as
/// another type here than elsewhere is not merged: the description says one type.
fn merge(
    laser: &mut LaserMeta,
    spec: &ChannelSpec,
    dtype: ChannelDType,
    samples: u64,
    layer: LayerId,
) -> Result<(), Error> {
    let at = match laser.channels.iter().position(|c| c.kind == spec.kind) {
        Some(at) => at,
        None => {
            laser.channels.push(descriptor_for(spec, dtype.clone()));
            laser.channels.len() - 1
        }
    };
    let channel = &mut laser.channels[at];
    if channel.dtype != dtype {
        return Err(corrupt(format!(
            "the channel is stored as {dtype:?} here and as {:?} in other layers",
            channel.dtype
        )));
    }
    channel
        .presence
        .insert(layer, Presence::from_count(samples));
    Ok(())
}
