//! The **IO layer**: reads and writes HDF5 files, translating between a file's own layout and the Model.
//!
//! * **Layer:** between the Model and the files. Version adapters (V16, V17, V18) live here, one module per version, from IP-03 and IP-04 on.
//! * **May depend on:** `build-log-model`. In tests only: `build-log-testkit`.
//! * **Must not depend on:** `build-log-processing` or the `build-log` facade. Nothing above it may reach an HDF5 path or group name through it.
//!
//! This is a skeleton. The HDF5 binding (`hdf5-metno`) and the link test arrive with IP-01 phase 1D; the first adapter with IP-05.
//! The architecture guards in `rust/build-log/tests` enforce the dependency rules above.

// Unsafe code is denied for every crate by the workspace lints; this crate needs none while it uses the `hdf5-metno` wrapper (decision D01-7),
// so it stays at `deny` rather than `forbid`.
