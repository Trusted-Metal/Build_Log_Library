//! Availability: what a consumer can do with each layer, each laser in a layer and each channel in a layer (IP-07 phase 7D, decisions D07-3 and D07-4; requirements BL-FA-09 and BL-CD-04,
//! Vision's QE-04).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** library (`ItemAvailability`, `AvailabilityCounts`, `AvailabilityReport`); the enums `Item` and `ItemKind`.
//! * **Requirement:** BL-FA-09 (each channel or laser carries Present, NotRecorded, Degraded, Unreadable or Unsupported; the outcome is on the result, as counts), BL-CD-04 (explicit presence).
//!
//! # What this module claims, and what it does not
//!
//! * **Availability is derived, not stored (D07-4).** An item's state comes from four things and from nothing else: its **presence** in the metadata; the **typed cause** of a failed read
//!   ([`Origin::NotRead`](crate::Origin::NotRead)); the **warnings** among the findings that name it (phase 7C); and **what the source can read**, which the caller passes as a function
//!   (D07-3: the column engine's one list). The mapping: recorded, with samples or empty, is `Present`; not in the file is `NotRecorded`; a warning at the item is `Degraded`, with the
//!   findings as its issues; a fault that was not read is `Unreadable`, with the error; a cause that is the library's, or a type the source cannot read, is `Unsupported`, naming the feature.
//!   **The strongest state wins**: unreadable, then unsupported, then degraded.
//! * **One item never changes another.** A laser that cannot be read in a layer is unreadable there and nowhere else, and its neighbours are as they were.
//! * **The report is compact.** A build of 2,110 layers has about 40,000 items, and a list of 40,000 copies of the word `Present` is megabytes. It holds the **counts** of each state, for each kind of item
//!   ([`AvailabilityReport::counts_for`]) and overall ([`AvailabilityReport::counts`]), and the list of the items whose state their presence does not decide (degraded, unreadable, unsupported:
//!   [`AvailabilityReport::exceptions`]); **[`AvailabilityReport::status`] answers for any item**.
//! * **What is counted:** every layer; every laser in every layer; every channel in the layers where its laser is recorded (a channel in a layer where its laser is not is not counted again).
//!   **The kinds of item are open**: sensors, event places and images are added when a source needs them (D07-4, risk R6).
//!
//! The policy guards in `tests/policy_guards.rs` scan this file.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::channel::ChannelDescriptor;
use crate::error::{Error, ErrorKind, Location, Violation};
use crate::identity::{Availability, ChannelKey, LaserKey, LayerId, Presence};
use crate::meta::BuildMeta;
use crate::provenance::{channel_of_key, NotReadCause, Origin};
use crate::validation::{Severity, ValidationReport};

/// The kind of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ItemKind {
    Layer,
    Laser,
    Channel,
}

/// Something a state is told of: a layer, a laser in a layer, or a channel in a layer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Item {
    /// A layer of the build.
    Layer(LayerId),
    /// A laser in a layer.
    Laser { layer: LayerId, laser: LaserKey },
    /// A channel in a layer: of a laser, or (`laser` is `None`) of the build.
    Channel {
        layer: LayerId,
        laser: Option<LaserKey>,
        channel: ChannelKey,
    },
}

impl Item {
    pub fn kind(&self) -> ItemKind {
        match self {
            Item::Layer(_) => ItemKind::Layer,
            Item::Laser { .. } => ItemKind::Laser,
            Item::Channel { .. } => ItemKind::Channel,
        }
    }

    /// Where the item is, as an error and a finding say it.
    pub fn location(&self) -> Location {
        match self {
            Item::Layer(layer) => Location::new().with_layer(*layer),
            Item::Laser { layer, laser } => {
                Location::new().with_layer(*layer).with_laser(laser.clone())
            }
            Item::Channel {
                layer,
                laser,
                channel,
            } => {
                let at = Location::new()
                    .with_layer(*layer)
                    .with_channel(channel.clone());
                match laser {
                    Some(laser) => at.with_laser(laser.clone()),
                    None => at,
                }
            }
        }
    }
}

/// An item and its state.
// constructed-by: library
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ItemAvailability {
    pub item: Item,
    pub availability: Availability,
}

impl ItemAvailability {
    pub fn new(item: Item, availability: Availability) -> Self {
        ItemAvailability { item, availability }
    }
}

/// How many items are in each state.
// constructed-by: library
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct AvailabilityCounts {
    pub present: u64,
    pub not_recorded: u64,
    pub degraded: u64,
    pub unreadable: u64,
    pub unsupported: u64,
}

impl AvailabilityCounts {
    /// How many items there are, in all states.
    pub fn total(&self) -> u64 {
        self.present + self.not_recorded + self.degraded + self.unreadable + self.unsupported
    }

    fn add(&mut self, state: &Availability) {
        match state {
            Availability::Present => self.present += 1,
            Availability::NotRecorded => self.not_recorded += 1,
            Availability::Degraded { .. } => self.degraded += 1,
            Availability::Unreadable { .. } => self.unreadable += 1,
            Availability::Unsupported { .. } => self.unsupported += 1,
        }
    }

    fn sum(&self, other: &AvailabilityCounts) -> AvailabilityCounts {
        AvailabilityCounts {
            present: self.present + other.present,
            not_recorded: self.not_recorded + other.not_recorded,
            degraded: self.degraded + other.degraded,
            unreadable: self.unreadable + other.unreadable,
            unsupported: self.unsupported + other.unsupported,
        }
    }
}

/// What a source says it can do with each item of its build: counts of each state, the items that are not simply present or not recorded, and a state for any item that is asked about.
// constructed-by: library
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AvailabilityReport {
    build: Arc<BuildMeta>,
    by_kind: [AvailabilityCounts; 3],
    exceptions: Vec<ItemAvailability>,
    index: BTreeMap<Item, usize>,
}

fn kind_index(kind: ItemKind) -> usize {
    match kind {
        ItemKind::Layer => 0,
        ItemKind::Laser => 1,
        ItemKind::Channel => 2,
    }
}

impl AvailabilityReport {
    /// A report of a build with nothing counted yet.
    pub fn new(build: Arc<BuildMeta>) -> Self {
        AvailabilityReport {
            build,
            by_kind: [AvailabilityCounts::default(); 3],
            exceptions: Vec::new(),
            index: BTreeMap::new(),
        }
    }

    /// The counts of one kind of item.
    pub fn counts_for(&self, kind: ItemKind) -> &AvailabilityCounts {
        &self.by_kind[kind_index(kind)]
    }

    /// The counts of every item, of all kinds together.
    pub fn counts(&self) -> AvailabilityCounts {
        self.by_kind
            .iter()
            .fold(AvailabilityCounts::default(), |all, kind| all.sum(kind))
    }

    /// The items that are degraded, unreadable or unsupported, in the order of the items. Every other item is present or not recorded, and its presence says which.
    pub fn exceptions(&self) -> &[ItemAvailability] {
        &self.exceptions
    }

    /// The state of any item: its entry among the exceptions, or what its presence in the metadata says.
    pub fn status(&self, item: &Item) -> Availability {
        if let Some(&at) = self.index.get(item) {
            return self.exceptions[at].availability.clone();
        }
        presence_state(&self.build, item)
    }
}

/// What a presence says of an item: present where something is recorded (an empty stream too), and not recorded where the metadata holds nothing or says so.
fn recorded(presence: Option<&Presence>) -> Availability {
    match presence {
        Some(Presence::NotRecorded) | None => Availability::NotRecorded,
        Some(_) => Availability::Present,
    }
}

/// What the presence of an item in the metadata says of it.
fn presence_state(build: &BuildMeta, item: &Item) -> Availability {
    match item {
        Item::Layer(id) => {
            if build.layers.contains(*id) {
                Availability::Present
            } else {
                Availability::NotRecorded
            }
        }
        Item::Laser { layer, laser } => recorded(
            build
                .laser(laser)
                .and_then(|l| l.layers.get(*layer))
                .map(|entry| &entry.presence),
        ),
        Item::Channel {
            layer,
            laser: Some(laser),
            channel,
        } => recorded(
            build
                .laser(laser)
                .and_then(|l| l.channels.iter().find(|c| &c.key == channel))
                .and_then(|c| c.presence.get(*layer)),
        ),
        Item::Channel {
            layer,
            laser: None,
            channel,
        } => recorded(
            build
                .sensors
                .iter()
                .find(|c| &c.key == channel)
                .and_then(|c| c.presence.get(*layer)),
        ),
    }
}

/// How strong a state is: when two things are said of one item, the stronger is its state.
fn rank(state: &Availability) -> u8 {
    match state {
        Availability::Unreadable { .. } => 3,
        Availability::Unsupported { .. } => 2,
        Availability::Degraded { .. } => 1,
        _ => 0,
    }
}

/// Says `new` of `item`: it is kept if it is stronger than what was said, and two degradations put their issues together.
fn put(said: &mut BTreeMap<Item, Availability>, item: Item, new: Availability) {
    match said.get_mut(&item) {
        None => {
            said.insert(item, new);
        }
        Some(old) => {
            if let (Availability::Degraded { issues }, Availability::Degraded { issues: more }) =
                (&mut *old, &new)
            {
                issues.extend(more.iter().cloned());
            } else if rank(&new) > rank(old) {
                *old = new;
            }
        }
    }
}

/// The availability of every item of a build (the module documentation says how).
///
/// `validation` is the report of phase 7C: its warnings degrade the items they name. `cannot_read` is what the source cannot read: it is asked of each channel and answers with the feature that
/// is missing, such as `a column stored as a 16-bit unsigned integer`, or `None` where the channel can be read. A source that can read everything passes `&|_| None`.
pub fn availability_of(
    build: &Arc<BuildMeta>,
    validation: &ValidationReport,
    cannot_read: &dyn Fn(&ChannelDescriptor) -> Option<String>,
) -> AvailabilityReport {
    let mut said: BTreeMap<Item, Availability> = BTreeMap::new();

    // what was not read, on the laser's entry for the layer: the laser itself, or one of its channels
    for laser in build.lasers() {
        for (layer, entry) in laser.layers.iter() {
            for (field, origin) in entry.origins.iter() {
                let Origin::NotRead { cause, reason } = origin else {
                    continue;
                };
                let (item, at) = if field == "laser" {
                    let item = Item::Laser {
                        layer,
                        laser: laser.key.clone(),
                    };
                    let at = item.location();
                    (item, at)
                } else if let Some(name) = channel_of_key(field) {
                    let item = Item::Channel {
                        layer,
                        laser: Some(laser.key.clone()),
                        channel: ChannelKey::new([name]),
                    };
                    let at = item.location();
                    (item, at)
                } else {
                    continue;
                };
                let state = match cause {
                    NotReadCause::Unsupported => Availability::Unsupported {
                        feature: reason.clone(),
                    },
                    _ => Availability::Unreadable {
                        error: Box::new(
                            Error::new(ErrorKind::Corrupt {
                                detail: reason.clone(),
                            })
                            .at(at),
                        ),
                    },
                };
                put(&mut said, item, state);
            }
        }
    }

    // a type that the source cannot read, in each layer where the channel is recorded
    for laser in build.lasers() {
        for channel in &laser.channels {
            let Some(feature) = cannot_read(channel) else {
                continue;
            };
            for (layer, presence) in channel.presence.iter() {
                if *presence == Presence::NotRecorded {
                    continue;
                }
                put(
                    &mut said,
                    Item::Channel {
                        layer,
                        laser: Some(laser.key.clone()),
                        channel: channel.key.clone(),
                    },
                    Availability::Unsupported {
                        feature: feature.clone(),
                    },
                );
            }
        }
    }

    // a warning at an item degrades it
    for finding in validation
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
    {
        let location = &finding.location;
        let Some(layer) = location.layer else {
            continue;
        };
        let item = match (&location.laser, &location.channel) {
            (Some(laser), Some(channel)) => Item::Channel {
                layer,
                laser: Some(laser.clone()),
                channel: channel.clone(),
            },
            (Some(laser), None) => Item::Laser {
                layer,
                laser: laser.clone(),
            },
            _ => Item::Layer(layer),
        };
        put(
            &mut said,
            item,
            Availability::Degraded {
                issues: vec![Violation::new(finding.message.clone()).at(location.clone())],
            },
        );
    }

    // the counts: every layer, every laser in every layer, every channel in the layers where its laser is recorded
    let mut report = AvailabilityReport::new(Arc::clone(build));
    let mut counted: BTreeSet<Item> = BTreeSet::new();
    let nothing_said = said.is_empty();
    // the state of an item: what was said of it, or what its presence says (the item is only made if something was said)
    let state_of = |make: &dyn Fn() -> Item, base: Availability, counted: &mut BTreeSet<Item>| {
        if nothing_said {
            return base;
        }
        let item = make();
        match said.get(&item) {
            Some(state) => {
                counted.insert(item);
                state.clone()
            }
            None => base,
        }
    };
    let mut counts = [AvailabilityCounts::default(); 3];
    for (layer, _) in build.layers.iter() {
        let state = state_of(&|| Item::Layer(layer), Availability::Present, &mut counted);
        counts[kind_index(ItemKind::Layer)].add(&state);
    }
    for laser in build.lasers() {
        for (layer, _) in build.layers.iter() {
            let entry = laser.layers.get(layer);
            let base = recorded(entry.map(|e| &e.presence));
            let is_recorded = matches!(base, Availability::Present);
            let state = state_of(
                &|| Item::Laser {
                    layer,
                    laser: laser.key.clone(),
                },
                base,
                &mut counted,
            );
            counts[kind_index(ItemKind::Laser)].add(&state);
            if !is_recorded {
                continue;
            }
            for channel in &laser.channels {
                let base = recorded(channel.presence.get(layer));
                let state = state_of(
                    &|| Item::Channel {
                        layer,
                        laser: Some(laser.key.clone()),
                        channel: channel.key.clone(),
                    },
                    base,
                    &mut counted,
                );
                counts[kind_index(ItemKind::Channel)].add(&state);
            }
        }
    }
    // what was said of an item that the metadata does not otherwise hold (a channel that was not read in any layer is not described at all) is counted too
    for (item, state) in &said {
        if !counted.contains(item) {
            counts[kind_index(item.kind())].add(state);
        }
    }
    report.by_kind = counts;
    for (item, state) in said {
        report.index.insert(item.clone(), report.exceptions.len());
        report.exceptions.push(ItemAvailability::new(item, state));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelDType, ChannelKind, ClockClass, NumericType};
    use crate::column::{
        BuildSource, ColumnData, ColumnRequest, ColumnSource, ReadContext, Revision,
    };
    use crate::identity::ByLayer;
    use crate::meta::{BuildIdentity, LaserLayerMeta, LaserMeta, LayerMeta};
    use crate::time::TimeValue;
    use crate::validation::validate_build;

    fn build() -> BuildMeta {
        BuildMeta::new(BuildIdentity {
            build_id: Some("a build".into()),
            date: None,
            username: None,
            alloy: None,
            part_count: None,
            project_id: None,
            source_mcf_name: None,
            source_mcf_hash: None,
        })
    }

    fn channel(name: &str, numeric: NumericType, samples: &[(u32, u64)]) -> ChannelDescriptor {
        let mut presence = ByLayer::new();
        for &(layer, count) in samples {
            presence.insert(LayerId(layer), Presence::from_count(count));
        }
        ChannelDescriptor::new(
            ChannelKey::new([name]),
            ChannelKind::LaserPower,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(numeric),
        )
        .with_presence(presence)
    }

    fn laser(number: u32, layers: &[(u32, u64)], channels: Vec<ChannelDescriptor>) -> LaserMeta {
        let mut laser = LaserMeta::new(LaserKey::new(format!("laser-{number}"), number));
        for &(id, samples) in layers {
            laser.layers.insert(
                LayerId(id),
                LaserLayerMeta::new(Presence::from_count(samples)),
            );
        }
        laser.channels = channels;
        laser
    }

    /// Two layers; laser 1 in both with channels A (whole) and B (recorded in layer 1 only); laser 2 in layer 1 only, with no channel.
    fn small() -> BuildMeta {
        let mut build = build();
        for (id, start, end) in [(1u32, 0i64, 10i64), (2, 20, 30)] {
            let mut layer = LayerMeta::new(LayerId(id));
            layer.start = Some(TimeValue::from_file(start, None));
            layer.end = Some(TimeValue::from_file(end, None));
            build.layers.insert(LayerId(id), layer);
        }
        build.add_laser(laser(
            1,
            &[(1, 100), (2, 100)],
            vec![
                channel("A", NumericType::F32, &[(1, 100), (2, 100)]),
                channel("B", NumericType::I32, &[(1, 100)]),
            ],
        ));
        let mut two = laser(2, &[(1, 100)], vec![]);
        two.layers
            .insert(LayerId(2), LaserLayerMeta::new(Presence::NotRecorded));
        build.add_laser(two);
        build
    }

    fn report_of(
        build: BuildMeta,
        cannot_read: &dyn Fn(&ChannelDescriptor) -> Option<String>,
    ) -> AvailabilityReport {
        let build = Arc::new(build);
        availability_of(&build, &validate_build(&build), cannot_read)
    }

    fn laser_key(number: u32) -> LaserKey {
        LaserKey::new(format!("laser-{number}"), number)
    }

    fn channel_item(layer: u32, number: u32, name: &str) -> Item {
        Item::Channel {
            layer: LayerId(layer),
            laser: Some(laser_key(number)),
            channel: ChannelKey::new([name]),
        }
    }

    fn laser_item(layer: u32, number: u32) -> Item {
        Item::Laser {
            layer: LayerId(layer),
            laser: laser_key(number),
        }
    }

    #[test]
    fn a_recorded_channel_is_present_and_an_unrecorded_one_is_not_recorded() {
        let report = report_of(small(), &|_| None);
        assert!(matches!(
            report.status(&Item::Layer(LayerId(1))),
            Availability::Present
        ));
        assert!(matches!(
            report.status(&channel_item(1, 1, "A")),
            Availability::Present
        ));
        // B is recorded in layer 1 only, and laser 1 is recorded in layer 2: B is not recorded there
        assert!(matches!(
            report.status(&channel_item(2, 1, "B")),
            Availability::NotRecorded
        ));
        // laser 2 is recorded in layer 1 and is not in layer 2; a layer or a laser that the build does not hold is not recorded
        assert!(matches!(
            report.status(&laser_item(1, 2)),
            Availability::Present
        ));
        assert!(matches!(
            report.status(&laser_item(2, 2)),
            Availability::NotRecorded
        ));
        assert!(matches!(
            report.status(&Item::Layer(LayerId(9))),
            Availability::NotRecorded
        ));
        assert!(matches!(
            report.status(&laser_item(1, 7)),
            Availability::NotRecorded
        ));
        assert!(report.exceptions().is_empty());
    }

    #[test]
    fn the_counts_aggregate_the_states_overall_and_by_kind() {
        let report = report_of(small(), &|_| None);
        // two layers; four laser-layers (two lasers in two layers), three recorded; channels only where the laser is recorded: laser 1 in two layers has A and B, laser 2 has none
        assert_eq!(
            *report.counts_for(ItemKind::Layer),
            AvailabilityCounts {
                present: 2,
                ..Default::default()
            }
        );
        assert_eq!(
            *report.counts_for(ItemKind::Laser),
            AvailabilityCounts {
                present: 3,
                not_recorded: 1,
                ..Default::default()
            }
        );
        assert_eq!(
            *report.counts_for(ItemKind::Channel),
            AvailabilityCounts {
                present: 3,
                not_recorded: 1,
                ..Default::default()
            }
        );
        let all = report.counts();
        assert_eq!((all.present, all.not_recorded, all.total()), (8, 2, 10));
        assert_eq!(
            all.total(),
            2 + 4 + 4,
            "two layers, four laser-layers and four channel-layers"
        );
    }

    #[test]
    fn a_channel_with_a_warning_is_degraded_and_lists_its_issues() {
        let mut build = small();
        // B holds 40 of the 100 samples of the stream in layer 1
        let laser = build.laser_mut(&laser_key(1)).unwrap();
        laser.channels[1]
            .presence
            .insert(LayerId(1), Presence::from_count(40));
        let report = report_of(build, &|_| None);
        match report.status(&channel_item(1, 1, "B")) {
            Availability::Degraded { issues } => {
                assert_eq!(issues.len(), 1);
                assert!(issues[0].message.contains("40") && issues[0].message.contains("100"));
                assert_eq!(issues[0].location.channel, Some(ChannelKey::new(["B"])));
            }
            other => panic!("expected Degraded, got {other:?}"),
        }
        // and only B: the other channel, the laser and the layer are as they were
        assert!(matches!(
            report.status(&channel_item(1, 1, "A")),
            Availability::Present
        ));
        assert!(matches!(
            report.status(&laser_item(1, 1)),
            Availability::Present
        ));
        assert_eq!(report.exceptions().len(), 1);
        assert_eq!(report.counts().degraded, 1);
        // a layer that overlaps the last is degraded
        let mut build = small();
        build.layers.get_mut(LayerId(2)).unwrap().start = Some(TimeValue::from_file(5, None));
        let report = report_of(build, &|_| None);
        assert!(matches!(
            report.status(&Item::Layer(LayerId(2))),
            Availability::Degraded { .. }
        ));
        assert_eq!(report.counts_for(ItemKind::Layer).degraded, 1);
    }

    #[test]
    fn two_warnings_at_one_item_are_one_degradation_with_both_issues() {
        // layer 2 ends before it starts, and starts before layer 1 ended: two findings at the one layer
        let mut build = small();
        let layer = build.layers.get_mut(LayerId(2)).unwrap();
        layer.start = Some(TimeValue::from_file(5, None));
        layer.end = Some(TimeValue::from_file(2, None));
        let report = report_of(build, &|_| None);
        match report.status(&Item::Layer(LayerId(2))) {
            Availability::Degraded { issues } => {
                assert_eq!(issues.len(), 2, "{issues:?}");
                assert!(issues
                    .iter()
                    .any(|i| i.message.contains("before it starts")));
                assert!(issues
                    .iter()
                    .any(|i| i.message.contains("before the last layer")));
            }
            other => panic!("expected Degraded, got {other:?}"),
        }
        // the item is listed once and counted once
        assert_eq!(report.exceptions().len(), 1);
        assert_eq!(report.counts_for(ItemKind::Layer).degraded, 1);
        assert_eq!(report.counts_for(ItemKind::Layer).present, 1);
    }

    #[test]
    fn a_channel_is_counted_only_where_its_laser_is_recorded() {
        // a laser that is recorded in layer 1 and is not in layer 2, with two channels recorded in layer 1: four channel-layers would be counted if the layer where the laser is not recorded were
        let mut build = build();
        for id in [1u32, 2] {
            build
                .layers
                .insert(LayerId(id), LayerMeta::new(LayerId(id)));
        }
        let mut one = laser(
            1,
            &[(1, 10)],
            vec![
                channel("A", NumericType::F32, &[(1, 10)]),
                channel("B", NumericType::F32, &[(1, 10)]),
            ],
        );
        one.layers
            .insert(LayerId(2), LaserLayerMeta::new(Presence::NotRecorded));
        build.add_laser(one);
        let report = report_of(build, &|_| None);
        assert_eq!(report.counts_for(ItemKind::Channel).total(), 2);
        assert_eq!(report.counts_for(ItemKind::Channel).present, 2);
        assert_eq!(report.counts_for(ItemKind::Channel).not_recorded, 0);
        assert_eq!(report.counts_for(ItemKind::Laser).total(), 2);
        // a channel in a layer where its laser is not recorded is still not recorded when it is asked about
        assert!(matches!(
            report.status(&channel_item(2, 1, "A")),
            Availability::NotRecorded
        ));
    }

    #[test]
    fn a_note_degrades_nothing() {
        // layer 2 holds no laser, which is a note, and the build declares nine layers, which is another: neither makes an item degraded
        let mut build = small();
        build.layers_declared = Some(9);
        build.layers.insert(LayerId(3), LayerMeta::new(LayerId(3)));
        let validation = validate_build(&build);
        assert_eq!(validation.counts.warnings, 0);
        assert!(validation.counts.notes >= 2, "{validation:?}");
        let report = report_of(build, &|_| None);
        assert!(report.exceptions().is_empty(), "{:?}", report.exceptions());
        assert_eq!(report.counts().degraded, 0);
        assert!(matches!(
            report.status(&Item::Layer(LayerId(3))),
            Availability::Present
        ));
    }

    #[test]
    fn a_channel_that_the_source_cannot_read_is_unsupported_only_where_it_is_recorded() {
        // B is stored as 32-bit integers, which the source cannot read; the metadata says explicitly that it is not recorded in layer 2
        let mut build = small();
        build.laser_mut(&laser_key(1)).unwrap().channels[1]
            .presence
            .insert(LayerId(2), Presence::NotRecorded);
        let cannot = |c: &ChannelDescriptor| {
            (c.key == ChannelKey::new(["B"])).then(|| "a column stored as I32".to_string())
        };
        let report = report_of(build, &cannot);
        assert!(matches!(
            report.status(&channel_item(1, 1, "B")),
            Availability::Unsupported { .. }
        ));
        assert!(matches!(
            report.status(&channel_item(2, 1, "B")),
            Availability::NotRecorded
        ));
        assert_eq!(report.counts_for(ItemKind::Channel).unsupported, 1);
        assert_eq!(report.counts_for(ItemKind::Channel).not_recorded, 1);
        assert_eq!(report.exceptions().len(), 1);
    }

    #[test]
    fn a_channel_not_read_is_unreadable_and_the_others_are_present() {
        let mut build = small();
        let corrupt = Error::new(ErrorKind::Corrupt {
            detail: "a channel of 2 dimensions".into(),
        });
        // A was not read in layer 2 and is left out of it; a laser that cannot be read in a layer has no entry but the reason
        let laser = build.laser_mut(&laser_key(1)).unwrap();
        laser.channels[0].presence = {
            let mut p = ByLayer::new();
            p.insert(LayerId(1), Presence::from_count(100));
            p
        };
        laser
            .layers
            .get_mut(LayerId(2))
            .unwrap()
            .origins
            .record("channel A", Origin::not_read(&corrupt));
        let report = report_of(build, &|_| None);
        match report.status(&channel_item(2, 1, "A")) {
            Availability::Unreadable { error } => {
                assert!(error.to_string().contains("2 dimensions"), "{error}");
                assert_eq!(error.location().layer, Some(LayerId(2)));
                assert_eq!(error.location().laser, Some(laser_key(1)));
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
        assert!(matches!(
            report.status(&channel_item(1, 1, "A")),
            Availability::Present
        ));
        assert!(matches!(
            report.status(&channel_item(1, 1, "B")),
            Availability::Present
        ));
        // it is counted once, as unreadable and not as not recorded, and the build says it is a fault
        assert_eq!(report.counts_for(ItemKind::Channel).unreadable, 1);
        assert_eq!(report.counts_for(ItemKind::Channel).not_recorded, 1);
        assert_eq!(report.counts().total(), 10);
    }

    #[test]
    fn a_bad_laser_changes_no_other_laser() {
        let reference = report_of(small(), &|_| None);
        let mut build = small();
        // laser 2 cannot be read in layer 1: it is not recorded there, with the reason
        let corrupt = Error::new(ErrorKind::Corrupt {
            detail: "the start of the laser is two numbers".into(),
        });
        let two = build.laser_mut(&laser_key(2)).unwrap();
        let mut entry = LaserLayerMeta::new(Presence::NotRecorded);
        entry.origins.record("laser", Origin::not_read(&corrupt));
        two.layers.insert(LayerId(1), entry);
        let report = report_of(build, &|_| None);
        assert!(matches!(
            report.status(&laser_item(1, 2)),
            Availability::Unreadable { .. }
        ));
        // every item of laser 1 and of the layers is exactly as it was
        for item in [
            laser_item(1, 1),
            laser_item(2, 1),
            channel_item(1, 1, "A"),
            channel_item(2, 1, "A"),
            channel_item(1, 1, "B"),
            channel_item(2, 1, "B"),
            Item::Layer(LayerId(1)),
            Item::Layer(LayerId(2)),
        ] {
            assert_eq!(
                format!("{:?}", report.status(&item)),
                format!("{:?}", reference.status(&item)),
                "{item:?}"
            );
        }
        assert_eq!(report.exceptions().len(), 1);
        assert_eq!(report.counts_for(ItemKind::Laser).unreadable, 1);
        assert_eq!(report.counts_for(ItemKind::Laser).present, 2);
    }

    #[test]
    fn a_type_the_engine_cannot_read_is_unsupported_and_names_the_feature() {
        // B is stored as 32-bit integers, which this source says it cannot read; A can be read
        let cannot = |c: &ChannelDescriptor| match c.dtype {
            ChannelDType::Numeric(NumericType::I32) => Some("a column stored as I32".to_string()),
            _ => None,
        };
        let report = report_of(small(), &cannot);
        match report.status(&channel_item(1, 1, "B")) {
            Availability::Unsupported { feature } => assert_eq!(feature, "a column stored as I32"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
        // B is recorded in layer 1 only; where it is not recorded it is not recorded, and A is present
        assert!(matches!(
            report.status(&channel_item(2, 1, "B")),
            Availability::NotRecorded
        ));
        assert!(matches!(
            report.status(&channel_item(1, 1, "A")),
            Availability::Present
        ));
        assert_eq!(report.counts_for(ItemKind::Channel).unsupported, 1);
        // an unsupported item is not readable
        assert!(!report.status(&channel_item(1, 1, "B")).is_readable());
    }

    #[test]
    fn the_strongest_state_wins_and_a_cause_that_is_the_librarys_is_unsupported() {
        let mut build = small();
        let unsupported = Error::new(ErrorKind::UnsupportedInVersion {
            version: "17".into(),
            feature: "a thing".into(),
        });
        let corrupt = Error::new(ErrorKind::Corrupt {
            detail: "damage".into(),
        });
        let laser = build.laser_mut(&laser_key(1)).unwrap();
        // B in layer 1 is short (degraded), cannot be read by the source (unsupported) and was not read (unreadable): the strongest is unreadable
        laser.channels[1]
            .presence
            .insert(LayerId(1), Presence::from_count(40));
        let entry = laser.layers.get_mut(LayerId(1)).unwrap();
        entry
            .origins
            .record("channel B", Origin::not_read(&corrupt));
        // A in layer 2 was not read because the version does not offer it
        let entry = laser.layers.get_mut(LayerId(2)).unwrap();
        entry
            .origins
            .record("channel A", Origin::not_read(&unsupported));
        let report = report_of(build, &|c| {
            (c.key == ChannelKey::new(["B"])).then(|| "a column".to_string())
        });
        assert!(matches!(
            report.status(&channel_item(1, 1, "B")),
            Availability::Unreadable { .. }
        ));
        assert!(matches!(
            report.status(&channel_item(2, 1, "A")),
            Availability::Unsupported { .. }
        ));
        // unsupported is stronger than degraded: with no not-read origin, B in layer 1 is unsupported
        let mut build = small();
        build.laser_mut(&laser_key(1)).unwrap().channels[1]
            .presence
            .insert(LayerId(1), Presence::from_count(40));
        let report = report_of(build, &|c| {
            (c.key == ChannelKey::new(["B"])).then(|| "a column".to_string())
        });
        assert!(matches!(
            report.status(&channel_item(1, 1, "B")),
            Availability::Unsupported { .. }
        ));
    }

    #[test]
    fn a_channel_that_was_not_read_in_any_layer_is_counted_though_it_is_not_described() {
        let mut build = small();
        let corrupt = Error::new(ErrorKind::Corrupt {
            detail: "damage".into(),
        });
        let laser = build.laser_mut(&laser_key(1)).unwrap();
        laser
            .layers
            .get_mut(LayerId(1))
            .unwrap()
            .origins
            .record("channel Z", Origin::not_read(&corrupt));
        let report = report_of(build, &|_| None);
        assert!(matches!(
            report.status(&channel_item(1, 1, "Z")),
            Availability::Unreadable { .. }
        ));
        // the channels that are described are 3 present and 1 not recorded, and Z makes five
        assert_eq!(report.counts_for(ItemKind::Channel).total(), 5);
        assert_eq!(report.counts_for(ItemKind::Channel).unreadable, 1);
    }

    #[test]
    fn every_item_has_one_of_the_five_states() {
        // a build with every state in it
        let mut build = small();
        let corrupt = Error::new(ErrorKind::Corrupt {
            detail: "damage".into(),
        });
        let laser = build.laser_mut(&laser_key(1)).unwrap();
        laser.channels[1]
            .presence
            .insert(LayerId(1), Presence::from_count(40)); // degraded
        laser
            .layers
            .get_mut(LayerId(2))
            .unwrap()
            .origins
            .record("channel A", Origin::not_read(&corrupt)); // unreadable
        let report = report_of(build, &|c| {
            (c.key == ChannelKey::new(["B"])).then(|| "x".to_string())
        });
        let states: Vec<&str> = [
            channel_item(1, 1, "A"),
            channel_item(2, 1, "B"),
            channel_item(2, 1, "A"),
            channel_item(1, 1, "B"),
        ]
        .iter()
        .map(|item| match report.status(item) {
            Availability::Present => "present",
            Availability::NotRecorded => "not recorded",
            Availability::Degraded { .. } => "degraded",
            Availability::Unreadable { .. } => "unreadable",
            Availability::Unsupported { .. } => "unsupported",
        })
        .collect();
        assert_eq!(
            states,
            ["present", "not recorded", "unreadable", "unsupported"]
        );
        // and the counts add up to the items: five states, and not one item uncounted or counted twice
        let all = report.counts();
        assert_eq!(
            all.total(),
            2 + 4 + 4,
            "two layers, four laser-layers and four channel-layers: {all:?}"
        );
        assert_eq!(
            all.degraded + all.unreadable + all.unsupported,
            report.exceptions().len() as u64
        );
    }

    /// A source whose columns cannot be read: a read panics, so a report that is made without a read is the only one that can be made.
    struct NoSamples(Arc<BuildMeta>);

    impl ColumnSource for NoSamples {
        fn read(&self, _: &ColumnRequest, _: &ReadContext) -> Result<ColumnData, Error> {
            panic!("availability must not read a column");
        }
    }

    impl BuildSource for NoSamples {
        fn build(&self) -> Result<Arc<BuildMeta>, Error> {
            Ok(Arc::clone(&self.0))
        }
        fn revision(&self) -> Result<Revision, Error> {
            Ok(Revision::default())
        }
    }

    #[test]
    fn a_source_that_is_not_a_file_has_an_availability_that_reads_no_sample_and_calls_everything_readable(
    ) {
        let source = NoSamples(Arc::new(small()));
        let report = source.availability().expect("a report");
        assert_eq!(report.counts().unsupported, 0);
        assert_eq!(report.counts().total(), 10);
        assert!(matches!(
            report.status(&channel_item(1, 1, "B")),
            Availability::Present
        ));
    }

    #[test]
    fn the_report_is_send_and_sync() {
        fn is_send_and_sync<T: Send + Sync>() {}
        is_send_and_sync::<AvailabilityReport>();
        is_send_and_sync::<Arc<AvailabilityReport>>();
        is_send_and_sync::<ItemAvailability>();
        is_send_and_sync::<Item>();
    }
}
