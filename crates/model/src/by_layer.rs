use std::collections::BTreeMap;

/// A per-build-layer value that may be genuinely absent for some layers —
/// gaps are meaningful (the sensor/data simply wasn't recorded that
/// layer), not bugs. Verified across all 42 entries under `/Build/Sensors`
/// and Laser's own `Monitoring_Data`: presence varies per layer,
/// independently, for every kind of sensor — not just telemetry-specific
/// ones.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ByLayer<T>(pub BTreeMap<u32, T>);
