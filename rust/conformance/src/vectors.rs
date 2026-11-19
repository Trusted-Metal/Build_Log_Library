//! The vectors of the stage packages, read and compared (IP-27, decisions D27-14 and D27-15).
//!
//! A vector file (`testdata/vectors/<chunk>.json`) is generated in Vision from Vision's own code, one file for each chunk, and is held to a hash by the test kit. Each case has its inputs and
//! **the outputs that Vision's code returned for them**. This module is what a library test needs to run a case and compare: it reads the file, turns the recorded outputs into
//! [`Columns`], and says in what places a result differs from them, **bit for bit** (a number by its bits, a digest and a count as written).
//!
//! * **A column** is one of three kinds: the bits of numbers ([`Col::F64`]), whole numbers ([`Col::I64`]) or words ([`Col::Text`]).
//! * **The digests** are Vision's: FNV-1a over the little-endian bytes of the values (an `f64` or `f32` as its bits), written as a signed 64-bit integer.
//! * **A case's reason text** is Vision's wording and is not the rule: a test leaves it out of the comparison with the `skip` of [`differences`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

/// A column of a vector's recorded output.
#[derive(Debug, Clone, PartialEq)]
pub enum Col {
    /// Whole numbers (a count, a digest, a native index).
    I64(Vec<i64>),
    /// The exact bits of each number: equal means the same bits, so `0.0` is not `-0.0` and a `NaN` equals only the same `NaN`.
    F64(Vec<u64>),
    /// Words (a layer id, a reason).
    Text(Vec<String>),
}

/// The columns of one output, by name.
pub type Columns = BTreeMap<String, Col>;

/// The bits of numbers, as a column.
pub fn bits(values: &[f64]) -> Col {
    Col::F64(values.iter().map(|v| v.to_bits()).collect())
}

/// Whole numbers, as a column.
pub fn whole(values: &[i64]) -> Col {
    Col::I64(values.to_vec())
}

/// Words, as a column.
pub fn words(values: &[&str]) -> Col {
    Col::Text(values.iter().map(|w| w.to_string()).collect())
}

/// Where the file of a chunk's vectors is: `testdata/vectors/<chunk>.json` of this repository.
pub fn vectors_path(chunk: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the conformance crate lives two folders below the repository root")
        .join("testdata")
        .join("vectors")
        .join(format!("{chunk}.json"))
}

/// The vector file of a chunk, parsed. `Err` says which file could not be read or why it is not JSON.
pub fn load_vectors(chunk: &str) -> Result<Value, String> {
    let path = vectors_path(chunk);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if doc["chunk"] != chunk {
        return Err(format!(
            "{} holds the chunk {}, not {chunk}",
            path.display(),
            doc["chunk"]
        ));
    }
    Ok(doc)
}

fn column_of(doc: &Value) -> Col {
    if let Some(hex) = doc.get("f64_bits") {
        return Col::F64(
            hex.as_array()
                .expect("f64_bits is a list")
                .iter()
                .map(|h| {
                    let text = h.as_str().expect("hex text");
                    u64::from_str_radix(text.trim_start_matches("0x"), 16).expect("hex bits")
                })
                .collect(),
        );
    }
    if let Some(values) = doc.get("i64") {
        return Col::I64(
            values
                .as_array()
                .expect("i64 is a list")
                .iter()
                .map(|v| v.as_i64().expect("a whole number"))
                .collect(),
        );
    }
    if let Some(values) = doc.get("text") {
        return Col::Text(
            values
                .as_array()
                .expect("text is a list")
                .iter()
                .map(|v| v.as_str().expect("words").to_string())
                .collect(),
        );
    }
    panic!("a column that is neither f64_bits, i64 nor text: {doc}");
}

/// The recorded output of a case, as columns. An `f64` column is read from its `f64_bits`, never from the decimal numbers beside them.
pub fn expected_columns(case: &Value) -> Columns {
    case["expected"]["columns"]
        .as_object()
        .expect("expected.columns")
        .iter()
        .map(|(name, doc)| (name.clone(), column_of(doc)))
        .collect()
}

/// A number of a vector's inputs: a JSON number, or the words `NaN`, `inf` and `-inf`; `None` for a value that is absent or `null`.
pub fn opt_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Null => None,
        Value::Number(n) => n.as_f64(),
        Value::String(s) => match s.as_str() {
            "NaN" => Some(f64::NAN),
            "inf" => Some(f64::INFINITY),
            "-inf" => Some(f64::NEG_INFINITY),
            other => panic!("a number of a vector's inputs is not {other:?}"),
        },
        other => panic!("a number of a vector's inputs is not {other}"),
    }
}

/// The number that a vector writes as the hex text of its bits (`0x3ee4f8b588e368f1`).
pub fn f64_from_bits_text(text: &str) -> f64 {
    let digits = text
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("the bits of a number start with 0x, not {text:?}"));
    f64::from_bits(u64::from_str_radix(digits, 16).expect("hex bits"))
}

/// A number input of a case **read from its exact bits**. A case that has a float input carries `<key>_bits` (hex text) beside the decimal text that a person reads, because a decimal
/// that goes through JSON can come back a unit in the last place off with the default parser, and a stage's result can depend on the last place (Vision's note to the recipe, IP-27 phase
/// 27E). A case without the bits is read from the number itself (`NaN`, `inf` and `-inf` as words).
pub fn exact_f64(inputs: &Value, key: &str) -> f64 {
    match inputs.get(format!("{key}_bits")) {
        Some(bits) => f64_from_bits_text(bits.as_str().expect("the bits are hex text")),
        None => opt_f64(&inputs[key]).unwrap_or_else(|| panic!("the input {key} is a number")),
    }
}

/// An optional number input of a case: `null` or absent is `None`; otherwise as [`exact_f64`] (an override that is not set, or one that is set to `NaN`, which is the word `NaN` in the text and
/// its bits beside it).
pub fn exact_opt_f64(inputs: &Value, key: &str) -> Option<f64> {
    match inputs.get(key) {
        None | Some(Value::Null) => None,
        Some(_) => Some(exact_f64(inputs, key)),
    }
}

/// A list of number inputs of a case, read from its exact bits when it has them (see [`exact_f64`]).
pub fn exact_f64s(inputs: &Value, key: &str) -> Vec<f64> {
    match inputs.get(format!("{key}_bits")) {
        Some(bits) => bits
            .as_array()
            .expect("the bits are a list of hex text")
            .iter()
            .map(|b| f64_from_bits_text(b.as_str().expect("hex text")))
            .collect(),
        None => inputs[key]
            .as_array()
            .unwrap_or_else(|| panic!("the input {key} is a list of numbers"))
            .iter()
            .map(|n| opt_f64(n).unwrap_or_else(|| panic!("a number of {key}")))
            .collect(),
    }
}

/// A whole number of a vector's inputs; `None` for a value that is absent or `null`.
pub fn opt_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Null => None,
        other => Some(other.as_i64().expect("a whole number")),
    }
}

/// FNV-1a, 64 bits, over bytes, as the vectors write their digests (a signed integer).
pub fn fnv1a64(bytes: impl IntoIterator<Item = u8>) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as i64
}

/// The digest of `f64` values: their bits, little-endian.
pub fn digest_f64(values: &[f64]) -> i64 {
    fnv1a64(values.iter().flat_map(|v| v.to_bits().to_le_bytes()))
}

/// The digest of `f32` values: their bits, little-endian.
pub fn digest_f32(values: &[f32]) -> i64 {
    fnv1a64(values.iter().flat_map(|v| v.to_bits().to_le_bytes()))
}

/// The digest of whole numbers: eight bytes each, little-endian.
pub fn digest_i64(values: &[i64]) -> i64 {
    fnv1a64(values.iter().flat_map(|v| v.to_le_bytes()))
}

/// The digest of bytes.
pub fn digest_u8(values: &[u8]) -> i64 {
    fnv1a64(values.iter().copied())
}

/// The first and the last four values, or all of them (and no tail) when there are no more than eight, for a person to read in a vector.
pub fn head_and_tail(values: &[f64]) -> (Vec<f64>, Vec<f64>) {
    if values.len() <= 8 {
        (values.to_vec(), Vec::new())
    } else {
        (values[..4].to_vec(), values[values.len() - 4..].to_vec())
    }
}

/// In what places the `actual` columns of a case differ from the `expected` ones, as lines that say which column and what each side holds.
/// A column that `skip` names is not compared (Vision's wording of a reason; the columns of a stream that was not asked for). A column that is
/// only on one side is a difference.
pub fn differences(
    case: &str,
    expected: &Columns,
    actual: &Columns,
    skip: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, want) in expected {
        if skip(name) {
            continue;
        }
        match actual.get(name) {
            None => lines.push(format!("{case}: the library has no column {name}")),
            Some(got) if got != want => lines.push(format!(
                "{case}: {name}: Vision recorded {want:?}, the library gave {got:?}"
            )),
            Some(_) => {}
        }
    }
    for name in actual.keys() {
        if !skip(name) && !expected.contains_key(name) {
            lines.push(format!(
                "{case}: the library gave {name}, which Vision did not record"
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_is_fnv_1a_over_the_little_endian_bytes() {
        // the offset basis is the digest of nothing, as the signed integer the vectors write
        assert_eq!(digest_u8(&[]), 0xcbf2_9ce4_8422_2325_u64 as i64);
        // FNV-1a 64 of "a" is af63dc4c8601ec8c
        assert_eq!(digest_u8(b"a"), 0xaf63_dc4c_8601_ec8c_u64 as i64);
        assert_ne!(digest_f64(&[0.0]), digest_f64(&[-0.0]));
        assert_eq!(digest_i64(&[1]), fnv1a64(1_i64.to_le_bytes()));
        assert_eq!(digest_f32(&[1.0]), fnv1a64(1.0_f32.to_bits().to_le_bytes()));
    }

    #[test]
    fn the_numbers_of_a_vectors_inputs_may_be_words_for_the_values_json_has_no_number_for() {
        assert!(opt_f64(&serde_json::json!("NaN")).is_some_and(f64::is_nan));
        assert_eq!(opt_f64(&serde_json::json!("-inf")), Some(f64::NEG_INFINITY));
        assert_eq!(opt_f64(&serde_json::json!(76.5)), Some(76.5));
        assert_eq!(opt_f64(&Value::Null), None);
        assert_eq!(opt_i64(&serde_json::json!(-4)), Some(-4));
        assert_eq!(opt_i64(&Value::Null), None);
    }

    #[test]
    fn a_float_input_is_read_from_its_exact_bits_when_the_case_has_them() {
        // the decimal 1e-5 and the number Vision's code held (one unit in the last place below it) are different numbers
        let held = f64::from_bits(0x3ee4_f8b5_88e3_68f0);
        assert_ne!(held, 1e-5);
        let case = serde_json::json!({
            "period": 1e-5, "period_bits": "0x3ee4f8b588e368f0",
            "values": [1e-5, "NaN"], "values_bits": ["0x3ee4f8b588e368f0", "0x7ff8000000000000"],
            "plain": 2.5, "words": ["inf", -1.5]
        });
        assert_eq!(exact_f64(&case, "period").to_bits(), held.to_bits());
        let values = exact_f64s(&case, "values");
        assert_eq!(values[0].to_bits(), held.to_bits());
        assert!(values[1].is_nan());
        // an optional input: not set is `None`, and a NaN that is set is `Some`
        let optional =
            serde_json::json!({ "a": null, "b": "NaN", "b_bits": "0x7ff8000000000000", "c": 2.5 });
        assert_eq!(exact_opt_f64(&optional, "a"), None);
        assert_eq!(exact_opt_f64(&optional, "missing"), None);
        assert!(exact_opt_f64(&optional, "b").is_some_and(f64::is_nan));
        assert_eq!(exact_opt_f64(&optional, "c"), Some(2.5));
        // without the bits, the number itself, and the words for what JSON has no number for
        assert_eq!(exact_f64(&case, "plain"), 2.5);
        assert_eq!(exact_f64s(&case, "words"), vec![f64::INFINITY, -1.5]);
        assert_eq!(f64_from_bits_text("0x3ff0000000000000"), 1.0);
    }

    #[test]
    fn the_head_and_tail_keep_a_short_column_whole() {
        assert_eq!(head_and_tail(&[1.0, 2.0]), (vec![1.0, 2.0], vec![]));
        let long: Vec<f64> = (0..20).map(f64::from).collect();
        let (head, tail) = head_and_tail(&long);
        assert_eq!(
            (head, tail),
            (vec![0.0, 1.0, 2.0, 3.0], vec![16.0, 17.0, 18.0, 19.0])
        );
    }

    #[test]
    fn a_difference_names_the_column_and_both_sides_and_a_skipped_column_is_left_out() {
        let mut want = Columns::new();
        want.insert("n".into(), whole(&[3]));
        want.insert("reason".into(), words(&["Vision's words"]));
        let mut got = Columns::new();
        got.insert("n".into(), whole(&[4]));
        got.insert("extra".into(), whole(&[1]));
        let lines = differences("case", &want, &got, &|name| name == "reason");
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("n: Vision recorded I64([3]), the library gave I64([4])"));
        assert!(lines[1].contains("the library gave extra"));
        assert!(differences("case", &want, &want, &|_| false).is_empty());
    }

    #[test]
    fn the_vector_file_of_a_chunk_is_found_and_names_its_chunk() {
        // the file of 27C is committed: it parses, and it is the chunk that its name says
        let doc = load_vectors("read_selection").expect("the committed vectors of 27C");
        assert_eq!(doc["chunk"], "read_selection");
        assert!(load_vectors("no_such_chunk").is_err());
    }
}
