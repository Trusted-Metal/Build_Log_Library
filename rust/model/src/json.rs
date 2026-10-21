//! Canonical metadata JSON (RFC 8785 with the rules of decision D02-10) and the export of the JSON Schema.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** none: functions only.
//! * **Requirement:** BL-CD-17.
//!
//! Nothing is here yet: the types arrive with IP-02 phase 2I and 2S. The policy guards in `tests/policy_guards.rs` scan this file from now on: every enum is
//! `#[non_exhaustive]` or says `// closed: <reason>`, every public struct says `// constructed-by: producer|library|caller-options` and follows that rule,
//! and none holds bulk data.
