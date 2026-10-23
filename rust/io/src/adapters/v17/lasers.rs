//! The lasers of a V17 build (IP-13 phase 13D): each laser in each layer, its presence, its time axis and its channels.
//!
//! * **A laser is a group named for it in a layer** (`Laser_` and a number). The lasers of a build are the lasers of all its layers, in numeric order; a layer
//!   in which a laser has no group has no entry for it, which is "not recorded" (decision D13-4).
//! * **Presence is the length of the laser's power**: a stream with samples is recorded, one whose power has none, or that has no power at all, is recorded and
//!   empty (the second laser of the 2026-10-15 file; layers 2007 and 2008 of the special-layer miniature). No sample is read: the length comes from the
//!   dataset's descriptor.
//! * **The time axis is the laser's own**: its window is the start and end times of the laser **as written** (absent from the March files, so there is no
//!   window), its stored samples are the length of the power, and its sample period is the scanner card's of the optical train **of the same number**, with no
//!   unit. The accuracy is nominal when there is a period and unknown when there is none (decisions D13-5 and D13-18).
//! * **A channel is a dataset the adapter knows**: the power, the state and the six positions. Its type is the type it is stored as; the unit of the power
//!   (volts) and of the positions (bits) is declared by the adapter (J7); the state is an enum whose labels nobody has declared. A channel is described once for
//!   the laser, with its presence in each layer where its dataset exists.
//! * **Whatever else a laser's group holds is kept** in the extra of the laser in the layer, and whatever else a layer holds in the layer's.
//! * **A laser that cannot be read is reported on itself and the build still opens** (decision D13-21): the laser in that layer is not recorded, with an origin
//!   that says why, and what its group holds is kept in the layer's extra. A channel that cannot be read is left out of that layer with an origin that says
//!   why. Only damage to the file itself (an I/O error) stops the open.

use std::collections::BTreeMap;

use build_log_model::{
    Accuracy, Axis, BuildMeta, ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind,
    ClockClass, EnumLabels, Error, ErrorKind, LaserKey, LaserLayerMeta, LaserMeta, LayerId,
    LayerMeta, NumericType, Origin, Presence, Quantity, TimeAxis, TimeValue, TimeWindow, Unit,
    UnitSource,
};
use hdf5::{Dataset, Group};

use super::layout;
use super::values::{optional_float, optional_integer};
use crate::hdf5::{
    describe_dataset, from_binding, keep_dataset_remainder, keep_group_remainder, keep_node,
    keep_other_attributes, keep_other_nodes, Class,
};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// Can this failure be reported on the laser or the channel it happened in? A file that is not what V17 says it is can; damage to the file itself cannot.
fn is_about_the_content(error: &Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Corrupt { .. }
            | ErrorKind::OutOfWidth { .. }
            | ErrorKind::UnsupportedInVersion { .. }
    )
}

/// The number of the laser that a group name stands for: the laser prefix and nothing but digits after it. `None` for any other name.
fn laser_number_of(name: &str) -> Result<Option<u32>, Error> {
    let Some(digits) = name.strip_prefix(layout::LASER_PREFIX) else {
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

// ---------------------------------------------------------------- the channels the adapter knows

/// A channel dataset of V17: where it is under a laser's group, and what the adapter says it is.
struct Spec {
    group: &'static str,
    dataset: &'static str,
    kind: ChannelKind,
    /// The unit of the stored values, declared by the adapter because every V17 file says the same thing (J7).
    unit: Option<&'static str>,
}

/// The eight channels, in the order that a laser's channels are listed.
fn specs() -> [Spec; 8] {
    let position = |dataset, kind| Spec {
        group: layout::POSITION,
        dataset,
        kind,
        unit: Some("bits"),
    };
    [
        Spec {
            group: layout::LASER_DATA,
            dataset: layout::LASER_POWER,
            kind: ChannelKind::LaserPower,
            unit: Some("V"),
        },
        Spec {
            group: layout::LASER_DATA,
            dataset: layout::LASER_STATE,
            kind: ChannelKind::LaserState,
            unit: None,
        },
        position(
            layout::ACTUAL_X,
            ChannelKind::ActualPosition { axis: Axis::X },
        ),
        position(
            layout::ACTUAL_Y,
            ChannelKind::ActualPosition { axis: Axis::Y },
        ),
        position(
            layout::ACTUAL_Z,
            ChannelKind::ActualPosition { axis: Axis::Z },
        ),
        position(
            layout::COMMANDED_X,
            ChannelKind::CommandedPosition { axis: Axis::X },
        ),
        position(
            layout::COMMANDED_Y,
            ChannelKind::CommandedPosition { axis: Axis::Y },
        ),
        position(
            layout::COMMANDED_Z,
            ChannelKind::CommandedPosition { axis: Axis::Z },
        ),
    ]
}

/// The dataset of a channel under a laser's group, or `None` if the laser has no such group or dataset in this layer.
fn channel_dataset(laser: &Group, spec: &Spec) -> Result<Option<Dataset>, Error> {
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

/// The numeric type that a class is, if it is one.
fn numeric_type(class: &Class) -> Result<NumericType, Error> {
    match class {
        Class::Float { bits: 32, .. } => Ok(NumericType::F32),
        Class::Float { bits: 64, .. } => Ok(NumericType::F64),
        Class::Integer {
            bits: 8,
            signed: true,
            ..
        } => Ok(NumericType::I8),
        Class::Integer {
            bits: 16,
            signed: true,
            ..
        } => Ok(NumericType::I16),
        Class::Integer {
            bits: 32,
            signed: true,
            ..
        } => Ok(NumericType::I32),
        Class::Integer {
            bits: 64,
            signed: true,
            ..
        } => Ok(NumericType::I64),
        Class::Integer {
            bits: 8,
            signed: false,
            ..
        } => Ok(NumericType::U8),
        Class::Integer {
            bits: 16,
            signed: false,
            ..
        } => Ok(NumericType::U16),
        Class::Integer {
            bits: 32,
            signed: false,
            ..
        } => Ok(NumericType::U32),
        Class::Integer {
            bits: 64,
            signed: false,
            ..
        } => Ok(NumericType::U64),
        other => Err(corrupt(format!(
            "a channel stored as {other:?}, which is not a number"
        ))),
    }
}

/// What a channel's dataset says: how it is typed and how many samples it holds. It has to be a one-dimensional dataset of numbers.
fn channel_facts(dataset: &Dataset, spec: &Spec) -> Result<(ChannelDType, u64), Error> {
    let descriptor = describe_dataset(dataset)?;
    if descriptor.shape.len() != 1 {
        return Err(corrupt(format!(
            "a channel of {} dimensions: a stream of samples has one",
            descriptor.shape.len()
        )));
    }
    let samples = descriptor.shape[0] as u64;
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
    Ok((dtype, samples))
}

fn descriptor_for(spec: &Spec, dtype: ChannelDType) -> ChannelDescriptor {
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
    lasers: BTreeMap<u32, LaserMeta>,
    periods: BTreeMap<u32, Option<Quantity>>,
}

impl Lasers {
    pub fn new(mount: &Group) -> Self {
        Lasers {
            mount: mount.clone(),
            lasers: BTreeMap::new(),
            periods: BTreeMap::new(),
        }
    }

    /// The sample period of laser `number`: the scanner card's of the optical train of the same number, with no unit. `None` if the file has none.
    fn read_period(&self, number: u32) -> Result<Option<Quantity>, Error> {
        let train = format!("{}{number}", layout::OPTICAL_TRAIN_PREFIX);
        let mut group = self.mount.clone();
        for part in [
            layout::CALIBRATION_DATA,
            train.as_str(),
            layout::SCANNER_CARD,
        ] {
            if !group.link_exists(part) {
                return Ok(None);
            }
            group = group
                .group(part)
                .map_err(|e| corrupt(format!("`{part}` is not a group: {e}")))?;
        }
        Ok(optional_float(&group, layout::SAMPLE_PERIOD)?
            .map(|value| Quantity::from_file(value, None)))
    }

    /// Makes sure laser `number` is known, and reads its period the first time.
    fn know(&mut self, name: &str, number: u32) -> Result<(), Error> {
        if self.lasers.contains_key(&number) {
            return Ok(());
        }
        let mut laser = LaserMeta::new(LaserKey::new(name, number));
        let period = match self.read_period(number) {
            Ok(period) => period,
            Err(error) if is_about_the_content(&error) => {
                laser
                    .origins
                    .record("sample_period", Origin::Other(format!("not read: {error}")));
                None
            }
            Err(error) => return Err(error),
        };
        self.periods.insert(number, period);
        self.lasers.insert(number, laser);
        Ok(())
    }

    /// Reads the lasers of one layer's group into the collection, and what else the layer's group holds into the layer's `extra`.
    pub fn read_layer(&mut self, group: &Group, layer: &mut LayerMeta) -> Result<(), Error> {
        let names = group
            .member_names()
            .map_err(|e| from_binding("listing the members of a layer", e))?;
        let mut here: Vec<(String, u32)> = Vec::new();
        for name in &names {
            if let Some(number) = laser_number_of(name)? {
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
                    read_laser_in_layer(&laser_group, layer.id, period.as_ref(), laser)
                });
            let entry = match read {
                Ok(entry) => entry,
                Err(error) if is_about_the_content(&error) => {
                    // reported on itself: not recorded, with the reason, and what the group holds is kept
                    let mut entry = LaserLayerMeta::new(Presence::NotRecorded);
                    entry
                        .origins
                        .record("laser", Origin::Other(format!("not read: {error}")));
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
        let typed: Vec<&str> = here.iter().map(|(name, _)| name.as_str()).collect();
        keep_other_nodes(group, &typed, &[], &mut layer.extra)
    }

    /// Adds the lasers to the build, in numeric order, with their channels in the order of the specification.
    pub fn finish(self, build: &mut BuildMeta) {
        let order = specs();
        for (_, mut laser) in self.lasers {
            laser
                .channels
                .sort_by_key(|c| order.iter().position(|s| s.kind == c.kind));
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
) -> Result<LaserLayerMeta, Error> {
    let start = optional_integer(group, layout::START_LASER_TIME)?;
    let end = optional_integer(group, layout::END_LASER_TIME)?;
    let all = specs();
    // the length of the stream is the length of the power; a stream without one is empty
    let samples = match channel_dataset(group, &all[0])? {
        Some(power) => channel_facts(&power, &all[0])?.1,
        None => 0,
    };
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
    let mut entry = LaserLayerMeta::new(Presence::from_count(samples)).with_time_axis(axis);

    // what each channel says is collected first, and merged into the laser only when the whole group has been read, so that a laser that turns out to be
    // unreadable leaves nothing of itself in the laser's channels
    let mut found: Vec<(&Spec, ChannelDType, u64)> = Vec::new();
    for spec in &all {
        let Some(dataset) = channel_dataset(group, spec)? else {
            continue;
        };
        match channel_facts(&dataset, spec) {
            Ok((dtype, count)) => found.push((spec, dtype, count)),
            Err(error) if is_about_the_content(&error) => {
                entry.origins.record(
                    format!("channel {}", spec.dataset),
                    Origin::Other(format!("not read: {error}")),
                );
            }
            Err(error) => return Err(error),
        }
        keep_dataset_remainder(
            &dataset,
            &[],
            &[spec.group.to_string(), spec.dataset.to_string()],
            &mut entry.extra,
        )?;
    }

    // what else the group holds: its attributes, its children that are not the two groups of channels, and what the two groups of channels hold besides
    // the channels
    keep_other_attributes(group, &layout::TYPED_ON_A_LASER, &mut entry.extra)?;
    keep_other_nodes(
        group,
        &[layout::LASER_DATA, layout::POSITION],
        &[],
        &mut entry.extra,
    )?;
    for holder in [layout::LASER_DATA, layout::POSITION] {
        if group.link_exists(holder) {
            let typed: Vec<&str> = all
                .iter()
                .filter(|s| s.group == holder)
                .map(|s| s.dataset)
                .collect();
            let inner = group
                .group(holder)
                .map_err(|e| corrupt(format!("`{holder}` is not a group: {e}")))?;
            keep_group_remainder(&inner, &typed, &[], &[holder.to_string()], &mut entry.extra)?;
        }
    }
    for (spec, dtype, count) in found {
        match merge(laser, spec, dtype, count, layer) {
            Ok(()) => {}
            Err(error) if is_about_the_content(&error) => {
                entry.origins.record(
                    format!("channel {}", spec.dataset),
                    Origin::Other(format!("not read: {error}")),
                );
            }
            Err(error) => return Err(error),
        }
    }
    Ok(entry)
}

/// Adds a channel's presence in one layer to the laser's description of it, describing the channel the first time it is seen. A channel that is stored as
/// another type here than elsewhere is not merged: the description says one type.
fn merge(
    laser: &mut LaserMeta,
    spec: &Spec,
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
