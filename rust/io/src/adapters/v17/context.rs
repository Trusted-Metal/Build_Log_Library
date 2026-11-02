//! The build's context in a V17 file (IP-02 phase 2V; decisions D02-20 and D02-21): the machine, the material, the starting height and the build plate, read from the attributes of the
//! mount into [`BuildMeta::context`].
//!
//! * **One table says which attribute holds which value** ([`layout::CONTEXT`]); the Model maps a name to a field (`BuildContext::set`), so this file names no attribute and no field.
//! * **Every value is as the file gave it.** A text is as written (an empty text is a recorded empty text); a number is a number whether the file stored it as a float or an integer; a
//!   quantity has the unit of its companion attribute where the file has one, and an unknown unit otherwise.
//! * **A placeholder is not a value** ([`layout::CONTEXT_PLACEHOLDERS`]): the Model's rule of the layer thickness makes the field absent and the build's origin for it says `PlaceholderDropped`
//!   with the raw spelling.
//! * **The plate keeps the vocabulary of its file**: one dimension and a shape, or three dimensions and a corner radius, and the build's origin for `plate` says which. Neither is converted into
//!   the other.
//! * **An attribute that cannot be read is reported on itself and the build opens** (the rule of decision D13-21): the value is absent, the build's origin for it is `NotRead` with the cause
//!   (the validation report makes a fault of it), and **the attribute stays in `extra`**, so nothing a file holds is dropped.

use build_log_model::{
    BuildContext, BuildMeta, Error, Origin, ParameterValue, PLATE_AS_AXES, PLATE_AS_DIMENSION,
};
use hdf5::Group;

use super::layout::{self, ContextKind};
use super::values::{optional_number, optional_text};

/// The values of the older vocabulary of the plate, and of the current one, by the CDM's names.
const OLDER_VOCABULARY: [&str; 2] = ["plate.dimension", "plate.shape"];
const CURRENT_VOCABULARY: [&str; 4] = ["plate.x", "plate.y", "plate.z", "plate.corner_radius"];

/// Reads the context into `build`. Returns the attributes that were **typed** (each value's, and its unit's where it had one), which the build level does not keep in `extra`; an
/// attribute that was absent or that could not be read is not among them.
pub fn read(mount: &Group, build: &mut BuildMeta) -> Vec<&'static str> {
    let mut typed = Vec::new();
    let (mut older, mut current) = (false, false);
    for (attribute, name, kind, unit_attribute) in layout::CONTEXT {
        let outcome = read_one(mount, build, attribute, name, kind, unit_attribute);
        match outcome {
            Ok(false) => continue,
            Ok(true) => {
                typed.push(attribute);
                typed.extend(unit_attribute);
            }
            Err(error) => {
                build
                    .origins
                    .record(BuildContext::origin_name(name), Origin::not_read(&error));
            }
        }
        // the attribute is in the file, whether or not it could be read: the file used that vocabulary
        older |= OLDER_VOCABULARY.contains(&name);
        current |= CURRENT_VOCABULARY.contains(&name);
    }
    // which vocabulary the plate is given in: a file that gives both says both
    let sentence = match (older, current) {
        (false, false) => None,
        (true, false) => Some(PLATE_AS_DIMENSION.to_string()),
        (false, true) => Some(PLATE_AS_AXES.to_string()),
        (true, true) => Some(format!("{PLATE_AS_DIMENSION}; {PLATE_AS_AXES}")),
    };
    if let Some(sentence) = sentence {
        build
            .origins
            .record(BuildContext::origin_name("plate"), Origin::Other(sentence));
    }
    typed
}

/// One attribute of the context: whether the file has it (and it was read), or an error that says why it could not be.
fn read_one(
    mount: &Group,
    build: &mut BuildMeta,
    attribute: &str,
    name: &str,
    kind: ContextKind,
    unit_attribute: Option<&str>,
) -> Result<bool, Error> {
    match kind {
        ContextKind::Text => match optional_text(mount, attribute)? {
            None => Ok(false),
            Some(text) => {
                build.context.set(name, ParameterValue::Text(text))?;
                Ok(true)
            }
        },
        ContextKind::Quantity => {
            let Some(value) = optional_number(mount, attribute)? else {
                return Ok(false);
            };
            let unit = match unit_attribute {
                Some(unit_attribute) => optional_text(mount, unit_attribute)?,
                None => None,
            };
            let placeholders: Vec<f64> = layout::CONTEXT_PLACEHOLDERS
                .iter()
                .filter(|(placeholder_of, _)| *placeholder_of == name)
                .map(|(_, value)| *value)
                .collect();
            build.set_context_quantity_from_file(name, value, unit.as_deref(), &placeholders)?;
            Ok(true)
        }
    }
}
