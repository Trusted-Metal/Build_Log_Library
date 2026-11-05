//! The correction grids of a laser, held to an expectation (IP-24 phase 24B, task T24.2; decision D24-11): the descriptors in the metadata, the values that a source returns for them, and the
//! snapshot that an expectation describes.
//!
//! An expectation lists, for each laser that has a snapshot, its `grids`: each by **the name the file spells**, which grid the name says it is (`Forward`, `Inverse` or none), the stored type, the extents in
//! the order the file stores them, that the domain and the axis order are `Unrecorded`, and a **fingerprint of the values in that order**: the count, the stored type, **how many are not a number**, the
//! minimum and the maximum of those that are, and the SHA-256 of the exact bytes stored, little-endian (a NaN is part of the hash, bit for bit). The independent script writes it from the miniature alone.
//!
//! * [`check_descriptors`] holds the snapshot's list of grids to the expectation's, in order, field by field (the check of the parameters part calls it).
//! * [`check_grids`] reads each described grid from a [`ColumnSource`] in full, as stored, and says every way in which what it got is not what the expectation says. **A V16 or V17 laser with a snapshot
//!   must list its grids** (an empty list is a list), so that a check cannot pass by having nothing to check.
//! * [`synthesise`] builds the descriptors from an expectation, for a build made from it.
//!
//! The fingerprint is computed here from a [`GridData`] in the same way the script computes it from the file; a test holds both to the same literals.

use build_log_model::{
    ColumnData, ColumnSource, CorrectionGrid, Dims, GridData, GridRequest, GridRole, LaserKey,
    LaserMeta, LaserParameters, ReadContext,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::check::same;
use crate::columns::{hex, Stored, HASH_BLOCK};
use crate::expectation::{array, field, float, int, opt_text, text};
use crate::names::numeric_from;
use crate::Expectation;

/// What a stored grid is, in a few numbers and one hash.
#[derive(Debug, Clone, PartialEq)]
pub struct GridFingerprint {
    pub count: u64,
    /// The stored type as a channel names it: `F64`, `I32` and so on.
    pub stored_as: String,
    /// How many of the values are not a number (a NaN): the V17 grids are mostly NaN.
    pub not_a_number: u64,
    /// The minimum and the maximum of the values that are numbers, or `None` if there are none.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The SHA-256 of the values' little-endian bytes, in the order stored, in lower-case hexadecimal.
    pub sha256: String,
}

fn describe<T: Stored>(values: &[T]) -> GridFingerprint {
    let numbers = || values.iter().map(|v| v.as_f64()).filter(|v| !v.is_nan());
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity(HASH_BLOCK * std::mem::size_of::<T>());
    for block in values.chunks(HASH_BLOCK) {
        bytes.clear();
        for value in block {
            value.extend_le(&mut bytes);
        }
        hasher.update(&bytes);
    }
    GridFingerprint {
        count: values.len() as u64,
        stored_as: T::NAME.to_string(),
        not_a_number: values.iter().filter(|v| v.as_f64().is_nan()).count() as u64,
        min: numbers().min_by(f64::total_cmp),
        max: numbers().max_by(f64::total_cmp),
        sha256: hex(&hasher.finalize()),
    }
}

impl GridFingerprint {
    /// The fingerprint of the values of a grid that was read, or `None` if they are not numbers.
    pub fn of(data: &GridData) -> Option<GridFingerprint> {
        Some(match &data.values {
            ColumnData::I8(v) => describe(v.as_slice()),
            ColumnData::I16(v) => describe(v.as_slice()),
            ColumnData::I32(v) => describe(v.as_slice()),
            ColumnData::I64(v) => describe(v.as_slice()),
            ColumnData::U8(v) => describe(v.as_slice()),
            ColumnData::U16(v) => describe(v.as_slice()),
            ColumnData::U32(v) => describe(v.as_slice()),
            ColumnData::U64(v) => describe(v.as_slice()),
            ColumnData::F32(v) => describe(v.as_slice()),
            ColumnData::F64(v) => describe(v.as_slice()),
            _ => return None,
        })
    }

    fn from_json(value: &Value) -> Result<GridFingerprint, String> {
        let opt = |key: &str| -> Result<Option<f64>, String> {
            match value.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(_) => float(value, key).map(Some),
            }
        };
        let count =
            |key: &str| u64::try_from(int(value, key)?).map_err(|_| format!("`{key}` is negative"));
        Ok(GridFingerprint {
            count: count("count")?,
            stored_as: text(value, "stored_as")?.to_string(),
            not_a_number: count("not_a_number")?,
            min: opt("min")?,
            max: opt("max")?,
            sha256: opt_text(value, "sha256")?
                .ok_or_else(|| "the fingerprint has no `sha256`".to_string())?,
        })
    }
}

fn role_text(role: Option<GridRole>) -> Option<String> {
    role.map(|role| format!("{role:?}"))
}

fn extents_of(entry: &Value) -> Result<Vec<u64>, String> {
    array(entry, "extents")?
        .iter()
        .map(|v| {
            v.as_u64()
                .ok_or_else(|| "an extent is not a whole number".to_string())
        })
        .collect()
}

/// Every disagreement between a snapshot's list of grids and the one an expectation lists for the laser, if it lists any.
pub(crate) fn check_descriptors(
    laser: &LaserMeta,
    actual: &LaserParameters,
    expected: &Value,
    found: &mut Vec<String>,
) -> Result<(), String> {
    let Some(list) = expected.get("grids").filter(|v| !v.is_null()) else {
        return Ok(());
    };
    let wanted = list
        .as_array()
        .ok_or_else(|| "`grids` is not a list".to_string())?;
    let what = |part: &str| {
        format!(
            "laser {}, its parameters, its correction grids, {part}",
            laser.key
        )
    };
    same(
        found,
        &what("how many there are"),
        actual.grids.len(),
        wanted.len(),
    );
    for (at, entry) in wanted.iter().enumerate() {
        let name = text(entry, "name")?;
        let Some(grid) = actual.grids.get(at) else {
            found.push(format!("{}: not there, expected {entry}", what(name)));
            continue;
        };
        let of = |field_name: &str| what(&format!("{name}, {field_name}"));
        same(found, &of("its name"), grid.name.as_str(), name);
        same(
            found,
            &of("which grid its name says it is"),
            role_text(grid.role),
            opt_text(entry, "role")?,
        );
        same(
            found,
            &of("the type it is stored as"),
            format!("{:?}", grid.stored_as),
            text(entry, "stored_as")?.to_string(),
        );
        same(
            found,
            &of("its extents"),
            grid.extents.as_slice().to_vec(),
            extents_of(entry)?,
        );
        same(
            found,
            &of("what is recorded of its meaning"),
            format!("{:?}", grid.interpretation),
            text(entry, "interpretation")?.to_string(),
        );
    }
    Ok(())
}

/// The descriptors that an expectation lists for a laser's snapshot, for a build made from it. A laser whose expectation lists none has none.
pub(crate) fn synthesise(expected: &Value) -> Result<Vec<CorrectionGrid>, String> {
    let Some(list) = expected.get("grids").filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let mut grids = Vec::new();
    for entry in list
        .as_array()
        .ok_or_else(|| "`grids` is not a list".to_string())?
    {
        let role = match opt_text(entry, "role")?.as_deref() {
            None => None,
            Some("Forward") => Some(GridRole::Forward),
            Some("Inverse") => Some(GridRole::Inverse),
            Some(other) => {
                return Err(format!("`{other}` is not a role that an expectation names"))
            }
        };
        let extents = Dims::new(&extents_of(entry)?).map_err(|e| e.to_string())?;
        grids.push(CorrectionGrid::new(
            text(entry, "name")?,
            role,
            numeric_from(text(entry, "stored_as")?)?,
            extents,
        ));
    }
    Ok(grids)
}

/// One grid that an expectation describes: whose it is, its name, its extents and its fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub struct GridExpectation {
    pub laser: LaserKey,
    pub name: String,
    pub extents: Vec<u64>,
    pub fingerprint: GridFingerprint,
}

/// Every grid that an expectation describes. **A V16 or V17 expectation must list the grids of each laser that has a snapshot** (an empty list is a list): one that does not is an error, so that a
/// check cannot pass by having nothing to check. An expectation of V18 describes none yet.
pub fn grid_expectations(expectation: &Expectation) -> Result<Vec<GridExpectation>, String> {
    let root = expectation.root();
    let version = int(root, "file_version")?;
    let mut found = Vec::new();
    for laser in array(root, "lasers")? {
        let key = LaserKey::new(text(laser, "key")?, int(laser, "number")? as u32);
        let Some(parameters) = laser.get("parameters").filter(|v| !v.is_null()) else {
            continue;
        };
        match parameters.get("grids").filter(|v| !v.is_null()) {
            None if matches!(version, 16 | 17) => {
                return Err(format!(
                    "{}: the expectation lists no grids for a laser that has a snapshot",
                    key.key
                ))
            }
            None => {}
            Some(_) => {
                for entry in array(parameters, "grids")? {
                    let name = text(entry, "name")?.to_string();
                    found.push(GridExpectation {
                        laser: key.clone(),
                        extents: extents_of(entry)?,
                        fingerprint: GridFingerprint::from_json(field(entry, "fingerprint")?)
                            .map_err(|e| format!("{}, {name}: {e}", key.key))?,
                        name,
                    });
                }
            }
        }
    }
    Ok(found)
}

/// Every way in which what `source` serves is not what the grids of `expectation` say, as lines that name the laser and the grid. Each described grid is read in full, as stored, with a context that
/// never cancels; one that cannot be read is reported with the error.
pub fn check_grids(source: &dyn ColumnSource, expectation: &Expectation) -> Vec<String> {
    let grids = match grid_expectations(expectation) {
        Ok(grids) => grids,
        Err(problem) => return vec![problem],
    };
    let context = ReadContext::new();
    let mut found = Vec::new();
    for wanted in &grids {
        let who = format!("{}, {}", wanted.laser.key, wanted.name);
        let request = GridRequest::new(wanted.laser.clone(), wanted.name.clone());
        let data = match source.read_grid(&request, &context) {
            Ok(data) => data,
            Err(error) => {
                found.push(format!("{who}: could not be read: {error}"));
                continue;
            }
        };
        same(
            &mut found,
            &format!("{who}, its extents"),
            data.extents.as_slice().to_vec(),
            wanted.extents.clone(),
        );
        let Some(actual) = GridFingerprint::of(&data) else {
            found.push(format!("{who}: the values are not numbers"));
            continue;
        };
        let a = &actual;
        let w = &wanted.fingerprint;
        same(
            &mut found,
            &format!("{who}, the stored type"),
            a.stored_as.clone(),
            w.stored_as.clone(),
        );
        same(
            &mut found,
            &format!("{who}, the count of values"),
            a.count,
            w.count,
        );
        same(
            &mut found,
            &format!("{who}, how many are not a number"),
            a.not_a_number,
            w.not_a_number,
        );
        same(&mut found, &format!("{who}, the minimum"), a.min, w.min);
        same(&mut found, &format!("{who}, the maximum"), a.max, w.max);
        same(
            &mut found,
            &format!("{who}, the SHA-256 of the values"),
            a.sha256.chars().take(16).collect::<String>() + "...",
            w.sha256.chars().take(16).collect::<String>() + "...",
        );
    }
    found
}
