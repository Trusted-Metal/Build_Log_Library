//! The overview of a build and the detail of one layer: what a source can say without walking every layer (IP-26 phase 26K, decision D26-19, IP-24 finding F17).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** library (`BuildOverview`, `LayerDetail`, `LaserInLayer`, `LaserOverview`).
//! * **Requirement:** BL-CD-01, BL-CD-03 and the plan's D26-19.
//!
//! # What this module claims
//!
//! * **The overview is the build's own level and the ids of its layers.** The identity, the declared layer count, the layer thickness, the outcome, the provenance, the
//!   context, and the lasers that the build's calibration names with their parameters. **It reads no layer.** The lasers *in a layer* are in [`LayerDetail`], and the
//!   exact set of lasers of the build, with their channels across layers, is in [`BuildMeta`] (which walks every layer).
//! * **A layer's detail is that layer's own metadata**: its [`LayerMeta`] (times, thicknesses, untyped extras), and for each laser recorded in it the entry
//!   ([`LaserLayerMeta`]) and the channels it has there, each described with its presence in that layer alone. **It is the slice of the whole build that belongs to
//!   one layer**: [`LayerDetail::from_build`] cuts it from a [`BuildMeta`], and a source that reads one layer at a time gives the same. The one difference is a file that
//!   stores a channel as different types in different layers, which only a walk of every layer can notice.
//! * **A source that holds a whole [`BuildMeta`] derives both from it** ([`BuildOverview::from_build`], [`LayerDetail::from_build`]), so a source that does not read lazily
//!   needs nothing of its own.

use crate::channel::ChannelDescriptor;
use crate::context::BuildContext;
use crate::identity::{ByLayer, LaserKey, LayerId};
use crate::meta::{BuildIdentity, BuildMeta, LaserLayerMeta, LaserMeta, LayerMeta};
use crate::params::LaserParameters;
use crate::provenance::{Origins, Provenance};
use crate::quality::QualityRecord;
use crate::units::Quantity;

/// A laser as the build's own level names it: its key and its parameters, as the calibration holds them.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LaserOverview {
    pub key: LaserKey,
    /// The parameters of the laser, as its file holds them. `None` for a laser with no calibration in the file.
    pub parameters: Option<LaserParameters>,
    /// What the reader says of how the parameters came to be what they are (not read, and why).
    pub origins: Origins,
}

/// The build's own level and the ids of its layers, read without walking a layer.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BuildOverview {
    pub identity: BuildIdentity,
    /// The number of layers the file *declares*. It is not a count of the layers present.
    pub layers_declared: Option<i64>,
    /// The layer thickness of the build, where the file gives one.
    pub layer_thickness: Option<Quantity>,
    /// The ids of the layers the build holds, in order. Gaps are allowed.
    pub layers: Vec<LayerId>,
    /// The lasers that the build's calibration names, in numeric order, with their parameters. **Not the lasers recorded in the layers**: those are in
    /// [`LayerDetail`] for a layer, and in [`BuildMeta`] for the build.
    pub lasers: Vec<LaserOverview>,
    /// How the build ended, as its file says.
    pub quality: QualityRecord,
    /// What was done to the data after capture, as its file says.
    pub provenance: Provenance,
    /// The machine, the material, the starting height and the build plate.
    pub context: BuildContext,
    pub origins: Origins,
}

impl LaserOverview {
    /// A laser of the calibration with no parameters and nothing said of them.
    pub fn new(key: LaserKey) -> Self {
        LaserOverview {
            key,
            parameters: None,
            origins: Origins::new(),
        }
    }
}

impl BuildOverview {
    /// An overview of a build with this identity and nothing else yet: no layer, no laser, nothing declared.
    pub fn new(identity: BuildIdentity) -> Self {
        BuildOverview {
            identity,
            layers_declared: None,
            layer_thickness: None,
            layers: Vec::new(),
            lasers: Vec::new(),
            quality: QualityRecord::default(),
            provenance: Provenance::default(),
            context: BuildContext::default(),
            origins: Origins::new(),
        }
    }

    /// The overview that a build's own level makes: `level` holds the build-level fields and the lasers that the calibration names (and no layer), and `layers` are the
    /// ids of the layers the build holds.
    pub fn from_level(level: &BuildMeta, layers: Vec<LayerId>) -> BuildOverview {
        BuildOverview {
            identity: level.identity.clone(),
            layers_declared: level.layers_declared,
            layer_thickness: level.layer_thickness.clone(),
            layers,
            lasers: level
                .lasers()
                .iter()
                .map(|laser| LaserOverview {
                    key: laser.key.clone(),
                    parameters: laser.parameters.clone(),
                    origins: laser.origins.clone(),
                })
                .collect(),
            quality: level.quality.clone(),
            provenance: level.provenance.clone(),
            context: level.context.clone(),
            origins: level.origins.clone(),
        }
    }

    /// The overview of a whole build: the lasers are those recorded in its layers.
    pub fn from_build(build: &BuildMeta) -> BuildOverview {
        BuildOverview::from_level(build, build.layers.ids().collect())
    }
}

/// A laser in one layer: its entry there, and the channels it has there, each described with its presence in that layer alone.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LaserInLayer {
    pub key: LaserKey,
    /// What the laser did in the layer: its presence, its time axis, its completeness and what else its group holds.
    pub entry: LaserLayerMeta,
    /// The channels recorded in the layer, in the order of the layout. Each has the layer, and only the layer, in its presence.
    pub channels: Vec<ChannelDescriptor>,
}

impl LaserInLayer {
    pub fn new(key: LaserKey, entry: LaserLayerMeta, channels: Vec<ChannelDescriptor>) -> Self {
        LaserInLayer {
            key,
            entry,
            channels,
        }
    }
}

/// One layer's own metadata: the layer, and the lasers recorded in it.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LayerDetail {
    pub layer: LayerMeta,
    /// The lasers recorded in the layer, in numeric order.
    pub lasers: Vec<LaserInLayer>,
}

impl LayerDetail {
    pub fn new(layer: LayerMeta, lasers: Vec<LaserInLayer>) -> Self {
        LayerDetail { layer, lasers }
    }

    /// The laser `key` as the layer records it, if it does.
    pub fn laser(&self, key: &LaserKey) -> Option<&LaserInLayer> {
        self.lasers.iter().find(|laser| &laser.key == key)
    }

    /// The slice of a whole build that belongs to layer `id`, or `None` if the build does not hold it. A laser is in the slice only if it has an entry in the layer, and
    /// a channel only if it is recorded in the layer.
    pub fn from_build(build: &BuildMeta, id: LayerId) -> Option<LayerDetail> {
        let layer = build.layers.get(id)?.clone();
        let lasers = build
            .lasers()
            .iter()
            .filter_map(|laser| slice_of_laser(laser, id))
            .collect();
        Some(LayerDetail { layer, lasers })
    }
}

fn slice_of_laser(laser: &LaserMeta, id: LayerId) -> Option<LaserInLayer> {
    let entry = laser.layers.get(id)?.clone();
    let channels = laser
        .channels
        .iter()
        .filter_map(|channel| {
            let presence = *channel.presence.get(id)?;
            let mut alone = channel.clone();
            let mut only = ByLayer::new();
            only.insert(id, presence);
            alone.presence = only;
            Some(alone)
        })
        .collect();
    Some(LaserInLayer {
        key: laser.key.clone(),
        entry,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelDType, ChannelKind, ClockClass, NumericType};
    use crate::identity::{ChannelKey, Presence};

    fn identity() -> BuildIdentity {
        BuildIdentity {
            build_id: Some("a build".into()),
            date: None,
            username: None,
            alloy: None,
            part_count: None,
            project_id: None,
            source_mcf_name: None,
            source_mcf_hash: None,
        }
    }

    #[test]
    fn the_overview_of_a_build_has_its_level_its_layer_ids_and_the_lasers_it_records() {
        let mut build = BuildMeta::new(identity());
        build.layers_declared = Some(3);
        build.layers.insert(LayerId(2), LayerMeta::new(LayerId(2)));
        build.layers.insert(LayerId(5), LayerMeta::new(LayerId(5)));
        build.add_laser(LaserMeta::new(LaserKey::new("the_laser", 1)));
        let overview = BuildOverview::from_build(&build);
        assert_eq!(overview.layers, [LayerId(2), LayerId(5)]);
        assert_eq!(overview.layers_declared, Some(3));
        assert_eq!(overview.identity, build.identity);
        assert_eq!(overview.lasers.len(), 1);
        assert_eq!(overview.lasers[0].key, LaserKey::new("the_laser", 1));
    }

    #[test]
    fn the_detail_of_a_layer_is_that_layers_slice_of_the_build_and_nothing_of_the_others() {
        let mut build = BuildMeta::new(identity());
        for n in [1, 2] {
            build.layers.insert(LayerId(n), LayerMeta::new(LayerId(n)));
        }
        let mut laser = LaserMeta::new(LaserKey::new("the_laser", 1));
        // recorded in both layers, with a channel recorded in both
        for n in [1, 2] {
            laser.layers.insert(
                LayerId(n),
                LaserLayerMeta::new(Presence::from_count(10 * u64::from(n))),
            );
        }
        let mut channel = ChannelDescriptor::new(
            ChannelKey::new(["A_Channel"]),
            ChannelKind::LaserPower,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(NumericType::F32),
        );
        for n in [1, 2] {
            channel
                .presence
                .insert(LayerId(n), Presence::from_count(10 * u64::from(n)));
        }
        laser.channels.push(channel);
        build.add_laser(laser);

        let second = LayerDetail::from_build(&build, LayerId(2)).expect("layer 2 is held");
        assert_eq!(second.layer.id, LayerId(2));
        assert_eq!(second.lasers.len(), 1);
        let only = &second.lasers[0];
        assert_eq!(
            only.entry.presence,
            build.lasers()[0].layers.get(LayerId(2)).unwrap().presence
        );
        // the channel carries the layer and only the layer
        assert_eq!(only.channels.len(), 1);
        assert_eq!(only.channels[0].presence.len(), 1);
        assert_eq!(
            only.channels[0].presence.get(LayerId(2)),
            Some(&Presence::from_count(20))
        );
        // a layer the build does not hold has no detail
        assert!(LayerDetail::from_build(&build, LayerId(9)).is_none());
    }
}
