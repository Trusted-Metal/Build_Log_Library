//! Channel descriptors: key, kind, clock class, data type, unit, captured width and renderability, with a per-layer presence. Sensor modelling stays minimal and provisional (decision D02-16).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** streams and telemetry.
//! * **Constructed by:** producers (`ChannelDescriptor`, `CapturedWidth`, `EnumLabel`); the enums `ChannelKind`, `ClockClass`, `ChannelDType`, `Renderability` and the enums they use.
//! * **Requirement:** BL-CD-05, BL-ST-07.
//!
//! # What this module claims, and what it does not
//!
//! * **A descriptor describes a stream and holds none of it.** The samples are bulk data and are reached through a column accessor (BL-CD-03); the descriptor
//!   says what the stream is, how it is clocked, what type it is stored in, and in which layers it was recorded.
//! * **The kind is open.** A kind that this library does not know is carried as [`ChannelKind::Other`] and the descriptor says it cannot be drawn, with the
//!   reason: that is a description, not an error (BL-CD-05).
//! * **The stored type and the captured width are two facts.** A commanded position of 16 bits in offset-binary form can be stored in an `i32`; the width
//!   the hardware delivered is [`CapturedWidth`], and it is not read off the storage type (BL-ST-07). No file we hold records it yet, so it is optional.
//! * **A unit says where it came from.** The stored unit of a raw channel is declared by the adapter of the version (laser power in volts, positions in
//!   bits: decision J7 of the field inventory); anything the file or the adapter does not say is unknown.
//! * **Sensors are minimal and provisional** (decision D02-16): a sensor is a channel with a kind, a clock class and a type, and nothing more is added
//!   until a real case needs it.

use crate::error::{Error, ErrorKind, Violation};
use crate::identity::{ByLayer, ChannelKey, Presence};
use crate::units::{Unit, UnitSource};

/// An axis of the scanner. `Z` is the focus axis, which is not the height of the build.
// closed: a laser scans in X and Y and focuses along Z, and the physical axes do not grow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// What a channel records. Open: files and consumers bring kinds that nobody has listed, and those are carried as `Other` and are not drawn.
// open-domain: kinds of channel that files or consumers may bring that nobody has listed yet
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChannelKind {
    /// The power of the laser.
    LaserPower,
    /// Whether the laser is on. An enum with declared labels where they are known (see [`ChannelDescriptor::laser_state`]).
    LaserState,
    /// Where the scanner is, as measured.
    ActualPosition { axis: Axis },
    /// Where the scanner was told to be.
    CommandedPosition { axis: Axis },
    /// A sampled sensor of any other kind. Provisional (decision D02-16).
    Sensor,
    /// A camera image.
    Image,
    /// A kind this library does not know, by the name the file gives it.
    Other(String),
}

/// How a channel is clocked: what its samples are tied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClockClass {
    /// One sample for each sample of the laser stream.
    LaserSynchronous,
    /// Each sample carries its own time.
    Timestamped,
    /// Values that happen at moments (an event), not at a rate.
    Event,
    /// A value that does not change over the layer.
    Static,
    /// An image, taken once, before or after the exposure.
    Image,
}

/// The number type a channel is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NumericType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}

/// One declared state of an enum channel: the stored number and its name.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumLabel {
    pub value: i64,
    pub name: String,
}

/// The states of an enum channel, declared or not. A file that does not say what its values mean gets `Unknown`: nothing is guessed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EnumLabels {
    Declared(Vec<EnumLabel>),
    Unknown,
}

/// The type of a channel's values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChannelDType {
    Numeric(NumericType),
    Bool,
    /// Discrete states, with their labels where they are declared.
    Enum {
        labels: EnumLabels,
    },
    Text,
}

impl ChannelDType {
    /// The declared labels of an enum channel, or `None` for a channel that is not an enum or whose labels are not declared.
    pub fn labels(&self) -> Option<&[EnumLabel]> {
        match self {
            ChannelDType::Enum {
                labels: EnumLabels::Declared(labels),
            } => Some(labels),
            _ => None,
        }
    }
}

/// How the bits of a captured value are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CapturedEncoding {
    Unsigned,
    TwosComplement,
    /// The zero sits in the middle of the range (a 16-bit commanded position on one protocol).
    OffsetBinary,
    /// The width is known and the encoding is not.
    Unknown,
}

/// The width and encoding of a value as the hardware delivered it, which is not the type it is stored in.
// constructed-by: producer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapturedWidth {
    pub bits: u8,
    pub encoding: CapturedEncoding,
}

/// Whether a channel can be drawn.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Renderability {
    Renderable,
    /// It cannot be drawn, and the reason says why. This is a description of the channel, not an error.
    Unrenderable {
        reason: String,
    },
}

impl Renderability {
    /// What a viewer can do with a channel of this kind: every kind this library knows can be drawn, and a kind it does not know cannot, for that reason.
    pub fn for_kind(kind: &ChannelKind) -> Renderability {
        match kind {
            ChannelKind::LaserPower
            | ChannelKind::LaserState
            | ChannelKind::ActualPosition { .. }
            | ChannelKind::CommandedPosition { .. }
            | ChannelKind::Sensor
            | ChannelKind::Image => Renderability::Renderable,
            ChannelKind::Other(name) => Renderability::Unrenderable {
                reason: format!("the channel kind `{name}` is not known to this library"),
            },
        }
    }
}

/// A stream of samples, described: what it is, how it is clocked, what it is stored as and in what unit, and in which layers it was recorded.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDescriptor {
    pub key: ChannelKey,
    pub kind: ChannelKind,
    pub clock_class: ClockClass,
    pub dtype: ChannelDType,
    /// The unit of the stored values, when anyone says. Unknown otherwise.
    pub unit: Unit,
    /// Where the unit came from: the file, the adapter of the version, or nowhere (unknown).
    pub unit_source: UnitSource,
    /// The width the hardware delivered, where the file records it. No file we hold does yet.
    pub captured_width: Option<CapturedWidth>,
    pub renderability: Renderability,
    /// In which layers the channel was recorded, and with how many samples.
    pub presence: ByLayer<Presence>,
}

impl ChannelDescriptor {
    /// A descriptor with an unknown unit, no captured width and no layers. Its renderability follows from its kind.
    pub fn new(
        key: ChannelKey,
        kind: ChannelKind,
        clock_class: ClockClass,
        dtype: ChannelDType,
    ) -> Self {
        let renderability = Renderability::for_kind(&kind);
        ChannelDescriptor {
            key,
            kind,
            clock_class,
            dtype,
            unit: Unit::unknown(),
            unit_source: UnitSource::Unknown,
            captured_width: None,
            renderability,
            presence: ByLayer::new(),
        }
    }

    /// The laser state: an enum, never a bare number, synchronous with the laser stream. Pass `EnumLabels::Unknown` where the file does not say what its
    /// values mean (the default, as nothing is guessed).
    pub fn laser_state(key: ChannelKey, labels: EnumLabels) -> Self {
        ChannelDescriptor::new(
            key,
            ChannelKind::LaserState,
            ClockClass::LaserSynchronous,
            ChannelDType::Enum { labels },
        )
    }

    pub fn with_unit(mut self, unit: Unit, source: UnitSource) -> Self {
        self.unit = unit;
        self.unit_source = source;
        self
    }

    pub fn with_captured_width(mut self, width: CapturedWidth) -> Self {
        self.captured_width = Some(width);
        self
    }

    pub fn with_presence(mut self, presence: ByLayer<Presence>) -> Self {
        self.presence = presence;
        self
    }

    /// Checks the descriptor against what its kind requires, and reports every violation: the laser state must be an enum.
    pub fn check(&self) -> Result<(), Error> {
        let mut violations = Vec::new();
        if self.kind == ChannelKind::LaserState && !matches!(self.dtype, ChannelDType::Enum { .. })
        {
            violations.push(Violation::new(format!(
                "the laser state `{}` must be an enum, not a bare number",
                self.key
            )));
        }
        if let Some(width) = self.captured_width {
            // a zero is how a producer's config says "not set": it is absent (`None`), never a width of nothing
            if !(1..=64).contains(&width.bits) {
                violations.push(Violation::new(format!(
                    "the captured width of `{}` is {} bits, which is not between 1 and 64; leave it unset when it is not known",
                    self.key, width.bits
                )));
            } else if let ChannelDType::Numeric(stored) = &self.dtype {
                // a value of n bits cannot be held by an integer of fewer bits; a float has no such limit (a power of 12 bits is stored as `f32` volts)
                if let Some(stored_bits) = stored.integer_bits().filter(|s| width.bits > *s) {
                    violations.push(Violation::new(format!(
                        "the captured width of `{}` is {} bits, which does not fit the {}-bit integer it is stored in",
                        self.key, width.bits, stored_bits
                    )));
                }
            }
        }
        if violations.is_empty() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::ValidationError { violations }))
        }
    }
}

impl NumericType {
    /// The width of an integer type in bits; `None` for a float.
    fn integer_bits(self) -> Option<u8> {
        match self {
            NumericType::I8 | NumericType::U8 => Some(8),
            NumericType::I16 | NumericType::U16 => Some(16),
            NumericType::I32 | NumericType::U32 => Some(32),
            NumericType::I64 | NumericType::U64 => Some(64),
            NumericType::F32 | NumericType::F64 => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::LayerId;

    fn key(name: &str) -> ChannelKey {
        ChannelKey::new([name])
    }

    #[test]
    fn unknown_channel_kind_is_unrenderable_with_reason_not_an_error() {
        // building the descriptor of an unknown kind succeeds: it is a description
        let unknown = ChannelDescriptor::new(
            key("thermal"),
            ChannelKind::Other("thermal_camera".into()),
            ClockClass::Timestamped,
            ChannelDType::Numeric(NumericType::F32),
        );
        match &unknown.renderability {
            Renderability::Unrenderable { reason } => {
                assert!(reason.contains("thermal_camera"), "{reason}")
            }
            other => panic!("expected an unrenderable channel, found {other:?}"),
        }
        assert!(
            unknown.check().is_ok(),
            "an unknown kind is not a violation"
        );
        // every kind the library knows can be drawn
        for kind in [
            ChannelKind::LaserPower,
            ChannelKind::ActualPosition { axis: Axis::X },
            ChannelKind::CommandedPosition { axis: Axis::Z },
            ChannelKind::Sensor,
            ChannelKind::Image,
        ] {
            assert_eq!(
                Renderability::for_kind(&kind),
                Renderability::Renderable,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn enum_channel_declares_labels_or_says_unknown() {
        let declared = ChannelDType::Enum {
            labels: EnumLabels::Declared(vec![
                EnumLabel {
                    value: 0,
                    name: "off".into(),
                },
                EnumLabel {
                    value: 1,
                    name: "on".into(),
                },
            ]),
        };
        let labels = declared.labels().expect("the labels are declared");
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[1].name, "on");
        // a file that does not say what its values mean: unknown, and nothing is guessed
        let unknown = ChannelDType::Enum {
            labels: EnumLabels::Unknown,
        };
        assert_eq!(unknown.labels(), None);
        assert_ne!(declared, unknown);
        // a number type has no labels at all
        assert_eq!(ChannelDType::Numeric(NumericType::U8).labels(), None);
    }

    #[test]
    fn laser_state_is_an_enum_not_a_bare_u8() {
        let state = ChannelDescriptor::laser_state(key("state"), EnumLabels::Unknown);
        assert_eq!(state.kind, ChannelKind::LaserState);
        assert!(matches!(
            state.dtype,
            ChannelDType::Enum {
                labels: EnumLabels::Unknown
            }
        ));
        assert_eq!(state.clock_class, ClockClass::LaserSynchronous);
        assert!(state.check().is_ok());
        // a descriptor built by hand with a bare number for the laser state is reported
        let bare = ChannelDescriptor::new(
            key("state"),
            ChannelKind::LaserState,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(NumericType::U8),
        );
        let error = bare.check().unwrap_err();
        assert!(error.to_string().contains("must be an enum"), "{error}");
    }

    #[test]
    fn a_captured_width_of_nothing_or_wider_than_its_integer_is_a_violation() {
        let with = |dtype, bits| {
            ChannelDescriptor::new(
                key("commanded_x"),
                ChannelKind::CommandedPosition { axis: Axis::X },
                ClockClass::LaserSynchronous,
                ChannelDType::Numeric(dtype),
            )
            .with_captured_width(CapturedWidth {
                bits,
                encoding: CapturedEncoding::OffsetBinary,
            })
        };
        // zero bits is "not set" in a producer's config, and is never a width
        assert!(with(NumericType::I32, 0).check().is_err());
        assert!(with(NumericType::I32, 65).check().is_err());
        // 18 bits cannot be held by an `i16`; 16 bits can, and so can 20 bits in an `i32`
        assert!(with(NumericType::I16, 18).check().is_err());
        assert!(with(NumericType::I16, 16).check().is_ok());
        assert!(with(NumericType::I32, 20).check().is_ok());
        // a float has no such limit: a 12-bit power is stored as `f32`
        assert!(with(NumericType::F32, 12).check().is_ok());
        // the violation names the channel
        let err = with(NumericType::I32, 0).check().unwrap_err();
        assert!(format!("{err:?}").contains("commanded_x"), "{err:?}");
    }

    #[test]
    fn captured_width_is_recorded_independently_of_storage_dtype() {
        let position = |dtype| {
            ChannelDescriptor::new(
                key("commanded_x"),
                ChannelKind::CommandedPosition { axis: Axis::X },
                ClockClass::LaserSynchronous,
                ChannelDType::Numeric(dtype),
            )
        };
        // the same storage type, two captured widths: the width is not read off the type
        let narrow = position(NumericType::I32).with_captured_width(CapturedWidth {
            bits: 16,
            encoding: CapturedEncoding::OffsetBinary,
        });
        let wide = position(NumericType::I32).with_captured_width(CapturedWidth {
            bits: 20,
            encoding: CapturedEncoding::TwosComplement,
        });
        assert_eq!(narrow.dtype, wide.dtype);
        assert_ne!(narrow.captured_width, wide.captured_width);
        assert_eq!(narrow.captured_width.unwrap().bits, 16);
        // and a file that records none leaves it absent: nothing is derived from the type
        assert_eq!(position(NumericType::I16).captured_width, None);
    }

    #[test]
    fn a_channel_says_its_unit_and_where_it_came_from_and_the_layers_it_is_in() {
        let layers: ByLayer<Presence> = [(1, 727_922), (2, 719_160), (4, 0)]
            .into_iter()
            .map(|(id, samples)| (LayerId(id), Presence::from_count(samples)))
            .collect();
        let power = ChannelDescriptor::new(
            key("power"),
            ChannelKind::LaserPower,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(NumericType::F32),
        );
        // by default nobody has said: unknown
        assert!(!power.unit.is_known());
        assert_eq!(power.unit_source, UnitSource::Unknown);
        // the adapter declares the stored unit of a raw channel (J7), and says so
        let power = power
            .with_unit(Unit::parse("V"), UnitSource::DeclaredByAdapter)
            .with_presence(layers);
        assert!(power.unit.is_known());
        assert_eq!(power.unit_source, UnitSource::DeclaredByAdapter);
        // present in layers 1 and 2, recorded empty in 4, not recorded in 3
        assert_eq!(
            power.presence.get(LayerId(1)).and_then(Presence::samples),
            Some(727_922)
        );
        assert_eq!(
            power.presence.get(LayerId(4)),
            Some(&Presence::RecordedEmpty)
        );
        assert_eq!(power.presence.get(LayerId(3)), None);
    }
}
