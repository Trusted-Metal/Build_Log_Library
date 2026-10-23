//! Outcomes and the quality record, with "none available" expressible.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** streams.
//! * **Constructed by:** library (`QualityRecord`, `GatingStats`); the enums `Outcome` and `SequenceIntegrity`.
//! * **Requirement:** BL-CD-14.
//!
//! Nothing is here yet: the types arrive with IP-02 phase 2O. The policy guards in `tests/policy_guards.rs` scan this file from now on: every enum is
//! `#[non_exhaustive]` or says `// closed: <reason>`, every public struct says `// constructed-by: producer|library|caller-options` and follows that rule,
//! and none holds bulk data.
