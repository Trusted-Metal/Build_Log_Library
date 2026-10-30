//! Outcomes and the quality record, with "none available" expressible.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** streams.
//! * **Constructed by:** library (`QualityRecord`, `GatingStats`); the enums `Outcome` and `SequenceIntegrity`.
//! * **Requirement:** BL-CD-14.
//!
//! # What is here so far
//!
//! Only [`Outcome`] and [`QualityRecord`] with the build's outcome, which IP-13 phase 13J needs (decision J14: the build status of a V17 file is the build's
//! outcome). The layer outcomes (truncated, failed with a reason), the counts of dropped telemetry and connection losses, `GatingStats` and
//! `SequenceIntegrity` arrive with the versions that record them (the rest of IP-02 phase 2O). A laser's `complete` flag is not here: it is a fact of one
//! laser in one layer, so it sits on [`crate::LaserLayerMeta`] (decision J9).
//!
//! The policy guards in `tests/policy_guards.rs` scan this file from now on: every enum is `#[non_exhaustive]` or says `// closed: <reason>`, every public
//! struct says `// constructed-by: producer|library|caller-options` and follows that rule, and none holds bulk data.

/// How a build ended, as its file says. **It is never worked out**: a file with no status is [`Outcome::NotRecorded`], not complete and not failed.
// open-domain: the text a writer chooses for its status is not a closed list, and a status that nobody has named is kept as the file spells it
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Outcome {
    /// The file says nothing about how the build ended.
    #[default]
    NotRecorded,
    /// The writer marked the build finished.
    Complete,
    /// The writer had not marked the build finished when the file was last written: the build was still running, or it stopped without being marked. The file
    /// does not say which (tauri has no crash recovery), so neither does this.
    InProgress,
    /// A status that this library has no name for, exactly as the file spells it.
    Other(String),
}

/// What a build's file says about how the capture went. Only the outcome is here so far.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct QualityRecord {
    /// How the build ended.
    pub outcome: Outcome,
}

impl QualityRecord {
    /// A record that says nothing: the outcome is not recorded.
    pub fn new() -> Self {
        QualityRecord::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_that_says_nothing_has_no_outcome_and_is_not_a_complete_one() {
        let record = QualityRecord::new();
        assert_eq!(record.outcome, Outcome::NotRecorded);
        assert_ne!(record.outcome, Outcome::Complete);
        assert_eq!(Outcome::default(), Outcome::NotRecorded);
    }

    #[test]
    fn a_status_with_no_name_is_kept_as_the_file_spells_it() {
        let other = Outcome::Other("Complete".into());
        // the same word in another case is not the named outcome: nothing is matched loosely
        assert_ne!(other, Outcome::Complete);
        assert_eq!(other.clone(), other);
    }
}
