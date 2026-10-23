//! The aggregates: build identity, and the metadata of the build, its layers, its lasers and each laser in each layer.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** producer (`BuildIdentity`); library (`BuildMeta`, `LayerMeta`, `LaserMeta`, `LaserLayerMeta`).
//! * **Requirement:** BL-CD-01, BL-CD-02, BL-CD-03.
//!
//! # What this module claims, and what it does not
//!
//! * **It is a skeleton of metadata and holds no samples.** A build has layers and lasers; each laser in each layer has a presence and a time axis; each laser
//!   has channel descriptors with a per-layer presence. The samples are reached through column accessors (BL-CD-03).
//! * **The layout is neutral** (BL-CD-02): whether a version nests layers inside lasers or lasers inside layers is the adapter's concern, and a layer that
//!   a version stores only as part of its lasers is built from them ([`BuildMeta::synthesise_missing_layers`]).
//! * **Declared is not present.** `Layer_Quantity` is what a file *declares*; the layers it *holds* are counted separately, and neither is derived from
//!   the other. (A writer may declare the number planned or the last committed, CT-UC-09; no file says which, so nothing here does.)
//! * **The build date is the string the file gave** (phase 2E, which would parse it, was dropped): [`BuildIdentity::date`] is written back unchanged.
//! * **Absent is absent** (decision D02-4). Optional fields are `Option`, and a value that was read as "not recorded" says so in the node's
//!   [`Origins`] (a thickness of `0.0` that a writer uses when it does not know, decision J4). A layer has two thicknesses, the actual and the commanded; each
//!   is read and reported on its own.
//! * **Nothing a file holds is dropped.** Every node but the channel carries an [`Extra`]: what the adapter did not type is kept there (IP-13 phase 13A).
//! * **The completion tranche adds** parameters, provenance, quality and events to these nodes (task T02.26); they are optional and omitted when absent,
//!   so they do not change what is written here.

use crate::channel::ChannelDescriptor;
use crate::extra::Extra;
use crate::identity::{ByLayer, LaserKey, LayerId, Presence};
use crate::provenance::{Origin, Origins};
use crate::time::{TimeAxis, TimeValue};
use crate::units::Quantity;

/// Who and what a build is, as its producer states it. A producer builds this with a full struct literal, so that a new field makes it decide.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIdentity {
    pub build_id: String,
    /// The build date exactly as the file gave it (decision: the CDM does not parse it). `None` if the file has none.
    pub date: Option<String>,
    pub username: Option<String>,
    pub alloy: Option<String>,
    pub part_count: Option<i64>,
    pub project_id: Option<String>,
    /// The name of the machine configuration the build was made from, as an opaque string: stored and returned as given, never resolved (rule M2).
    pub source_mcf_name: Option<String>,
    /// The hash of that configuration, as an opaque string.
    pub source_mcf_hash: Option<String>,
}

/// A layer of the build: its id (the file's own), its times as written and its two thicknesses.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LayerMeta {
    pub id: LayerId,
    /// The start and end of the layer as the file gave them, with a unit only if the file records one (decision D02-19).
    pub start: Option<TimeValue>,
    pub end: Option<TimeValue>,
    /// The thickness that the file says was laid, where it gives a real one. A placeholder is not a thickness: see
    /// [`LayerMeta::set_actual_thickness_from_file`]. It is a different quantity from [`LayerMeta::commanded_thickness`], and neither is derived from the other.
    pub actual_thickness: Option<Quantity>,
    /// The thickness that the file says was commanded, where it gives a real one. A placeholder is not a thickness: see
    /// [`LayerMeta::set_commanded_thickness_from_file`].
    pub commanded_thickness: Option<Quantity>,
    pub origins: Origins,
    /// What the adapter did not type of this layer.
    pub extra: Extra,
}

impl LayerMeta {
    pub fn new(id: LayerId) -> Self {
        LayerMeta {
            id,
            start: None,
            end: None,
            actual_thickness: None,
            commanded_thickness: None,
            origins: Origins::new(),
            extra: Extra::new(),
        }
    }

    /// Sets the actual thickness from the value a file gave. A value that is one of `placeholders` (what that version's writer writes when it does not
    /// know: `1.0` in V16, `0.0` in V17) is **not a thickness**: the field stays absent and the origin `actual_thickness` is `PlaceholderDropped` and
    /// records the value that was there. Which values are placeholders is the adapter's knowledge of its version, so the adapter passes them.
    pub fn set_actual_thickness_from_file(
        &mut self,
        value: f64,
        unit: Option<&str>,
        placeholders: &[f64],
    ) {
        self.actual_thickness = thickness_from_file(
            &mut self.origins,
            "actual_thickness",
            value,
            unit,
            placeholders,
        );
    }

    /// The same for the commanded thickness, whose origin is `commanded_thickness`. The two are separate facts of the file and are read, kept and reported
    /// separately: one can be a placeholder while the other is not.
    pub fn set_commanded_thickness_from_file(
        &mut self,
        value: f64,
        unit: Option<&str>,
        placeholders: &[f64],
    ) {
        self.commanded_thickness = thickness_from_file(
            &mut self.origins,
            "commanded_thickness",
            value,
            unit,
            placeholders,
        );
    }
}

/// A thickness that a file gave, or nothing and an origin if the value is a placeholder.
fn thickness_from_file(
    origins: &mut Origins,
    field: &str,
    value: f64,
    unit: Option<&str>,
    placeholders: &[f64],
) -> Option<Quantity> {
    if placeholders.contains(&value) {
        origins.record(
            field,
            Origin::PlaceholderDropped {
                raw: format!("{value:?}"),
            },
        );
        None
    } else {
        Some(Quantity::from_file(value, unit))
    }
}

/// One laser in one layer: whether it recorded anything, and the time axis of what it recorded. The window of the stream is the axis's.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LaserLayerMeta {
    pub presence: Presence,
    pub time_axis: TimeAxis,
    pub origins: Origins,
    /// What the adapter did not type of this laser in this layer.
    pub extra: Extra,
}

impl LaserLayerMeta {
    pub fn new(presence: Presence) -> Self {
        LaserLayerMeta {
            presence,
            time_axis: TimeAxis::new(),
            origins: Origins::new(),
            extra: Extra::new(),
        }
    }

    pub fn with_time_axis(mut self, time_axis: TimeAxis) -> Self {
        self.time_axis = time_axis;
        self
    }
}

/// A laser of the build: its key, its channels (described once, with their per-layer presence) and what it did in each layer.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LaserMeta {
    pub key: LaserKey,
    pub channels: Vec<ChannelDescriptor>,
    pub layers: ByLayer<LaserLayerMeta>,
    pub origins: Origins,
    /// What the adapter did not type of this laser.
    pub extra: Extra,
}

impl LaserMeta {
    pub fn new(key: LaserKey) -> Self {
        LaserMeta {
            key,
            channels: Vec::new(),
            layers: ByLayer::new(),
            origins: Origins::new(),
            extra: Extra::new(),
        }
    }
}

/// The metadata of a build: identity, layers, lasers and sensors. No samples.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BuildMeta {
    pub identity: BuildIdentity,
    /// The number of layers the file *declares* (`Layer_Quantity`). It is not a count of the layers present and is never computed from them.
    pub layers_declared: Option<i64>,
    /// The layer thickness of the build, where the file gives one.
    pub layer_thickness: Option<Quantity>,
    /// The layers the file holds, by the file's own ids. Gaps are allowed.
    pub layers: ByLayer<LayerMeta>,
    /// Build-level channels: the sensors. Provisional (decision D02-16).
    pub sensors: Vec<ChannelDescriptor>,
    pub origins: Origins,
    /// What the adapter did not type of the build.
    pub extra: Extra,
    lasers: Vec<LaserMeta>,
}

impl BuildMeta {
    pub fn new(identity: BuildIdentity) -> Self {
        BuildMeta {
            identity,
            layers_declared: None,
            layer_thickness: None,
            layers: ByLayer::new(),
            sensors: Vec::new(),
            origins: Origins::new(),
            extra: Extra::new(),
            lasers: Vec::new(),
        }
    }

    /// The lasers in numeric order of their keys, so that laser 10 comes after laser 2.
    pub fn lasers(&self) -> &[LaserMeta] {
        &self.lasers
    }

    pub fn laser(&self, key: &LaserKey) -> Option<&LaserMeta> {
        self.lasers.iter().find(|laser| &laser.key == key)
    }

    /// Adds a laser, keeping the lasers in numeric order. A laser with the same key replaces the old one, which is returned.
    pub fn add_laser(&mut self, laser: LaserMeta) -> Option<LaserMeta> {
        match self.lasers.binary_search_by(|l| l.key.cmp(&laser.key)) {
            Ok(at) => Some(std::mem::replace(&mut self.lasers[at], laser)),
            Err(at) => {
                self.lasers.insert(at, laser);
                None
            }
        }
    }

    /// How many layers the file holds. Compare [`BuildMeta::layers_declared`], which is what it says it holds.
    pub fn layers_present(&self) -> usize {
        self.layers.len()
    }

    /// Adds a layer for each layer id that a laser has and the build does not. A version that stores a layer only as part of its lasers (V18) has no
    /// layer group, so the layer is built from its lasers: it has the id and nothing else, and its origin says it was inferred. Returns how many were added.
    pub fn synthesise_missing_layers(&mut self) -> usize {
        let missing: Vec<LayerId> = self
            .lasers
            .iter()
            .flat_map(|laser| laser.layers.ids())
            .filter(|id| !self.layers.contains(*id))
            .collect();
        let mut added = 0;
        for id in missing {
            if !self.layers.contains(id) {
                let mut layer = LayerMeta::new(id);
                layer.origins.record("layer", Origin::Inferred);
                self.layers.insert(id, layer);
                added += 1;
            }
        }
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelDType, ChannelKind, ClockClass, NumericType};
    use crate::identity::ChannelKey;

    fn identity() -> BuildIdentity {
        BuildIdentity {
            build_id: "build-1".into(),
            date: Some("2026-08-18 16:48:25 UTC+00:00".into()),
            username: None,
            alloy: None,
            part_count: None,
            project_id: None,
            source_mcf_name: None,
            source_mcf_hash: None,
        }
    }

    fn laser(number: u32, layers: &[u32]) -> LaserMeta {
        let mut laser = LaserMeta::new(LaserKey::new(number.to_string(), number));
        for id in layers {
            laser.layers.insert(
                LayerId(*id),
                LaserLayerMeta::new(Presence::from_count(1_000)),
            );
        }
        laser
    }

    #[test]
    fn layer_quantity_is_declared_not_counted() {
        // the raw V18 declared 136 layers and held 2
        let mut build = BuildMeta::new(identity());
        build.layers_declared = Some(136);
        build.layers.insert(LayerId(1), LayerMeta::new(LayerId(1)));
        build.layers.insert(LayerId(2), LayerMeta::new(LayerId(2)));
        assert_eq!(build.layers_declared, Some(136));
        assert_eq!(build.layers_present(), 2);
        // adding a layer does not change what the file declared, and declaring does not add a layer
        build.layers.insert(LayerId(3), LayerMeta::new(LayerId(3)));
        assert_eq!(build.layers_declared, Some(136));
        // a file that declares nothing says nothing: the count is not substituted
        let undeclared = BuildMeta::new(identity());
        assert_eq!(undeclared.layers_declared, None);
        assert_eq!(undeclared.layers_present(), 0);
    }

    #[test]
    fn a_layer_without_a_layer_group_is_synthesised_from_its_lasers() {
        // V18 keeps each laser's layers under the laser: there is no layer group, only lasers with layers 1, 2 and 5
        let mut build = BuildMeta::new(identity());
        build.add_laser(laser(2, &[1, 2]));
        build.add_laser(laser(1, &[1, 2, 5]));
        assert_eq!(build.layers_present(), 0);
        assert_eq!(build.synthesise_missing_layers(), 3);
        let ids: Vec<u32> = build.layers.ids().map(|id| id.0).collect();
        assert_eq!(ids, [1, 2, 5]);
        // each is the id and nothing else, and says it was inferred
        let layer = build.layers.get(LayerId(5)).unwrap();
        assert_eq!(
            (
                layer.start.is_none(),
                layer.end.is_none(),
                layer.actual_thickness.is_none() && layer.commanded_thickness.is_none()
            ),
            (true, true, true)
        );
        assert_eq!(layer.origins.get("layer"), Some(&Origin::Inferred));
        // doing it again adds nothing, and a layer that exists is left as it is
        let mut existing = LayerMeta::new(LayerId(1));
        existing.start = Some(TimeValue::from_file(1_000, None));
        build.layers.insert(LayerId(1), existing.clone());
        assert_eq!(build.synthesise_missing_layers(), 0);
        assert_eq!(build.layers.get(LayerId(1)), Some(&existing));
    }

    #[test]
    fn thickness_placeholder_becomes_not_recorded_with_origin() {
        // V17 writes 0.0 when it does not know the thickness; V16 writes 1.0
        let mut layer = LayerMeta::new(LayerId(1));
        layer.set_actual_thickness_from_file(0.0, None, &[0.0]);
        layer.set_commanded_thickness_from_file(0.0, None, &[0.0]);
        assert_eq!(
            (
                layer.actual_thickness.clone(),
                layer.commanded_thickness.clone()
            ),
            (None, None)
        );
        // each of the two says so on its own, with the value that was there
        for field in ["actual_thickness", "commanded_thickness"] {
            assert_eq!(
                layer.origins.get(field),
                Some(&Origin::PlaceholderDropped { raw: "0.0".into() }),
                "{field}"
            );
        }
        // a real thickness is kept as written, with an unknown unit when the file has none (J5), and says nothing
        let mut real = LayerMeta::new(LayerId(2));
        real.set_actual_thickness_from_file(0.03, None, &[0.0]);
        let thickness = real.actual_thickness.as_ref().unwrap();
        assert_eq!(thickness.value, 0.03);
        assert!(!thickness.unit.is_known());
        assert!(real.origins.is_empty());
        // the same value is a thickness where the adapter's version does not use it as a placeholder
        let mut v16 = LayerMeta::new(LayerId(3));
        v16.set_actual_thickness_from_file(1.0, None, &[1.0]);
        assert_eq!(v16.actual_thickness, None);
        let mut other = LayerMeta::new(LayerId(4));
        other.set_actual_thickness_from_file(1.0, None, &[0.0]);
        assert_eq!(other.actual_thickness.as_ref().map(|t| t.value), Some(1.0));
    }

    #[test]
    fn the_actual_and_the_commanded_thickness_are_separate_facts() {
        // one can be real while the other is a placeholder: neither stands for the other, and each origin is its own
        let mut layer = LayerMeta::new(LayerId(1));
        layer.set_actual_thickness_from_file(0.031, None, &[0.0]);
        layer.set_commanded_thickness_from_file(0.0, None, &[0.0]);
        assert_eq!(
            layer.actual_thickness.as_ref().map(|t| t.value),
            Some(0.031)
        );
        assert_eq!(layer.commanded_thickness, None);
        assert_eq!(layer.origins.get("actual_thickness"), None);
        assert_eq!(
            layer.origins.get("commanded_thickness"),
            Some(&Origin::PlaceholderDropped { raw: "0.0".into() })
        );
        // and the other way round
        let mut other = LayerMeta::new(LayerId(2));
        other.set_actual_thickness_from_file(0.0, None, &[0.0]);
        other.set_commanded_thickness_from_file(0.03, None, &[0.0]);
        assert_eq!(other.actual_thickness, None);
        assert_eq!(
            other.commanded_thickness.as_ref().map(|t| t.value),
            Some(0.03)
        );
        // an attribute that the file does not have is not set at all: absent is absent, and says nothing
        let absent = LayerMeta::new(LayerId(3));
        assert!(
            absent.actual_thickness.is_none()
                && absent.commanded_thickness.is_none()
                && absent.origins.is_empty()
        );
    }

    #[test]
    fn lasers_are_kept_in_numeric_order_and_a_laser_with_the_same_key_replaces() {
        let mut build = BuildMeta::new(identity());
        for number in [10, 2, 1] {
            assert!(build.add_laser(laser(number, &[1])).is_none());
        }
        let numbers: Vec<u32> = build.lasers().iter().map(|l| l.key.number).collect();
        assert_eq!(numbers, [1, 2, 10]);
        // the same key: replaced, and the old one is returned
        let mut replacement = laser(2, &[1, 2, 3]);
        replacement.channels.push(ChannelDescriptor::new(
            ChannelKey::new(["power"]),
            ChannelKind::LaserPower,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(NumericType::F32),
        ));
        let old = build.add_laser(replacement).expect("laser 2 was there");
        assert_eq!(old.layers.len(), 1);
        assert_eq!(build.lasers().len(), 3);
        assert_eq!(
            build.laser(&LaserKey::new("2", 2)).unwrap().channels.len(),
            1
        );
        assert!(build.laser(&LaserKey::new("7", 7)).is_none());
    }

    #[test]
    fn every_node_carries_an_extra_that_starts_empty() {
        assert!(BuildMeta::new(identity()).extra.is_empty());
        assert!(LayerMeta::new(LayerId(1)).extra.is_empty());
        assert!(LaserMeta::new(LaserKey::new("1", 1)).extra.is_empty());
        assert!(LaserLayerMeta::new(Presence::from_count(1))
            .extra
            .is_empty());
    }

    #[test]
    fn the_build_date_is_the_string_the_file_gave() {
        // no parsing, no normalising, and no offset check: the CDM keeps what it was given
        let odd = BuildIdentity {
            date: Some("2026-03-20 20:37:57 UTC-05:00".into()),
            ..identity()
        };
        assert_eq!(odd.date.as_deref(), Some("2026-03-20 20:37:57 UTC-05:00"));
        let build = BuildMeta::new(odd.clone());
        assert_eq!(build.identity, odd);
    }
}
