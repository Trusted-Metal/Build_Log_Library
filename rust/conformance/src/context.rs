//! The build's context, checked against an expectation and built from one (IP-02 phase 2V; decisions D02-20 and D02-21).
//!
//! An expectation lists each value of the context by the CDM's name and kind (a quantity or a text, as a parameter is listed), **the values that are placeholders of the version and are
//! not recorded, with the raw spelling the file wrote**, and which plate vocabulary the file uses, with **the sentence that the build's origin for the plate must say**: the independent
//! script's own words, so that the library's constant is held to them. A file with none of the values has an empty context and no origin about it.

use std::collections::BTreeMap;

use build_log_model::{BuildContext, BuildMeta, Origin, ParameterValue};
use serde_json::Value;

use crate::check::same;
use crate::expectation::{array, opt_text, text};
use crate::parameters::{value_against, value_from};

/// Every disagreement between the build's context (and what the build's origins say of it) and the context an expectation lists.
pub(crate) fn check(
    meta: &BuildMeta,
    expected: Option<&Value>,
    found: &mut Vec<String>,
) -> Result<(), String> {
    let what = |part: &str| format!("the context, {part}");
    // what the build's origins say of the context, by the name of the value that the origin is about
    let prefix = BuildContext::origin_name("");
    let mut origins: BTreeMap<String, Origin> = meta
        .origins
        .iter()
        .filter(|(field, _)| field.starts_with(&prefix))
        .map(|(field, origin)| (field.to_string(), origin.clone()))
        .collect();
    let Some(expected) = expected.filter(|v| !v.is_null()) else {
        same(
            found,
            &what("whether it has any value"),
            meta.context.is_empty(),
            true,
        );
        for (name, origin) in origins {
            found.push(format!(
                "{}: found {origin:?}, expected nothing",
                what(&name)
            ));
        }
        return Ok(());
    };

    // the values, by name, as written
    let mut have: BTreeMap<String, ParameterValue> = meta.context.values().into_iter().collect();
    for entry in array(expected, "values")? {
        let name = text(entry, "name")?;
        let Some(value) = have.remove(name) else {
            found.push(format!("{}: not there, expected {entry}", what(name)));
            continue;
        };
        value_against(&what(name), &value, entry, found)?;
    }
    // a value that is set and is not listed is found where nothing was expected: a placeholder that was recorded is one of them
    for name in have.keys() {
        found.push(format!("{}: found, expected nothing", what(name)));
    }

    // the placeholders are not recorded, and the origin of each says what the file wrote
    for entry in array(expected, "dropped")? {
        let name = text(entry, "name")?;
        let raw = Origin::PlaceholderDropped {
            raw: text(entry, "raw")?.to_string(),
        };
        same(
            found,
            &what(&format!("{name}, its origin")),
            origins.remove(&BuildContext::origin_name(name)),
            Some(raw),
        );
    }

    // which vocabulary the plate is given in, as the sentence of the origin
    same(
        found,
        &what("the vocabulary of the plate, as its origin says it"),
        origins.remove(&BuildContext::origin_name("plate")),
        opt_text(expected, "plate_origin")?.map(Origin::Other),
    );

    // any other origin about the context says something that was not expected
    for (name, origin) in origins {
        found.push(format!(
            "{}: found {origin:?}, expected nothing",
            what(&name)
        ));
    }
    Ok(())
}

/// Makes the context an expectation describes, in the build, with the public calls an adapter uses.
pub(crate) fn synthesise(expected: Option<&Value>, meta: &mut BuildMeta) -> Result<(), String> {
    let Some(expected) = expected.filter(|v| !v.is_null()) else {
        return Ok(());
    };
    for entry in array(expected, "values")? {
        meta.context
            .set(text(entry, "name")?, value_from(entry)?)
            .map_err(|e| e.to_string())?;
    }
    for entry in array(expected, "dropped")? {
        meta.origins.record(
            BuildContext::origin_name(text(entry, "name")?),
            Origin::PlaceholderDropped {
                raw: text(entry, "raw")?.to_string(),
            },
        );
    }
    if let Some(sentence) = opt_text(expected, "plate_origin")? {
        meta.origins
            .record(BuildContext::origin_name("plate"), Origin::Other(sentence));
    }
    Ok(())
}
