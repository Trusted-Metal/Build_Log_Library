//! The fingerprint of a stored column, and the check of a column source against the fingerprints of an expectation (IP-06 phase 6A, task T06.1, decision D06-16).
//!
//! A V17 expectation file carries (and, since IP-12 phase 12F, a V16 one), for each laser channel recorded with samples in a layer, a **fingerprint** of the stored column, written by the independent script
//! from the miniature alone: the count, the stored type, the minimum and the maximum, the first and the last three values, and the SHA-256 of the values'
//! little-endian bytes. [`check_columns`] reads each of those columns from a [`ColumnSource`] in full, as stored, and says every way in which what it got is not what
//! the fingerprint says. The expectation holds no sample, so the check is cheap to keep and the samples stay bulk data.
//!
//! * The fingerprint is computed here from a [`ColumnData`] **in the same way the script computes it from the file**: the values in order, each as its own type's
//!   little-endian bytes, whatever machine this runs on. A test holds both to the same literals.
//! * A minimum, a maximum or a value of a 64-bit integer column is compared as an `f64`, as the expectation file holds numbers; the hash is the exact check.
//! * A column that is not a column of numbers (text, flags, segment rows) has no fingerprint; a laser channel never is one.

use build_log_model::{
    ChannelKey, ColumnData, ColumnRequest, ColumnSource, LaserKey, LayerId, ReadContext,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::expectation::{array, float, int, opt_text, text};
use crate::Expectation;

/// How many values are hashed at a time.
pub(crate) const HASH_BLOCK: usize = 8192;

/// How many values of each end of a column a fingerprint shows.
const ENDS: usize = 3;

/// What a stored column is, in a few numbers and one hash.
#[derive(Debug, Clone, PartialEq)]
pub struct Fingerprint {
    pub count: u64,
    /// The stored type as a channel names it: `F32`, `I32`, `U8` and so on.
    pub stored_as: String,
    /// `None` for a column of no value, and for one that holds a value that is not a number.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Up to the first three values, and up to the last three.
    pub first: Vec<f64>,
    pub last: Vec<f64>,
    /// The SHA-256 of the values' little-endian bytes, in lower-case hexadecimal.
    pub sha256: String,
}

/// A number that a column holds.
pub(crate) trait Stored: Copy {
    const NAME: &'static str;
    fn as_f64(self) -> f64;
    fn extend_le(self, out: &mut Vec<u8>);
}

macro_rules! stored {
    ($t:ty, $name:literal, $v:ident => $to_f64:expr) => {
        impl Stored for $t {
            const NAME: &'static str = $name;
            fn as_f64(self) -> f64 {
                let $v = self;
                $to_f64
            }
            fn extend_le(self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
        }
    };
}

stored!(i8, "I8", v => f64::from(v));
stored!(i16, "I16", v => f64::from(v));
stored!(i32, "I32", v => f64::from(v));
stored!(i64, "I64", v => v as f64);
stored!(u8, "U8", v => f64::from(v));
stored!(u16, "U16", v => f64::from(v));
stored!(u32, "U32", v => f64::from(v));
stored!(u64, "U64", v => v as f64);
stored!(f32, "F32", v => f64::from(v));
stored!(f64, "F64", v => v);

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn describe<T: Stored>(values: &[T]) -> Fingerprint {
    let numbers = || values.iter().map(|v| v.as_f64());
    let comparable = numbers().all(|v| !v.is_nan());
    let (min, max) = if values.is_empty() || !comparable {
        (None, None)
    } else {
        (
            numbers().min_by(f64::total_cmp),
            numbers().max_by(f64::total_cmp),
        )
    };
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity(HASH_BLOCK * std::mem::size_of::<T>());
    for block in values.chunks(HASH_BLOCK) {
        bytes.clear();
        for value in block {
            value.extend_le(&mut bytes);
        }
        hasher.update(&bytes);
    }
    Fingerprint {
        count: values.len() as u64,
        stored_as: T::NAME.to_string(),
        min,
        max,
        first: values.iter().take(ENDS).map(|v| v.as_f64()).collect(),
        last: values[values.len().saturating_sub(ENDS)..]
            .iter()
            .map(|v| v.as_f64())
            .collect(),
        sha256: hex(&hasher.finalize()),
    }
}

impl Fingerprint {
    /// The fingerprint of a column that was read, or `None` if it is not a column of numbers.
    pub fn of(data: &ColumnData) -> Option<Fingerprint> {
        Some(match data {
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

    /// The fingerprint as an expectation file holds it, for a test that builds an expectation of its own.
    pub fn to_json(&self) -> Value {
        json!({
            "count": self.count,
            "stored_as": self.stored_as,
            "min": self.min,
            "max": self.max,
            "first": self.first,
            "last": self.last,
            "sha256": self.sha256,
        })
    }

    pub(crate) fn from_json(value: &Value) -> Result<Fingerprint, String> {
        let numbers = |key: &str| -> Result<Vec<f64>, String> {
            array(value, key)?
                .iter()
                .map(|v| {
                    v.as_f64()
                        .ok_or_else(|| format!("`{key}` holds a value that is not a number"))
                })
                .collect()
        };
        let opt = |key: &str| -> Result<Option<f64>, String> {
            match value.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(_) => float(value, key).map(Some),
            }
        };
        Ok(Fingerprint {
            count: u64::try_from(int(value, "count")?)
                .map_err(|_| "`count` is negative".to_string())?,
            stored_as: text(value, "stored_as")?.to_string(),
            min: opt("min")?,
            max: opt("max")?,
            first: numbers("first")?,
            last: numbers("last")?,
            sha256: opt_text(value, "sha256")?
                .ok_or_else(|| "the fingerprint has no `sha256`".to_string())?,
        })
    }
}

/// One stored column that an expectation describes: where it is, and its fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnExpectation {
    pub layer: LayerId,
    pub laser: LaserKey,
    pub channel: ChannelKey,
    pub fingerprint: Fingerprint,
}

impl ColumnExpectation {
    /// Where the column is, for a message: `layer 2, Laser_1, Actual_X`.
    fn whose(&self) -> String {
        format!(
            "layer {}, {}, {}",
            self.layer.0,
            self.laser.key,
            self.channel.name().unwrap_or("")
        )
    }
}

/// Every stored column that an expectation describes. **A V17 expectation must describe every channel that is recorded with samples**: one that does not is an
/// error, so that a check cannot pass by having nothing to check; **so must a V16 expectation** (IP-12 phase 12F). An expectation of V18 describes none yet.
pub fn column_expectations(expectation: &Expectation) -> Result<Vec<ColumnExpectation>, String> {
    let root = expectation.root();
    let version = int(root, "file_version")?;
    let mut found = Vec::new();
    for laser in array(root, "lasers")? {
        let key = LaserKey::new(text(laser, "key")?, int(laser, "number")? as u32);
        for channel in array(laser, "channels")? {
            let name = text(channel, "source_name")?;
            for entry in array(channel, "layers")? {
                let layer = LayerId(int(entry, "layer")? as u32);
                match entry.get("column") {
                    Some(column) if !column.is_null() => found.push(ColumnExpectation {
                        layer,
                        laser: key.clone(),
                        channel: ChannelKey::new([name]),
                        fingerprint: Fingerprint::from_json(column)
                            .map_err(|e| format!("layer {}, {}, {name}: {e}", layer.0, key.key))?,
                    }),
                    _ if matches!(version, 16 | 17) && text(entry, "state")? == "recorded" => {
                        return Err(format!(
                            "layer {}, {}, {name}: the expectation has no fingerprint for a channel recorded with samples",
                            layer.0, key.key
                        ))
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(found)
}

pub(crate) fn differences(
    who: &str,
    actual: &Fingerprint,
    wanted: &Fingerprint,
    found: &mut Vec<String>,
) {
    let mut say = |what: &str, got: String, want: String| {
        found.push(format!("{who}: {what} is {got}, expected {want}"));
    };
    if actual.stored_as != wanted.stored_as {
        say(
            "the stored type",
            actual.stored_as.clone(),
            wanted.stored_as.clone(),
        );
    }
    if actual.count != wanted.count {
        say(
            "the count of values",
            actual.count.to_string(),
            wanted.count.to_string(),
        );
    }
    if actual.min != wanted.min {
        say(
            "the minimum",
            format!("{:?}", actual.min),
            format!("{:?}", wanted.min),
        );
    }
    if actual.max != wanted.max {
        say(
            "the maximum",
            format!("{:?}", actual.max),
            format!("{:?}", wanted.max),
        );
    }
    if actual.first != wanted.first {
        say(
            "the first values",
            format!("{:?}", actual.first),
            format!("{:?}", wanted.first),
        );
    }
    if actual.last != wanted.last {
        say(
            "the last values",
            format!("{:?}", actual.last),
            format!("{:?}", wanted.last),
        );
    }
    if actual.sha256 != wanted.sha256 {
        say(
            "the SHA-256 of the values",
            actual.sha256.chars().take(16).collect::<String>() + "...",
            wanted.sha256.chars().take(16).collect::<String>() + "...",
        );
    }
}

/// Every way in which what `source` serves is not what the fingerprints of `expectation` say, as lines that name the layer, the laser and the channel.
///
/// Each described column is read in full, as stored, with a context that never cancels; one that cannot be read is reported with the error. An expectation that
/// cannot be read as one that describes columns is a single line.
pub fn check_columns(source: &dyn ColumnSource, expectation: &Expectation) -> Vec<String> {
    let columns = match column_expectations(expectation) {
        Ok(columns) => columns,
        Err(problem) => return vec![problem],
    };
    let context = ReadContext::new();
    let mut found = Vec::new();
    for column in &columns {
        let who = column.whose();
        let request = ColumnRequest::new(
            column.layer,
            Some(column.laser.clone()),
            column.channel.clone(),
        );
        match source.read(&request, &context) {
            Err(error) => found.push(format!("{who}: the column was not read: {error}")),
            Ok(data) => match Fingerprint::of(&data) {
                None => found.push(format!("{who}: the column is not a column of numbers")),
                Some(actual) => differences(&who, &actual, &column.fingerprint, &mut found),
            },
        }
    }
    found
}
