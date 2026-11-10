//! Provenance: where a value came from, the source of parameters, the stored stage and the decode and sign conventions (decision D02-5: origins are a side table).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** producer (`ProducerInfo`); library (`Provenance`, `Origins`); the enums `ParameterSource`, `Origin`, `StorageStage` and the convention enums.
//! * **Requirement:** BL-CD-11 and BL-CD-13.
//!
//! # What is here so far
//!
//! [`Origin`] and [`Origins`], which phase 2F needs (a placeholder that is read as "not recorded" must say so), and [`Provenance`] with the OPC UA join's status
//! and version, which IP-13 phase 13J needs (decision J14). `ProducerInfo`, `ParameterSource`, `StorageStage` and the decode and sign conventions arrive with
//! the versions that need them (the rest of IP-02 phase 2O).
//!
//! # What this module claims
//!
//! * **Most values come from the file, and say nothing.** The side table holds only the values that did not: it is empty for a file that was read
//!   as it is. A wrapper on every field was rejected as noise (decision D02-5).
//! * **An origin never changes a value.** It says how the value came to be what it is, so that a consumer, a conversion or a conformance check can see
//!   which values were declared, guessed or filled in, and which were taken as written.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorKind};

/// How a value came to be what it is, when that is not simply "the file said so".
// open-domain: ways a value can come to be that nobody has listed yet
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Origin {
    /// The value was not in the file, and the library used its default.
    Defaulted,
    /// The file had a value, and the caller replaced it.
    Overridden,
    /// The value was supplied by the caller, for instance from a machine configuration the application loaded (the library never loads one).
    CallerSupplied,
    /// A reasoned guess, not a fact of the file.
    Inferred,
    /// The file stored the value in a form that had to be read another way, such as a number held as raw bytes in a text attribute.
    Reinterpreted,
    /// The adapter of the version supplied it from the documented conventions of that version, as the stored unit of a raw channel.
    DeclaredByAdapter,
    /// The file held a value that its writer uses when it does not know (a thickness of `0.0`), and it is read as "not recorded". `raw` is that value,
    /// as the file spelled it.
    PlaceholderDropped { raw: String },
    /// The count of samples is that of the **longest** of the channels, because the channels of the stream differ in length and the file states no length for the stream. Nothing is cut or padded:
    /// each channel keeps its own count, and a consumer that aligns channels decides how (IP-07 phase 7A, decision D07-1).
    LongestChannel,
    /// Something the file holds was **not read**, and this is why (IP-07 phase 7B, decision D07-7): a dataset that is not what the version says it is, a value of a type the reader does not
    /// know, a group that cannot be described. The item is left out of the metadata, or described as not recorded, and the build still opens. [`Origin::not_read`] makes it from the
    /// error; the validation report and the availability of an item (IP-07 phases 7C and 7D) are told from it by the cause, and need not read a sentence.
    NotRead {
        cause: NotReadCause,
        /// What the reader said, as text for a person.
        reason: String,
    },
    /// The channel holds no value because its dataset has a **null dataspace**: HDF5 allows a dataset with a type, no data and no dimensions. It is recorded and empty, as a dataset of length
    /// zero is, and this origin tells the two apart (IP-07 phase 7A, decision D07-2).
    NullDataspace,
    /// Another way, by name.
    Other(String),
}

/// Why something was not read (IP-07 phase 7B, decision D07-7): whose fact it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum NotReadCause {
    /// The file is at fault: what it holds is not what its version says it is.
    Fault,
    /// The version or the library does not read this kind of thing (yet). It is a fact about the library, and not about the file.
    Unsupported,
}

impl Origin {
    /// The origin of something that was not read because of `error`: **`Unsupported` where the error says the version does not offer it** (`UnsupportedInVersion`), and `Fault` for any
    /// other error about the content of the file. The reason is what the error says.
    pub fn not_read(error: &Error) -> Origin {
        let cause = match error.kind() {
            ErrorKind::UnsupportedInVersion { .. } => NotReadCause::Unsupported,
            _ => NotReadCause::Fault,
        };
        Origin::NotRead {
            cause,
            reason: error.to_string(),
        }
    }
}

/// How far the post-capture join of OPC UA events to layers got, as the file says (decision J14). Nothing is worked out from the events themselves.
// open-domain: a writer may record a status that nobody has named, and it is kept as the file spells it
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum JoinStatus {
    /// The join ran to its end.
    Complete,
    /// A status that this library has no name for, exactly as the file spells it.
    Other(String),
}

/// What was done to a build's data after it was captured, as its file says. Only the OPC UA join is here so far; a file that records no join has `None` in
/// both fields, which is not the same as a join that did not complete.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Provenance {
    /// The status of the join of OPC UA events to layers.
    pub opcua_join_status: Option<JoinStatus>,
    /// The version of the join, as written (a text such as `2`: it is not read as a number).
    pub opcua_join_version: Option<String>,
}

impl Provenance {
    /// A provenance that says nothing.
    pub fn new() -> Self {
        Provenance::default()
    }
}

/// The origins of the values of one node (a build, a layer, a laser), by field name. Only values that are not plain `FromFile` are in it.
///
/// Adapters set origins through [`Origins::record`], so that none is forgotten (risk R5 of the plan).
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Origins {
    fields: BTreeMap<String, Origin>,
}

impl Origins {
    pub fn new() -> Self {
        Origins::default()
    }

    /// Records how a field came to be what it is, and returns the origin it replaced.
    pub fn record(&mut self, field: impl Into<String>, origin: Origin) -> Option<Origin> {
        self.fields.insert(field.into(), origin)
    }

    /// The origin of a field, or `None` when it is plain `FromFile`.
    pub fn get(&self, field: &str) -> Option<&Origin> {
        self.fields.get(field)
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The fields in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Origin)> {
        self.fields
            .iter()
            .map(|(field, origin)| (field.as_str(), origin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provenance_that_says_nothing_has_no_join_and_a_join_of_another_name_is_kept() {
        let none = Provenance::new();
        assert_eq!(
            (none.opcua_join_status, none.opcua_join_version),
            (None, None)
        );
        let mut some = Provenance::new();
        some.opcua_join_status = Some(JoinStatus::Other("Partial".into()));
        assert_ne!(some.opcua_join_status, Some(JoinStatus::Complete));
    }

    #[test]
    fn origins_hold_only_what_is_not_plain_from_the_file() {
        let mut origins = Origins::new();
        assert!(origins.is_empty());
        assert_eq!(origins.get("thickness"), None);
        let dropped = Origin::PlaceholderDropped { raw: "0.0".into() };
        assert_eq!(origins.record("thickness", dropped.clone()), None);
        assert_eq!(origins.get("thickness"), Some(&dropped));
        // recording again replaces, and says what it replaced
        assert_eq!(origins.record("thickness", Origin::Inferred), Some(dropped));
        origins.record("a_field", Origin::DeclaredByAdapter);
        let names: Vec<&str> = origins.iter().map(|(field, _)| field).collect();
        assert_eq!(names, ["a_field", "thickness"]);
        assert!(!origins.is_empty());
    }

    #[test]
    fn an_unsupported_type_is_told_from_a_fault() {
        // IP-07 phase 7B: the cause follows the kind of the error, and the reason is what the error says
        let unsupported = Error::new(ErrorKind::UnsupportedInVersion {
            version: "17".into(),
            feature: "a column stored as a 16-bit unsigned integer".into(),
        });
        let corrupt = Error::new(ErrorKind::Corrupt {
            detail: "a channel of 0 dimensions".into(),
        });
        let out_of_width = Error::new(ErrorKind::OutOfWidth {
            value: "300".into(),
            stored_as: "an 8-bit integer".into(),
        });
        match Origin::not_read(&unsupported) {
            Origin::NotRead { cause, reason } => {
                assert_eq!(cause, NotReadCause::Unsupported);
                assert_eq!(reason, unsupported.to_string());
                assert!(reason.contains("16-bit unsigned"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        for fault in [&corrupt, &out_of_width] {
            assert!(
                matches!(
                    Origin::not_read(fault),
                    Origin::NotRead {
                        cause: NotReadCause::Fault,
                        ..
                    }
                ),
                "{fault}"
            );
        }
        // it is a type, and neither the free text it replaced nor another origin
        assert_ne!(
            Origin::not_read(&corrupt),
            Origin::Other(corrupt.to_string())
        );
        assert_ne!(Origin::not_read(&corrupt), Origin::not_read(&unsupported));
        assert!(NotReadCause::Fault < NotReadCause::Unsupported);
    }

    #[test]
    fn the_origins_of_a_laser_stream_are_two_typed_facts_and_not_text() {
        // IP-07 phase 7A (D07-1, D07-2): the count of a stream whose channels differ, and a channel whose dataspace is null, say so by a type, so that a consumer matches a type
        assert_ne!(Origin::LongestChannel, Origin::NullDataspace);
        assert_ne!(
            Origin::LongestChannel,
            Origin::Other("LongestChannel".into())
        );
        assert_ne!(
            Origin::NullDataspace,
            Origin::Other("null dataspace".into())
        );
        let mut origins = Origins::new();
        origins.record("stored_samples", Origin::LongestChannel);
        origins.record("channel A_Channel", Origin::NullDataspace);
        assert_eq!(origins.get("stored_samples"), Some(&Origin::LongestChannel));
        assert_eq!(
            origins.get("channel A_Channel"),
            Some(&Origin::NullDataspace)
        );
    }
}
