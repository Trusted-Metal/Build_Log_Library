//! Validation: what is wrong or odd about a build, found from its metadata without reading a sample (IP-07 phase 7C, decisions D07-5 and D07-6; requirements BL-FA-09 and Vision's QE-05).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** library (`Finding`, `ValidationCounts`, `ValidationReport`); the enums `Severity` and `FindingCode`.
//! * **Requirement:** BL-FA-09 (failures are isolated per item and reported on the result, aggregated as counts), BL-FA-01 (a source that is not a file has it too).
//!
//! # What this module claims, and what it does not
//!
//! * **It is a function of the metadata, so it is the same for every version and every source** ([`validate_build`]). It reads no sample: a source that fails if a column is asked is the test.
//! * **Three severities.** A [`Severity::Note`] is something legitimate that is worth knowing; a [`Severity::Warning`] says the data is suspect, and makes the item at fault `Degraded` in the
//!   availability report (phase 7D); a [`Severity::Fault`] says something was not read.
//! * **The rules (D07-5):**
//!   **V1** the build declares a number of layers that is not the number it holds ([`FindingCode::LayerCountDiffers`], a note: the declared count is not a count of the layers present, and is zero
//!   in a file that was not closed);
//!   **V2** a layer ends before it starts ([`FindingCode::LayerEndsBeforeItStarts`]), or starts before the last layer that has times ended ([`FindingCode::LayersOverlap`]), in the integers the
//!   file wrote, both warnings (**a gap between two layers is not a finding**: a real build has one of forty hours);
//!   **V3** a channel of a laser in a layer holds fewer samples than the laser's stream there ([`FindingCode::ChannelShorterThanStream`], a warning for each short channel; **a channel that is not
//!   recorded at all is not a finding**, because a writer that omits an empty channel is a normal writer);
//!   **V4** something the adapter did not read: [`FindingCode::NotRead`], a fault, where the cause is the file's, and [`FindingCode::NotSupported`], a note, where it is the library's (a fact
//!   about the library and not about the file);
//!   **V5** a layer in which no laser is recorded, in a build that has lasers ([`FindingCode::LayerWithoutLasers`], a note).
//! * **What it leaves out (D07-6):** checks that read values, a laser's window against its layer's (V17 states no unit for either, decision D02-19, so a comparison would guess), a group that a
//!   version expects and the file lacks, and a values column shorter than its timestamps in an event series (a read reports it).
//! * **The findings are in a fixed order**: by code (the order the enum declares them), then layer, laser (by number) and channel, so that two sources of one build give the same report.
//!
//! The policy guards in `tests/policy_guards.rs` scan this file.

use crate::error::Location;
use crate::identity::{ChannelKey, Presence};
use crate::meta::BuildMeta;
use crate::provenance::{channel_of_key, NotReadCause, Origin, Origins};

/// How serious a finding is: whether it says something is worth knowing, that the data is suspect, or that something was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Severity {
    /// Legitimate, and worth knowing.
    Note,
    /// The data is suspect: the item it is about is degraded.
    Warning,
    /// Something was not read.
    Fault,
}

/// What a finding is about. Each rule has a code of its own, and each code has the severity that it always has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum FindingCode {
    /// V1: the number of layers the build declares is not the number it holds.
    LayerCountDiffers,
    /// V2: a layer ends before it starts.
    LayerEndsBeforeItStarts,
    /// V2: a layer starts before the last layer that had times ended.
    LayersOverlap,
    /// V3: a channel of a laser in a layer holds fewer samples than the laser's stream there.
    ChannelShorterThanStream,
    /// V4: something was not read, because of a fault of the file.
    NotRead,
    /// V4: something was not read, because the version or the library does not read it.
    NotSupported,
    /// V5: no laser is recorded in a layer of a build that has lasers.
    LayerWithoutLasers,
}

impl FindingCode {
    /// The code's name, which is stable: it is what an expectation file and a report for a person say.
    pub fn name(self) -> &'static str {
        match self {
            FindingCode::LayerCountDiffers => "LayerCountDiffers",
            FindingCode::LayerEndsBeforeItStarts => "LayerEndsBeforeItStarts",
            FindingCode::LayersOverlap => "LayersOverlap",
            FindingCode::ChannelShorterThanStream => "ChannelShorterThanStream",
            FindingCode::NotRead => "NotRead",
            FindingCode::NotSupported => "NotSupported",
            FindingCode::LayerWithoutLasers => "LayerWithoutLasers",
        }
    }

    /// The severity that a finding of this code has.
    pub fn severity(self) -> Severity {
        match self {
            FindingCode::LayerCountDiffers
            | FindingCode::NotSupported
            | FindingCode::LayerWithoutLasers => Severity::Note,
            FindingCode::LayerEndsBeforeItStarts
            | FindingCode::LayersOverlap
            | FindingCode::ChannelShorterThanStream => Severity::Warning,
            FindingCode::NotRead => Severity::Fault,
        }
    }
}

/// One thing that is wrong or odd, with where it was found and what it says.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finding {
    pub code: FindingCode,
    /// The severity of the code.
    pub severity: Severity,
    /// The layer, the laser and the channel it is about, as far as it is about one.
    pub location: Location,
    /// For a person.
    pub message: String,
}

impl Finding {
    pub fn new(code: FindingCode, location: Location, message: impl Into<String>) -> Self {
        Finding {
            code,
            severity: code.severity(),
            location,
            message: message.into(),
        }
    }
}

/// How many findings there are of each severity.
// constructed-by: library
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ValidationCounts {
    pub notes: u64,
    pub warnings: u64,
    pub faults: u64,
}

/// The findings about a build, in a fixed order, and how many there are of each severity. **The counts are of the findings, and nothing else is drained from anywhere**: a report is the whole
/// of what a source says is wrong with its build (BL-FA-09).
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ValidationReport {
    pub findings: Vec<Finding>,
    pub counts: ValidationCounts,
}

/// The place of a finding as a sort key: the layer, the laser's number and key, the channel.
fn place(finding: &Finding) -> (Option<u32>, Option<(u32, &str)>, Option<&ChannelKey>) {
    (
        finding.location.layer.map(|layer| layer.0),
        finding
            .location
            .laser
            .as_ref()
            .map(|laser| (laser.number, laser.key.as_str())),
        finding.location.channel.as_ref(),
    )
}

impl ValidationReport {
    /// A report of these findings, put in order and counted.
    pub fn new(mut findings: Vec<Finding>) -> Self {
        findings.sort_by(|a, b| {
            a.code
                .cmp(&b.code)
                .then_with(|| place(a).cmp(&place(b)))
                .then_with(|| a.message.cmp(&b.message))
        });
        let mut counts = ValidationCounts::default();
        for finding in &findings {
            match finding.severity {
                Severity::Note => counts.notes += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Fault => counts.faults += 1,
            }
        }
        ValidationReport { findings, counts }
    }

    /// Is there nothing to say?
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// How many findings have this code.
    pub fn count_of(&self, code: FindingCode) -> usize {
        self.findings.iter().filter(|f| f.code == code).count()
    }
}

/// The findings about a build, from its metadata: the rules of the module documentation. It reads no sample, and it is the same function for every version and every source.
pub fn validate_build(build: &BuildMeta) -> ValidationReport {
    let mut found = Vec::new();
    layer_count(build, &mut found);
    layer_times(build, &mut found);
    channel_lengths(build, &mut found);
    not_read(build, &mut found);
    layers_without_lasers(build, &mut found);
    ValidationReport::new(found)
}

/// V1.
fn layer_count(build: &BuildMeta, found: &mut Vec<Finding>) {
    let present = build.layers_present() as i64;
    if let Some(declared) = build
        .layers_declared
        .filter(|&declared| declared != present)
    {
        found.push(Finding::new(
            FindingCode::LayerCountDiffers,
            Location::new(),
            format!("{declared} layers are declared and {present} are held"),
        ));
    }
}

/// V2: the layers in the order of their ids; the one that a layer is compared with is the last that had both of its times.
fn layer_times(build: &BuildMeta, found: &mut Vec<Finding>) {
    let mut previous: Option<(i64, i64)> = None;
    for (id, layer) in build.layers.iter() {
        let (Some(start), Some(end)) = (&layer.start, &layer.end) else {
            continue;
        };
        let (start, end) = (start.value, end.value);
        let at = Location::new().with_layer(id);
        if end < start {
            found.push(Finding::new(
                FindingCode::LayerEndsBeforeItStarts,
                at.clone(),
                format!("the layer ends at {end}, before it starts at {start}"),
            ));
        }
        if let Some((_, before)) = previous.filter(|&(_, before)| before > start) {
            found.push(Finding::new(
                FindingCode::LayersOverlap,
                at,
                format!("the layer starts at {start}, before the last layer with times ended at {before}"),
            ));
        }
        previous = Some((start, end));
    }
}

/// V3: where a laser is recorded in a layer, each of its channels that is recorded there and holds fewer samples than the stream.
fn channel_lengths(build: &BuildMeta, found: &mut Vec<Finding>) {
    for laser in build.lasers() {
        for (layer, entry) in laser.layers.iter() {
            let Some(stream) = entry.presence.samples() else {
                continue;
            };
            for channel in &laser.channels {
                let Some(held) = channel.presence.get(layer).and_then(Presence::samples) else {
                    continue;
                };
                if held < stream {
                    found.push(Finding::new(
                        FindingCode::ChannelShorterThanStream,
                        Location::new()
                            .with_layer(layer)
                            .with_laser(laser.key.clone())
                            .with_channel(channel.key.clone()),
                        format!("the channel holds {held} samples and the laser's stream holds {stream}"),
                    ));
                }
            }
        }
    }
}

/// V4: every origin that says something was not read, on the build, a layer, a laser and a laser in a layer. The origin of a channel is named `channel` and the name of the channel.
fn not_read(build: &BuildMeta, found: &mut Vec<Finding>) {
    let mut add = |origins: &Origins, at: Location| {
        for (field, origin) in origins.iter() {
            let Origin::NotRead { cause, reason } = origin else {
                continue;
            };
            let (code, said) = match cause {
                NotReadCause::Unsupported => (
                    FindingCode::NotSupported,
                    "not read, because it is not offered",
                ),
                _ => (FindingCode::NotRead, "not read"),
            };
            let location = match channel_of_key(field) {
                Some(name) => at.clone().with_channel(ChannelKey::new([name])),
                None => at.clone(),
            };
            found.push(Finding::new(
                code,
                location,
                format!("{field}: {said} ({reason})"),
            ));
        }
    };
    add(&build.origins, Location::new());
    for (id, layer) in build.layers.iter() {
        add(&layer.origins, Location::new().with_layer(id));
    }
    for laser in build.lasers() {
        add(
            &laser.origins,
            Location::new().with_laser(laser.key.clone()),
        );
        for (id, entry) in laser.layers.iter() {
            add(
                &entry.origins,
                Location::new().with_layer(id).with_laser(laser.key.clone()),
            );
        }
    }
}

/// V5: a layer that no laser is recorded in, in a build that has lasers (a build that has none has no layer to be without one).
fn layers_without_lasers(build: &BuildMeta, found: &mut Vec<Finding>) {
    if build.lasers().is_empty() {
        return;
    }
    for (id, _) in build.layers.iter() {
        let recorded = build.lasers().iter().any(|laser| {
            laser
                .layers
                .get(id)
                .is_some_and(|entry| entry.presence != Presence::NotRecorded)
        });
        if !recorded {
            found.push(Finding::new(
                FindingCode::LayerWithoutLasers,
                Location::new().with_layer(id),
                "no laser is recorded in the layer",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::channel::{ChannelDType, ChannelDescriptor, ChannelKind, ClockClass, NumericType};
    use crate::column::{
        BuildSource, ColumnData, ColumnRequest, ColumnSource, ReadContext, Revision,
    };
    use crate::error::{Error, ErrorKind};
    use crate::identity::{ByLayer, LaserKey, LayerId};
    use crate::meta::{BuildIdentity, LaserLayerMeta, LaserMeta, LayerMeta};
    use crate::time::TimeValue;

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

    /// A layer with times (the integers a file wrote).
    fn layer(id: u32, start: i64, end: i64) -> LayerMeta {
        let mut layer = LayerMeta::new(LayerId(id));
        layer.start = Some(TimeValue::from_file(start, None));
        layer.end = Some(TimeValue::from_file(end, None));
        layer
    }

    fn with_layers(times: &[(u32, i64, i64)]) -> BuildMeta {
        let mut build = build();
        for &(id, start, end) in times {
            build.layers.insert(LayerId(id), layer(id, start, end));
        }
        build
    }

    fn channel(name: &str, samples: &[(u32, u64)]) -> ChannelDescriptor {
        let mut presence = ByLayer::new();
        for &(layer, count) in samples {
            presence.insert(LayerId(layer), Presence::from_count(count));
        }
        ChannelDescriptor::new(
            ChannelKey::new([name]),
            ChannelKind::LaserPower,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(NumericType::F32),
        )
        .with_presence(presence)
    }

    /// A laser recorded in `layers` with the stream's samples, and the channels given.
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

    fn codes(report: &ValidationReport) -> Vec<(FindingCode, Option<u32>)> {
        report
            .findings
            .iter()
            .map(|f| (f.code, f.location.layer.map(|l| l.0)))
            .collect()
    }

    #[test]
    fn a_clean_build_has_no_findings() {
        let mut build = with_layers(&[(1, 0, 10), (2, 20, 30)]);
        build.layers_declared = Some(2);
        build.add_laser(laser(
            1,
            &[(1, 100), (2, 100)],
            vec![channel("A", &[(1, 100), (2, 100)])],
        ));
        let report = validate_build(&build);
        assert!(
            report.is_clean() && report.findings.is_empty(),
            "{report:?}"
        );
        assert_eq!(report.counts, ValidationCounts::default());
        // an empty build is clean too
        assert!(validate_build(&self::build()).is_clean());
    }

    #[test]
    fn a_declared_layer_count_that_differs_from_the_layers_present_is_a_note() {
        let mut build = with_layers(&[(1, 0, 10), (2, 20, 30)]);
        build.layers_declared = Some(136);
        let report = validate_build(&build);
        assert_eq!(codes(&report), [(FindingCode::LayerCountDiffers, None)]);
        assert_eq!(report.findings[0].severity, Severity::Note);
        assert!(
            report.findings[0].message.contains("136") && report.findings[0].message.contains('2')
        );
        // a file that was not closed declares none; a file that declares nothing has nothing to differ from; a count that is true is not a finding
        build.layers_declared = Some(0);
        assert_eq!(
            validate_build(&build).count_of(FindingCode::LayerCountDiffers),
            1
        );
        build.layers_declared = Some(2);
        assert!(validate_build(&build).is_clean());
        build.layers_declared = None;
        assert!(validate_build(&build).is_clean());
    }

    #[test]
    fn a_layer_that_ends_before_it_starts_is_a_warning() {
        let report = validate_build(&with_layers(&[(1, 10, 0)]));
        assert_eq!(
            codes(&report),
            [(FindingCode::LayerEndsBeforeItStarts, Some(1))]
        );
        assert_eq!(report.findings[0].severity, Severity::Warning);
        assert_eq!(report.counts.warnings, 1);
        // a layer of no length is not one that ends before it starts
        assert!(validate_build(&with_layers(&[(1, 5, 5)])).is_clean());
    }

    #[test]
    fn overlapping_layers_are_a_warning_and_a_gap_is_not() {
        // a gap of forty hours between two layers is a pause in a build
        assert!(
            validate_build(&with_layers(&[(1, 0, 10), (2, 143_795_000, 143_796_000)])).is_clean()
        );
        // a layer that starts when the last ended does not overlap it, and one that starts before does
        assert!(validate_build(&with_layers(&[(1, 0, 10), (2, 10, 20)])).is_clean());
        assert_eq!(
            codes(&validate_build(&with_layers(&[(1, 0, 10), (2, 5, 20)]))),
            [(FindingCode::LayersOverlap, Some(2))]
        );
        // the layers are taken in the order of their ids; one with no times is skipped, and the next is compared with the last that had them
        let mut build = with_layers(&[(1, 0, 10), (3, 5, 9)]);
        build.layers.insert(LayerId(2), LayerMeta::new(LayerId(2)));
        assert_eq!(
            codes(&validate_build(&build)),
            [(FindingCode::LayersOverlap, Some(3))]
        );
        assert_eq!(
            codes(&validate_build(&with_layers(&[(2, 5, 20), (1, 0, 10)]))),
            [(FindingCode::LayersOverlap, Some(2))]
        );
    }

    #[test]
    fn channels_of_different_lengths_give_a_warning_for_each_short_one() {
        let mut build = with_layers(&[(1, 0, 10), (2, 20, 30)]);
        build.add_laser(laser(
            1,
            &[(1, 100), (2, 100)],
            vec![
                channel("A", &[(1, 100), (2, 100)]),
                channel("B", &[(1, 40)]),
                channel("C", &[(2, 0)]),
            ],
        ));
        let report = validate_build(&build);
        assert_eq!(
            codes(&report),
            [
                (FindingCode::ChannelShorterThanStream, Some(1)),
                (FindingCode::ChannelShorterThanStream, Some(2))
            ]
        );
        // each names its layer, its laser and its channel
        let (first, second) = (&report.findings[0], &report.findings[1]);
        assert_eq!(first.location.channel, Some(ChannelKey::new(["B"])));
        assert_eq!(second.location.channel, Some(ChannelKey::new(["C"])));
        assert_eq!(first.location.laser, Some(LaserKey::new("laser-1", 1)));
        assert!(first.message.contains("40") && first.message.contains("100"));
        assert_eq!(report.counts.warnings, 2);
    }

    #[test]
    fn a_channel_that_is_not_recorded_is_not_a_finding() {
        // B is recorded in layer 1 only, and the laser is recorded in layer 2 too: B is absent there, which is what a writer that omits an empty channel writes
        let mut build = with_layers(&[(1, 0, 10), (2, 20, 30)]);
        build.add_laser(laser(
            1,
            &[(1, 100), (2, 100)],
            vec![channel("B", &[(1, 100)])],
        ));
        assert!(validate_build(&build).is_clean());
        // a laser that is not recorded in a layer has no stream to be short of, and an empty stream has nothing to be short of
        let mut build = with_layers(&[(1, 0, 10)]);
        let mut absent = laser(1, &[], vec![channel("B", &[(1, 40)])]);
        absent
            .layers
            .insert(LayerId(1), LaserLayerMeta::new(Presence::NotRecorded));
        build.add_laser(absent);
        assert!(validate_build(&build).count_of(FindingCode::ChannelShorterThanStream) == 0);
        let mut build = with_layers(&[(1, 0, 10)]);
        build.add_laser(laser(1, &[(1, 0)], vec![channel("B", &[(1, 0)])]));
        assert!(validate_build(&build).is_clean());
    }

    #[test]
    fn a_not_read_item_is_a_fault_and_an_unsupported_one_a_note() {
        let mut build = with_layers(&[(1, 0, 10)]);
        let mut one = laser(1, &[(1, 100)], vec![channel("A", &[(1, 100)])]);
        let corrupt = Error::new(ErrorKind::Corrupt {
            detail: "a channel of 2 dimensions".into(),
        });
        let unsupported = Error::new(ErrorKind::UnsupportedInVersion {
            version: "17".into(),
            feature: "a column stored as a 16-bit unsigned integer".into(),
        });
        {
            let entry = one.layers.get_mut(LayerId(1)).unwrap();
            entry
                .origins
                .record("channel first-channel", Origin::not_read(&corrupt));
            entry
                .origins
                .record("channel second-channel", Origin::not_read(&unsupported));
        }
        one.origins.record("parameters", Origin::not_read(&corrupt));
        build.add_laser(one);
        build
            .layers
            .get_mut(LayerId(1))
            .unwrap()
            .origins
            .record("events", Origin::not_read(&corrupt));
        // an origin that is not a failed read says nothing here
        build.origins.record("anything", Origin::Inferred);
        let report = validate_build(&build);
        assert_eq!(report.count_of(FindingCode::NotRead), 3, "{report:?}");
        assert_eq!(report.count_of(FindingCode::NotSupported), 1);
        assert_eq!(
            report.counts,
            ValidationCounts {
                notes: 1,
                warnings: 0,
                faults: 3
            }
        );
        // each says where: the channel's own, the layer's, and the laser's
        let named = |field: &str| {
            report
                .findings
                .iter()
                .find(|f| f.message.starts_with(field))
                .unwrap()
        };
        assert_eq!(
            named("channel first-channel").location.channel,
            Some(ChannelKey::new(["first-channel"]))
        );
        assert_eq!(
            named("channel first-channel").location.layer,
            Some(LayerId(1))
        );
        assert_eq!(
            named("channel second-channel").code,
            FindingCode::NotSupported
        );
        assert_eq!(named("events").location.layer, Some(LayerId(1)));
        assert!(named("events").location.laser.is_none());
        assert_eq!(
            named("parameters").location.laser,
            Some(LaserKey::new("laser-1", 1))
        );
        assert!(named("parameters").location.layer.is_none());
    }

    #[test]
    fn a_layer_with_no_laser_is_a_note() {
        let mut build = with_layers(&[(1, 0, 10), (2, 20, 30)]);
        let mut one = laser(1, &[(1, 5)], vec![]);
        one.layers
            .insert(LayerId(2), LaserLayerMeta::new(Presence::NotRecorded));
        build.add_laser(one);
        let report = validate_build(&build);
        assert_eq!(codes(&report), [(FindingCode::LayerWithoutLasers, Some(2))]);
        assert_eq!(report.findings[0].severity, Severity::Note);
        // a laser that is in the layer and empty is recorded, and a build with no laser has no layer to be without one
        let mut build = with_layers(&[(1, 0, 10), (2, 20, 30)]);
        build.add_laser(laser(1, &[(1, 5), (2, 0)], vec![]));
        assert!(validate_build(&build).is_clean());
        assert!(validate_build(&with_layers(&[(1, 0, 10)])).is_clean());
    }

    #[test]
    fn the_counts_equal_the_findings_and_the_order_is_by_code_then_place() {
        let mut build = with_layers(&[(1, 0, 10), (2, 5, 20), (3, 30, 25)]);
        build.layers_declared = Some(9);
        build.add_laser(laser(
            10,
            &[(1, 10)],
            vec![channel("Z", &[(1, 3)]), channel("A", &[(1, 3)])],
        ));
        build.add_laser(laser(2, &[(1, 10)], vec![channel("B", &[(1, 3)])]));
        let report = validate_build(&build);
        let order: Vec<(FindingCode, Option<u32>, Option<String>)> = report
            .findings
            .iter()
            .map(|f| {
                (
                    f.code,
                    f.location.laser.as_ref().map(|l| l.number),
                    f.location.channel.as_ref().map(|c| c.segments.join("/")),
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                (FindingCode::LayerCountDiffers, None, None),
                (FindingCode::LayerEndsBeforeItStarts, None, None),
                (FindingCode::LayersOverlap, None, None),
                (
                    FindingCode::ChannelShorterThanStream,
                    Some(2),
                    Some("B".into())
                ),
                (
                    FindingCode::ChannelShorterThanStream,
                    Some(10),
                    Some("A".into())
                ),
                (
                    FindingCode::ChannelShorterThanStream,
                    Some(10),
                    Some("Z".into())
                ),
                (FindingCode::LayerWithoutLasers, None, None),
                (FindingCode::LayerWithoutLasers, None, None),
            ]
        );
        assert_eq!(
            report.counts,
            ValidationCounts {
                notes: 3,
                warnings: 5,
                faults: 0
            }
        );
        assert_eq!(
            (report.counts.notes + report.counts.warnings + report.counts.faults) as usize,
            report.findings.len()
        );
        // two builds that are the same give the same report, however their lasers were added
        let mut other = with_layers(&[(1, 0, 10), (2, 5, 20), (3, 30, 25)]);
        other.layers_declared = Some(9);
        other.add_laser(laser(2, &[(1, 10)], vec![channel("B", &[(1, 3)])]));
        other.add_laser(laser(
            10,
            &[(1, 10)],
            vec![channel("A", &[(1, 3)]), channel("Z", &[(1, 3)])],
        ));
        assert_eq!(validate_build(&other), report);
    }

    #[test]
    fn a_finding_names_its_layer_laser_and_channel() {
        let mut build = with_layers(&[(4, 0, 10)]);
        build.add_laser(laser(3, &[(4, 100)], vec![channel("Short", &[(4, 7)])]));
        let finding = &validate_build(&build).findings[0];
        assert_eq!(finding.location.layer, Some(LayerId(4)));
        assert_eq!(finding.location.laser, Some(LaserKey::new("laser-3", 3)));
        assert_eq!(finding.location.channel, Some(ChannelKey::new(["Short"])));
        assert_eq!(finding.severity, finding.code.severity());
        assert_eq!(finding.code.name(), "ChannelShorterThanStream");
    }

    #[test]
    fn every_code_has_a_name_and_a_severity_and_the_codes_are_in_the_order_of_the_rules() {
        let all = [
            FindingCode::LayerCountDiffers,
            FindingCode::LayerEndsBeforeItStarts,
            FindingCode::LayersOverlap,
            FindingCode::ChannelShorterThanStream,
            FindingCode::NotRead,
            FindingCode::NotSupported,
            FindingCode::LayerWithoutLasers,
        ];
        assert!(all.windows(2).all(|w| w[0] < w[1]));
        let names: Vec<&str> = all.iter().map(|c| c.name()).collect();
        assert_eq!(
            names,
            [
                "LayerCountDiffers",
                "LayerEndsBeforeItStarts",
                "LayersOverlap",
                "ChannelShorterThanStream",
                "NotRead",
                "NotSupported",
                "LayerWithoutLasers"
            ]
        );
        let severities: Vec<Severity> = all.iter().map(|c| c.severity()).collect();
        assert_eq!(
            severities,
            [
                Severity::Note,
                Severity::Warning,
                Severity::Warning,
                Severity::Warning,
                Severity::Fault,
                Severity::Note,
                Severity::Note
            ]
        );
        assert!(Severity::Note < Severity::Warning && Severity::Warning < Severity::Fault);
    }

    /// A source whose columns cannot be read: a read panics, so a report that is made without a read is the only one that can be made.
    struct NoSamples(Arc<BuildMeta>);

    impl ColumnSource for NoSamples {
        fn read(&self, _: &ColumnRequest, _: &ReadContext) -> Result<ColumnData, Error> {
            panic!("validation must not read a column");
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
    fn validation_reads_no_sample() {
        let mut build = with_layers(&[(1, 0, 10)]);
        build.layers_declared = Some(5);
        build.add_laser(laser(1, &[(1, 100)], vec![channel("A", &[(1, 7)])]));
        let source = NoSamples(Arc::new(build));
        let report = source.validate().expect("a report");
        assert_eq!(report.count_of(FindingCode::LayerCountDiffers), 1);
        assert_eq!(report.count_of(FindingCode::ChannelShorterThanStream), 1);
        // it is the same report as the function gives, and a source that is not a file has it without anything of its own
        assert_eq!(*report, validate_build(&source.build().unwrap()));
    }

    #[test]
    fn the_report_is_send_and_sync() {
        fn is_send_and_sync<T: Send + Sync>() {}
        is_send_and_sync::<ValidationReport>();
        is_send_and_sync::<Arc<ValidationReport>>();
        is_send_and_sync::<Finding>();
    }
}
