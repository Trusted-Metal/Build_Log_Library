//! The parameter snapshot of a laser, checked against an expectation and built from one (IP-13 phase 13H; decisions D13-18 and D13-25).
//!
//! An expectation lists, for a laser, the optical train it comes from, each value by the CDM's name and kind, the volts-to-watts equation as named constants, and
//! whether the power limits are not recorded as numbers because the file stores them as text (decision D13-16). A laser the file has no calibration for has none.

use std::collections::BTreeMap;

use build_log_model::{
    Equation, LaserMeta, LaserParameters, NamedConstant, Origin, ParameterValue, Quantity,
};
use serde_json::Value;

use crate::check::{same, unit};
use crate::expectation::{array, field, float, int, opt_bool, opt_text, text};
use crate::names::{equation_kind_from, protocol_from};

/// The name under which a snapshot's origins say that a laser's power limits are not recorded as numbers. The V17 adapter records the same name.
const POWER_LIMITS: &str = "light_source.power_limits";

/// What a snapshot says when the limits are text, and are not recorded as numbers.
const POWER_LIMITS_WHY: &str = "not recorded: the file stores text where a number belongs";

/// Every disagreement between a laser's snapshot and the one an expectation lists for it.
pub(crate) fn check_laser(
    laser: &LaserMeta,
    expected: Option<&Value>,
    found: &mut Vec<String>,
) -> Result<(), String> {
    let what = |part: &str| format!("laser {}, its parameters, {part}", laser.key);
    let Some(expected) = expected.filter(|v| !v.is_null()) else {
        same(
            found,
            &what("whether it has any"),
            laser.parameters.is_some(),
            false,
        );
        return Ok(());
    };
    let Some(actual) = &laser.parameters else {
        found.push(format!(
            "laser {}, its parameters: not there, expected a snapshot",
            laser.key
        ));
        return Ok(());
    };
    same(
        found,
        &what("the optical train"),
        actual.optical_train_number,
        Some(int(expected, "optical_train")? as u32),
    );

    let mut have: BTreeMap<String, ParameterValue> = actual.values().into_iter().collect();
    for entry in array(expected, "values")? {
        let name = text(entry, "name")?;
        let Some(value) = have.remove(name) else {
            found.push(format!("{}: not there, expected {entry}", what(name)));
            continue;
        };
        value_against(&what(name), &value, entry, found)?;
    }
    for name in have.keys() {
        found.push(format!("{}: found, expected nothing", what(name)));
    }

    let wanted = expected.get("volts_to_watts").filter(|v| !v.is_null());
    match (&actual.light_source.volts_to_watts, wanted) {
        (None, None) => {}
        (Some(_), None) => found.push(format!(
            "{}: found one, expected none",
            what("the volts-to-watts equation")
        )),
        (None, Some(_)) => found.push(format!(
            "{}: not there, expected one",
            what("the volts-to-watts equation")
        )),
        (Some(equation), Some(wanted)) => {
            same(
                found,
                &what("the kind of the volts-to-watts equation"),
                Some(equation.kind.clone()),
                Some(equation_kind_from(
                    text(wanted, "kind")?,
                    opt_text(wanted, "other")?,
                )?),
            );
            same(
                found,
                &what("the constants of the volts-to-watts equation, by name"),
                constants_of(equation),
                expected_constants(wanted)?,
            );
        }
    }

    same(
        found,
        &what("whether the power limits are not recorded as numbers"),
        matches!(actual.origins.get(POWER_LIMITS), Some(Origin::Other(_))),
        opt_bool(expected, "power_limits_not_recorded")?.unwrap_or(false),
    );
    Ok(())
}

/// One value of an expectation (`{name, kind, value, unit}`) against the value that was found. The parameters and the build's context list their values the same way.
pub(crate) fn value_against(
    what: &str,
    actual: &ParameterValue,
    expected: &Value,
    found: &mut Vec<String>,
) -> Result<(), String> {
    let kind = text(expected, "kind")?;
    match (kind, actual) {
        ("quantity", ParameterValue::Quantity(q)) => {
            same(found, what, q.value, float(expected, "value")?);
            unit(
                found,
                &format!("{what}, its unit"),
                &q.unit,
                q.unit_source,
                field(expected, "unit")?,
            )?;
        }
        ("plain", ParameterValue::Plain(n)) => same(found, what, *n, float(expected, "value")?),
        ("whole", ParameterValue::Whole(n)) => same(found, what, *n, int(expected, "value")?),
        ("flag", ParameterValue::Flag(b)) => {
            same(found, what, Some(*b), opt_bool(expected, "value")?)
        }
        ("text", ParameterValue::Text(t)) => {
            same(found, what, t.as_str(), text(expected, "value")?)
        }
        ("protocol", ParameterValue::Protocol(p)) => same(
            found,
            what,
            p.clone(),
            protocol_from(opt_text(expected, "named")?, opt_text(expected, "other")?)?,
        ),
        (kind, other) => found.push(format!(
            "{what}: found {other:?}, expected a value of the kind `{kind}`"
        )),
    }
    Ok(())
}

/// The constants of an equation as name and value, in name order: what an equation says does not depend on the order of its list.
fn constants_of(equation: &Equation) -> Vec<(String, f64)> {
    let mut list: Vec<(String, f64)> = equation
        .constants
        .iter()
        .map(|c| (c.name.clone(), c.value))
        .collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list
}

fn expected_constants(equation: &Value) -> Result<Vec<(String, f64)>, String> {
    let mut list = Vec::new();
    for constant in array(equation, "constants")? {
        list.push((
            text(constant, "name")?.to_string(),
            float(constant, "value")?,
        ));
    }
    list.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(list)
}

/// The value that one entry of an expectation (`{name, kind, value, unit}`) describes, as the public calls an adapter uses make it.
pub(crate) fn value_from(entry: &Value) -> Result<ParameterValue, String> {
    Ok(match text(entry, "kind")? {
        "quantity" => {
            let spelling = opt_text(field(entry, "unit")?, "spelling")?;
            ParameterValue::Quantity(Quantity::from_file(
                float(entry, "value")?,
                spelling.as_deref(),
            ))
        }
        "plain" => ParameterValue::Plain(float(entry, "value")?),
        "whole" => ParameterValue::Whole(int(entry, "value")?),
        "flag" => ParameterValue::Flag(
            opt_bool(entry, "value")?.ok_or_else(|| "a flag has no value".to_string())?,
        ),
        "text" => ParameterValue::Text(text(entry, "value")?.to_string()),
        "protocol" => ParameterValue::Protocol(protocol_from(
            opt_text(entry, "named")?,
            opt_text(entry, "other")?,
        )?),
        other => return Err(format!("`{other}` is not a kind of parameter")),
    })
}

/// The snapshot an expectation describes, built with the public calls an adapter uses.
pub(crate) fn synthesise(expected: Option<&Value>) -> Result<Option<LaserParameters>, String> {
    let Some(expected) = expected.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let mut parameters = LaserParameters::new();
    parameters.optical_train_number = Some(int(expected, "optical_train")? as u32);
    for entry in array(expected, "values")? {
        let value = value_from(entry)?;
        parameters
            .set(text(entry, "name")?, value)
            .map_err(|e| e.to_string())?;
    }
    if let Some(wanted) = expected.get("volts_to_watts").filter(|v| !v.is_null()) {
        let mut equation = Equation::new(equation_kind_from(
            text(wanted, "kind")?,
            opt_text(wanted, "other")?,
        )?);
        for constant in array(wanted, "constants")? {
            equation.constants.push(NamedConstant::new(
                text(constant, "name")?,
                float(constant, "value")?,
            ));
        }
        parameters.light_source.volts_to_watts = Some(equation);
    }
    if opt_bool(expected, "power_limits_not_recorded")?.unwrap_or(false) {
        parameters
            .origins
            .record(POWER_LIMITS, Origin::Other(POWER_LIMITS_WHY.to_string()));
    }
    Ok(Some(parameters))
}
