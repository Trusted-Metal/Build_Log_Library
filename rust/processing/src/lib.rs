//! The **Processing layer**: computation derived from Model data (decimation, corrections, conversions, level of detail).
//!
//! * **Layer:** above the Model, beside the IO layer. It works on Model values only, so it can run without HDF5.
//! * **May depend on:** `build-log-model`. In tests only: `build-log-testkit`.
//! * **Must not depend on:** `build-log-io` (directly or through another crate) or the `build-log` facade.
//!
//! This is a skeleton; the processing stages arrive with IP-16 and IP-17. The architecture guards in `rust/build-log/tests` enforce the rules above.

#![forbid(unsafe_code)]
