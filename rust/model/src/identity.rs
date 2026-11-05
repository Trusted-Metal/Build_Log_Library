//! Identity and presence: the keys of layers, lasers and channels, and the two enums for "not there" (`Presence` for the file, `Availability` for a read; decision D02-7).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** producers (`LayerId`, `LaserKey`, `ChannelKey`); library (`ByLayer`); the enums `Presence` and `Availability`.
//! * **Requirement:** BL-CD-04.
//!
//! # What this module claims, and what it does not
//!
//! * **A layer is its file's own id, and ids have gaps.** A file can hold layers 1, 2 and 2110 and nothing between, and the number of layers a file
//!   *declares* is not the number it *holds*. Nothing here counts layers or fills a gap.
//! * **Lasers order by number, never by text.** `Laser_10` sorts before `Laser_2` as text, so a [`LaserKey`] orders by its display number.
//! * **"Not there" has three meanings, and none of them is zero or an error.** [`Presence`] says what the *file* holds: samples, a recorded but empty
//!   stream (V16 has an empty second laser in every layer), or nothing recorded. [`Availability`] says what a *read* found, and can carry the
//!   issues or the error of that one item without failing the others (BL-FA-09).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;

use crate::error::{Error, Violation};

/// A layer, by the id its file gives it. Ids need not be contiguous and need not start at 1.
// constructed-by: producer
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerId(pub u32);

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A laser, by a stable key and the number it is shown with.
///
/// The ordering is by `number` and then by `key`, so laser 10 comes after laser 2. The `key` identifies the laser (it is what the file calls it) and is
/// never compared as a number.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LaserKey {
    /// The stable key: the file's own name for the laser.
    pub key: String,
    /// The number the laser is shown with, such as 1, 2 or 10. Numbers need not be contiguous.
    pub number: u32,
}

impl LaserKey {
    pub fn new(key: impl Into<String>, number: u32) -> Self {
        LaserKey {
            key: key.into(),
            number,
        }
    }
}

impl Ord for LaserKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.number
            .cmp(&other.number)
            .then_with(|| self.key.cmp(&other.key))
    }
}

impl PartialOrd for LaserKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for LaserKey {
    /// The display number, which is what a person reads: `10`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.number)
    }
}

/// A channel, by a hierarchical path of segments. A segment is a whole name and may hold any character, so the path is a list, not a string to be split.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelKey {
    pub segments: Vec<String>,
}

impl ChannelKey {
    pub fn new<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ChannelKey {
            segments: segments.into_iter().map(Into::into).collect(),
        }
    }

    /// The last segment: the name of the channel within its parent.
    pub fn name(&self) -> Option<&str> {
        self.segments.last().map(String::as_str)
    }

    /// The path without its last segment, or `None` for a path of one segment or none.
    pub fn parent(&self) -> Option<ChannelKey> {
        (self.segments.len() > 1).then(|| ChannelKey {
            segments: self.segments[..self.segments.len() - 1].to_vec(),
        })
    }
}

impl fmt::Display for ChannelKey {
    /// The segments joined by a slash. For messages only: a segment may itself hold a slash.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.segments.join("/"))
    }
}

/// A value for some of the layers of a build, by layer id. A layer that is not in it is a layer the value was not recorded for: a gap is meaningful, not
/// a bug, and it is not a zero. Layers are visited in id order.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ByLayer<T> {
    layers: BTreeMap<LayerId, T>,
}

impl<T> Default for ByLayer<T> {
    fn default() -> Self {
        ByLayer::new()
    }
}

impl<T> ByLayer<T> {
    pub fn new() -> Self {
        ByLayer {
            layers: BTreeMap::new(),
        }
    }

    /// Sets the value of a layer, and returns the value it replaced.
    pub fn insert(&mut self, id: LayerId, value: T) -> Option<T> {
        self.layers.insert(id, value)
    }

    pub fn get(&self, id: LayerId) -> Option<&T> {
        self.layers.get(&id)
    }

    pub fn get_mut(&mut self, id: LayerId) -> Option<&mut T> {
        self.layers.get_mut(&id)
    }

    pub fn contains(&self, id: LayerId) -> bool {
        self.layers.contains_key(&id)
    }

    /// How many layers hold a value. This is the number present, never the number a file declares.
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (LayerId, &T)> {
        self.layers.iter().map(|(id, value)| (*id, value))
    }

    pub fn ids(&self) -> impl Iterator<Item = LayerId> + '_ {
        self.layers.keys().copied()
    }
}

impl<T> FromIterator<(LayerId, T)> for ByLayer<T> {
    fn from_iter<I: IntoIterator<Item = (LayerId, T)>>(iter: I) -> Self {
        ByLayer {
            layers: iter.into_iter().collect(),
        }
    }
}

/// What a file holds for one stream (a laser in a layer, a channel): the description of the file, not of a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Presence {
    /// Recorded, with this many stored samples. Never zero: a stream with no samples is [`Presence::RecordedEmpty`].
    Recorded { samples: NonZeroU64 },
    /// The stream is in the file and holds no samples (V16 keeps an empty second laser in every layer).
    RecordedEmpty,
    /// The file has nothing for it. This is not an empty stream and not a zero.
    NotRecorded,
}

impl Presence {
    /// The presence of a stream that is in the file and holds `samples` samples: zero is a recorded empty stream.
    pub fn from_count(samples: u64) -> Presence {
        match NonZeroU64::new(samples) {
            Some(samples) => Presence::Recorded { samples },
            None => Presence::RecordedEmpty,
        }
    }

    /// How many samples are stored: `Some(0)` for a recorded empty stream, and `None` where nothing is recorded.
    pub fn samples(&self) -> Option<u64> {
        match self {
            Presence::Recorded { samples } => Some(samples.get()),
            Presence::RecordedEmpty => Some(0),
            Presence::NotRecorded => None,
        }
    }
}

/// What a read found for one item (a laser, a channel, a sensor). One item failing does not fail the others: the outcome is reported on the result.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Availability {
    /// The item was read.
    Present,
    /// The file does not hold it. This is an outcome, not an error.
    NotRecorded,
    /// The item was read, with these issues.
    Degraded { issues: Vec<Violation> },
    /// The item is in the file and could not be read.
    Unreadable { error: Box<Error> },
    /// The version or the library does not offer this item; the feature is named.
    Unsupported { feature: String },
}

impl Availability {
    /// Was the item read, whether or not with issues?
    pub fn is_readable(&self) -> bool {
        matches!(self, Availability::Present | Availability::Degraded { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    fn laser(key: &str, number: u32) -> LaserKey {
        LaserKey::new(key, number)
    }

    #[test]
    fn laser_10_sorts_after_laser_2() {
        // as text, "10" comes before "2": the trap this type exists to avoid
        assert!("10" < "2");
        let mut lasers = [laser("10", 10), laser("2", 2), laser("1", 1)];
        lasers.sort();
        let numbers: Vec<u32> = lasers.iter().map(|l| l.number).collect();
        assert_eq!(numbers, [1, 2, 10]);
        assert!(laser("2", 2) < laser("10", 10));
        // two lasers with one number are told apart by their keys
        assert_ne!(laser("a", 3), laser("b", 3));
        assert_eq!(laser("a", 3).cmp(&laser("b", 3)), Ordering::Less);
        assert_eq!(laser("10", 10).to_string(), "10");
    }

    #[test]
    fn layer_ids_with_gaps_are_preserved() {
        let layers: ByLayer<&str> = [(1, "a"), (2, "b"), (5, "c"), (2110, "d")]
            .into_iter()
            .map(|(id, value)| (LayerId(id), value))
            .collect();
        let ids: Vec<u32> = layers.ids().map(|id| id.0).collect();
        assert_eq!(ids, [1, 2, 5, 2110]);
        assert_eq!(layers.len(), 4);
        assert!(!layers.contains(LayerId(3)));
        assert_eq!(layers.get(LayerId(3)), None);
        assert_eq!(layers.get(LayerId(5)), Some(&"c"));
        // insertion order does not matter, and a later insert replaces
        let mut other = ByLayer::new();
        other.insert(LayerId(2110), "d");
        other.insert(LayerId(1), "a");
        assert_eq!(other.insert(LayerId(1), "z"), Some("a"));
        assert_eq!(other.iter().next(), Some((LayerId(1), &"z")));
        assert!(ByLayer::<u8>::default().is_empty());
    }

    #[test]
    fn recorded_empty_is_distinct_from_not_recorded_and_from_error() {
        assert_ne!(Presence::RecordedEmpty, Presence::NotRecorded);
        assert_ne!(Presence::RecordedEmpty, Presence::from_count(1));
        let outcomes = [
            Availability::Present,
            Availability::NotRecorded,
            Availability::Degraded {
                issues: vec![Violation::new("a gap")],
            },
            Availability::Unreadable {
                error: Box::new(Error::new(ErrorKind::Corrupt {
                    detail: "bad".into(),
                })),
            },
            Availability::Unsupported {
                feature: "images".into(),
            },
        ];
        // each outcome is its own arm: a missing item is not an error, and an error is not a missing item
        assert!(matches!(outcomes[1], Availability::NotRecorded));
        assert!(matches!(outcomes[3], Availability::Unreadable { .. }));
        assert!(outcomes[0].is_readable() && outcomes[2].is_readable());
        assert!(
            !outcomes[1].is_readable() && !outcomes[3].is_readable() && !outcomes[4].is_readable()
        );
    }

    #[test]
    fn not_recorded_is_not_zero() {
        assert_eq!(Presence::NotRecorded.samples(), None);
        assert_eq!(Presence::RecordedEmpty.samples(), Some(0));
        assert_eq!(Presence::from_count(0), Presence::RecordedEmpty);
        assert_eq!(Presence::from_count(727_922).samples(), Some(727_922));
    }

    #[test]
    fn channel_keys_are_hierarchical_paths() {
        let key = ChannelKey::new(["position", "actual", "x"]);
        assert_eq!(key.to_string(), "position/actual/x");
        assert_eq!(key.name(), Some("x"));
        assert_eq!(key.parent(), Some(ChannelKey::new(["position", "actual"])));
        assert_eq!(ChannelKey::new(["power"]).parent(), None);
        // a segment is a whole name: a slash inside one does not split it
        let odd = ChannelKey::new(["a/b", "c"]);
        assert_eq!(odd.segments.len(), 2);
        assert_ne!(odd, ChannelKey::new(["a", "b", "c"]));
    }
}
