//! The parameter snapshot: per-laser parameters, equations as a type plus named constants, and the descriptors of correction grids and files.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** streams.
//! * **Constructed by:** producers (`LaserParameters`, `Equation`, `CorrectionGrid` as a descriptor, `CorrectionFileInfo`); the enums `GridDomain`, `AxisOrder` and `GalvoProtocol`.
//! * **Requirement:** BL-CD-12.
//!
//! Nothing is here yet: the types arrive with IP-02 phase 2N. The policy guards in `tests/policy_guards.rs` scan this file from now on: every enum is
//! `#[non_exhaustive]` or says `// closed: <reason>`, every public struct says `// constructed-by: producer|library|caller-options` and follows that rule,
//! and none holds bulk data.
