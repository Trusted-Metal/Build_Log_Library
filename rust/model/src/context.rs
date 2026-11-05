//! The build as a whole, beyond its identity: the machine, the material, the starting height and the build plate (IP-02 phase 2V, task T02.30; decisions D02-20 and D02-21; judgement
//! items J3 and J4 of the field inventory).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group, a dataset or an attribute of a file. **Group (decision G1):** streams.
//! * **Constructed by:** library (`BuildContext` and `BuildPlate`), with `Default`, as the parameter snapshot is (decision D13-25): a context has fifteen optional values, a producer sets
//!   the ones it knows, and a value added later must not break a producer's build.
//! * **Requirement:** CT-UC-10 (tauri: the build metadata it writes is carried), V:UC-10 (Vision's build-information popup shows it), BL-CD-01.
//!
//! # What this module claims
//!
//! * **Every value is as the file gave it.** A text is as written (an empty text is a recorded empty text; the library interprets none of them). A quantity keeps its number, and says its
//!   unit only if the file does: the plate's three dimensions and its corner radius have a unit attribute (`mm`), and the starting height, the thickness, the weight and the older
//!   dimension have none in any file, so their unit is unknown and nothing is guessed (decision D02-18).
//! * **An absent value is absent.** Every value is an `Option`; a file with none of them has a context that [`BuildContext::is_empty`].
//! * **A placeholder is not a value.** A value that the adapter of a version knows to be what its writer writes when it does not know, and that is not a possible value of the field, is
//!   left absent, and the build's origins say `PlaceholderDropped` with its raw spelling, under the name of the value ([`BuildContext::origin_name`]). The rule is the layer thickness's
//!   (J4); which values are placeholders is the adapter's, by version (decision D02-21).
//! * **The plate has two vocabularies, and neither is converted into the other.** An older capture gives one dimension and a shape; tauri's current output gives three dimensions and a
//!   corner radius, each with its unit. [`BuildPlate`] holds both as named values, and the build's origin for `plate` says which one the file used ([`PLATE_AS_DIMENSION`] and
//!   [`PLATE_AS_AXES`]). The older dimension is never split into an X and a Y.
//!
//! The policy guards in `tests/policy_guards.rs` scan this file: every enum is `#[non_exhaustive]` or says `// closed: <reason>`, every public struct says
//! `// constructed-by: producer|library|caller-options` and follows that rule, and none holds bulk data.

use crate::error::{Error, ErrorKind, Violation};
use crate::params::ParameterValue;
use crate::units::Quantity;

/// What the build's origin for `plate` says when the file gives the plate as one dimension and a shape (the older vocabulary).
pub const PLATE_AS_DIMENSION: &str = "plate: one dimension and a shape";

/// What the build's origin for `plate` says when the file gives the plate as three dimensions and a corner radius (tauri's current vocabulary).
pub const PLATE_AS_AXES: &str = "plate: three dimensions and a corner radius";

/// The build plate, in the vocabularies of the files that carry it.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct BuildPlate {
    /// The older vocabulary: the shape of the plate, as written (`Square`).
    pub shape: Option<String>,
    /// The older vocabulary: the plate's one dimension. Its unit is unknown: no file states it.
    pub dimension: Option<Quantity>,
    /// The current vocabulary: the three dimensions and the corner radius, each with the unit its file states.
    pub x: Option<Quantity>,
    pub y: Option<Quantity>,
    pub z: Option<Quantity>,
    pub corner_radius: Option<Quantity>,
    /// Both vocabularies. Their unit is unknown: no file states it.
    pub thickness: Option<Quantity>,
    pub weight: Option<Quantity>,
}

/// What a build carries beyond its identity: the machine, the material, the starting height and the plate.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct BuildContext {
    pub machine_serial: Option<String>,
    pub scanner_id: Option<String>,
    pub material_parameter_id: Option<String>,
    pub recoater_blade: Option<String>,
    /// The direction of the gas flow, as written (`Y+`).
    pub gas_flow_direction: Option<String>,
    /// The direction of the recoater's travel, as written (`X+`).
    pub recoat_direction: Option<String>,
    /// The height the build starts at. Its unit is unknown: no file states it.
    pub starting_height: Option<Quantity>,
    pub plate: BuildPlate,
}

impl BuildContext {
    pub fn new() -> Self {
        BuildContext::default()
    }

    /// Whether no value is set: a file that carries none of them.
    pub fn is_empty(&self) -> bool {
        *self == BuildContext::default()
    }

    /// The name under which the build's origins say how a value came to be what it is (`context.plate.thickness`, and `context.plate` for the vocabulary). **The one place that spells it**, so
    /// that an adapter, the conformance crate and a writer say the same name.
    pub fn origin_name(name: &str) -> String {
        format!("context.{name}")
    }

    /// Whether this name is a quantity of the context (a number with a unit) and not a text.
    pub fn is_a_quantity(name: &str) -> bool {
        matches!(
            name,
            "starting_height"
                | "plate.dimension"
                | "plate.x"
                | "plate.y"
                | "plate.z"
                | "plate.corner_radius"
                | "plate.thickness"
                | "plate.weight"
        )
    }

    /// Every value that is set, by name, in name order. A value that is absent is not listed.
    pub fn values(&self) -> Vec<(String, ParameterValue)> {
        let mut found = Vec::new();
        let mut add = |name: &str, value: Option<ParameterValue>| {
            if let Some(value) = value {
                found.push((name.to_string(), value));
            }
        };
        let q = |v: &Option<Quantity>| v.clone().map(ParameterValue::Quantity);
        let t = |v: &Option<String>| v.clone().map(ParameterValue::Text);
        add("machine_serial", t(&self.machine_serial));
        add("scanner_id", t(&self.scanner_id));
        add("material_parameter_id", t(&self.material_parameter_id));
        add("recoater_blade", t(&self.recoater_blade));
        add("gas_flow_direction", t(&self.gas_flow_direction));
        add("recoat_direction", t(&self.recoat_direction));
        add("starting_height", q(&self.starting_height));
        let p = &self.plate;
        add("plate.shape", t(&p.shape));
        add("plate.dimension", q(&p.dimension));
        add("plate.x", q(&p.x));
        add("plate.y", q(&p.y));
        add("plate.z", q(&p.z));
        add("plate.corner_radius", q(&p.corner_radius));
        add("plate.thickness", q(&p.thickness));
        add("plate.weight", q(&p.weight));
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    /// Sets the value of this name, the inverse of [`BuildContext::values`]: the names are the same, and a value of the wrong kind for its name is refused. **This is the one place that
    /// maps a name to a field**, so an adapter says which attribute holds which name and nothing else.
    pub fn set(&mut self, name: &str, value: ParameterValue) -> Result<(), Error> {
        let refuse = |why: String| {
            Error::new(ErrorKind::ValidationError {
                violations: vec![Violation::new(why)],
            })
        };
        let wrong =
            |kind: &str| refuse(format!("`{name}` holds {kind}, and was given another kind"));
        let q = |v: ParameterValue| match v {
            ParameterValue::Quantity(q) => Ok(Some(q)),
            _ => Err(wrong("a quantity")),
        };
        let t = |v: ParameterValue| match v {
            ParameterValue::Text(text) => Ok(Some(text)),
            _ => Err(wrong("text")),
        };
        match name {
            "machine_serial" => self.machine_serial = t(value)?,
            "scanner_id" => self.scanner_id = t(value)?,
            "material_parameter_id" => self.material_parameter_id = t(value)?,
            "recoater_blade" => self.recoater_blade = t(value)?,
            "gas_flow_direction" => self.gas_flow_direction = t(value)?,
            "recoat_direction" => self.recoat_direction = t(value)?,
            "starting_height" => self.starting_height = q(value)?,
            "plate.shape" => self.plate.shape = t(value)?,
            "plate.dimension" => self.plate.dimension = q(value)?,
            "plate.x" => self.plate.x = q(value)?,
            "plate.y" => self.plate.y = q(value)?,
            "plate.z" => self.plate.z = q(value)?,
            "plate.corner_radius" => self.plate.corner_radius = q(value)?,
            "plate.thickness" => self.plate.thickness = q(value)?,
            "plate.weight" => self.plate.weight = q(value)?,
            other => return Err(refuse(format!("`{other}` is not a value of the context"))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::UnitSource;

    fn millimetres(value: f64) -> Quantity {
        Quantity::from_file(value, Some("mm"))
    }

    #[test]
    fn a_context_is_absent_until_a_file_or_a_producer_gives_it() {
        let context = BuildContext::new();
        assert!(context.is_empty());
        assert!(context.values().is_empty());
        let mut set = BuildContext::new();
        set.machine_serial = Some(String::new());
        // an empty text is a recorded empty text: the context is not empty
        assert!(!set.is_empty());
        assert_eq!(
            set.values(),
            [(
                "machine_serial".to_string(),
                ParameterValue::Text(String::new())
            )]
        );
    }

    #[test]
    fn values_lists_each_value_that_is_set_by_name_in_name_order() {
        let mut context = BuildContext::new();
        context.scanner_id = Some("a scanner".into());
        context.starting_height = Some(Quantity::from_file(0.0, None));
        context.plate.x = Some(millimetres(250.0));
        context.plate.shape = Some("Square".into());
        let names: Vec<String> = context.values().into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            ["plate.shape", "plate.x", "scanner_id", "starting_height"]
        );
    }

    #[test]
    fn set_is_the_inverse_of_values_for_every_name() {
        // each of the fifteen names is set, and what `values` lists is what was given
        let mut context = BuildContext::new();
        let texts = [
            "machine_serial",
            "scanner_id",
            "material_parameter_id",
            "recoater_blade",
            "gas_flow_direction",
            "recoat_direction",
            "plate.shape",
        ];
        let quantities = [
            "starting_height",
            "plate.dimension",
            "plate.x",
            "plate.y",
            "plate.z",
            "plate.corner_radius",
            "plate.thickness",
            "plate.weight",
        ];
        let mut given = Vec::new();
        for (n, name) in texts.iter().enumerate() {
            let value = ParameterValue::Text(format!("text {n}"));
            context.set(name, value.clone()).expect("a text name");
            given.push((name.to_string(), value));
        }
        for (n, name) in quantities.iter().enumerate() {
            let value = ParameterValue::Quantity(Quantity::from_file(n as f64, None));
            context.set(name, value.clone()).expect("a quantity name");
            given.push((name.to_string(), value));
        }
        given.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(given.len(), 15);
        assert_eq!(context.values(), given);
    }

    #[test]
    fn set_refuses_the_wrong_kind_and_a_name_that_is_not_a_value_of_the_context() {
        let mut context = BuildContext::new();
        assert!(context
            .set("machine_serial", ParameterValue::Quantity(millimetres(1.0)))
            .is_err());
        assert!(context
            .set("plate.x", ParameterValue::Text("wide".into()))
            .is_err());
        assert!(context
            .set("plate.depth", ParameterValue::Text("deep".into()))
            .is_err());
        // a refused value changes nothing
        assert!(context.is_empty());
    }

    #[test]
    fn the_plate_holds_both_vocabularies_and_converts_neither() {
        // the older vocabulary
        let older = BuildPlate {
            shape: Some("Square".into()),
            dimension: Some(Quantity::from_file(280.0, None)),
            ..BuildPlate::default()
        };
        assert!(older.x.is_none() && older.y.is_none() && older.corner_radius.is_none());
        assert_eq!(
            older.dimension.as_ref().map(|d| d.unit_source),
            Some(UnitSource::Unknown)
        );
        // the current vocabulary
        let current = BuildPlate {
            x: Some(millimetres(250.0)),
            corner_radius: Some(millimetres(125.0)),
            ..BuildPlate::default()
        };
        assert!(current.dimension.is_none() && current.shape.is_none());
        assert_eq!(
            current.x.as_ref().map(|d| d.unit_source),
            Some(UnitSource::FromFile)
        );
        // the two sentences that say which one a file used are not the same
        assert_ne!(PLATE_AS_DIMENSION, PLATE_AS_AXES);
    }

    #[test]
    fn the_origin_of_a_value_is_named_by_the_context_and_the_value() {
        assert_eq!(
            BuildContext::origin_name("plate.thickness"),
            "context.plate.thickness"
        );
        assert_eq!(BuildContext::origin_name("plate"), "context.plate");
    }

    #[test]
    fn a_context_compares_equal_after_a_clone_and_is_send_and_sync() {
        let mut context = BuildContext::new();
        context.recoat_direction = Some("X+".into());
        context.plate.weight = Some(Quantity::from_file(10.0, None));
        assert_eq!(context.clone(), context);
        fn is_send_and_sync<T: Send + Sync>() {}
        is_send_and_sync::<BuildContext>();
        is_send_and_sync::<BuildPlate>();
    }
}
