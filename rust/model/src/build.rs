use std::collections::BTreeMap;

use crate::laser::Laser;
use crate::sensor::Sensor;

/// Build-level metadata, corresponding to the attributes on `/Build` in the
/// reference fixture (`reference_build_log_opcua_and_clearbox_sensors.h5`).
/// All ten attributes are scalar; confirmed dtypes are per that one fixture
/// and have not yet been cross-checked against a second build.
///
/// `alloy` and `project_id` were observed as empty strings in this fixture
/// instance — structurally present, not omitted, but unpopulated for this
/// particular build.
///
/// `lasers`/`sensors` are keyed by their HDF5 group name (e.g. `"Laser_1"`,
/// `"Chamber_Oxygen_Sensor_OPCUA"`) rather than held as a bare `Vec`.
/// Neither `Laser` nor `Sensor` stores its own name — that was deliberately
/// excluded from each leaf type — so identity has to live somewhere, and a
/// bare `Vec` would silently lose it (nothing would distinguish element 0
/// from element 1 except iteration order). Keying the collection at this
/// level keeps the leaf types themselves identity-free while not losing
/// the information entirely. `BTreeMap` over `HashMap` for deterministic
/// iteration order. Neither count is assumed fixed — confirmed builds can
/// have more than two lasers, and there's no evidence sensor count/set is
/// fixed either.
///
/// See `docs/models/derivation.md` for the full fixture-path mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct Build {
    pub lasers: BTreeMap<String, Laser>,
    pub sensors: BTreeMap<String, Sensor>,
    pub alloy: String,
    /// Observed as a version-like string (e.g. `"4.9.7.1"`), not a numeric
    /// or UUID identifier — despite the name.
    pub build_id: String,
    /// Format observed: `"2026-08-18 16:48:25 UTC+00:00"`. Kept as a raw
    /// string rather than parsed — no date/time crate dependency exists
    /// yet, and picking one plus a canonical form is a separate decision,
    /// not made here. (This used to also need to reconcile a second,
    /// differently-formatted date field, `Source_MCF_Export_Date` — that
    /// field was removed from the fixture and replaced with
    /// `source_mcf_name` below, so `date` is the only date-shaped field
    /// left in this group.)
    pub date: String,
    pub layer_quantity: i32,
    pub layer_thickness: f32,
    /// Added to the fixture alongside `layer_thickness` after review caught
    /// it was the only physical-measurement attribute in the file missing
    /// its `_units` companion. Value observed: `"mm"` (consistent with
    /// `layer_thickness = 0.03`, i.e. a 30 µm layer — not `"µm"` directly,
    /// which the raw number would make physically implausible).
    pub layer_thickness_units: String,
    pub number_of_parts: i32,
    pub project_id: String,
    /// Hash of the *source MCF configuration* this build was exported
    /// from, not a hash of this build log itself.
    pub source_mcf_configuration_hash: String,
    /// Replaces the fixture's former `Source_MCF_Export_Date` attribute —
    /// removed since a name is more directly useful for tracing which MCF
    /// config was used than an export timestamp is. Value is currently a
    /// placeholder (`"reference_config_v1_1.h5"`), not sourced from a real
    /// capture.
    pub source_mcf_name: String,
    pub username: String,
}
