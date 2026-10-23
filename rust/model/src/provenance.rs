//! Provenance: where a value came from, the source of parameters, the stored stage and the decode and sign conventions (decision D02-5: origins are a side table).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** producer (`ProducerInfo`); library (`Provenance`, `Origins`); the enums `ParameterSource`, `Origin`, `StorageStage` and the convention enums.
//! * **Requirement:** BL-CD-11 and BL-CD-13.
//!
//! # What is here so far
//!
//! Only [`Origin`] and [`Origins`], which phase 2F needs (a placeholder that is read as "not recorded" must say so). `ProducerInfo`, `Provenance`,
//! `ParameterSource`, `StorageStage` and the decode and sign conventions arrive with IP-02 phase 2O.
//!
//! # What this module claims
//!
//! * **Most values come from the file, and say nothing.** The side table holds only the values that did not: it is empty for a file that was read
//!   as it is. A wrapper on every field was rejected as noise (decision D02-5).
//! * **An origin never changes a value.** It says how the value came to be what it is, so that a consumer, a conversion or a conformance check can see
//!   which values were declared, guessed or filled in, and which were taken as written.

use std::collections::BTreeMap;

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
    /// Another way, by name.
    Other(String),
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
}
