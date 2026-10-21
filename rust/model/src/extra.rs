//! The `extra` passthrough: unknown attributes and groups kept so that they round-trip verbatim.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** library (`Extra`, `UnknownGroupRef`); the enum `AttrValue`.
//! * **Requirement:** BL-CD-08.
//!
//! Nothing is here yet: the types arrive with IP-02 phase 2P. The policy guards in `tests/policy_guards.rs` scan this file from now on: every enum is
//! `#[non_exhaustive]` or says `// closed: <reason>`, every public struct says `// constructed-by: producer|library|caller-options` and follows that rule,
//! and none holds bulk data.
