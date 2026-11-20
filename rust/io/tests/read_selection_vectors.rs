//! The read selection against the vectors of Vision's stage package (IP-27 phase 27C; decisions D27-4, D27-14 and D27-15; requirements BL-FA-02, BL-FA-08, BL-CD-09).
//!
//! `testdata/vectors/read_selection.json` was generated in Vision from its own code, at the commit it names, and copied here with its hash (`the_vectors_match_their_checksums`
//! in the test kit holds the copy to it). Each case is run **through the library's read selection** and compared with what Vision recorded, **bit for bit** (an `f64` by
//! its bits, a count and a digest as they are written):
//!
//! * a `pure` case is a laser's native state, a stride and the rows to keep. The library says which rows it keeps (their native indices and the state of each), whether the laser
//!   is unavailable (a filter and no state channel: `NotPresent`) and, as Vision's loader does, whether the laser is skipped (a laser with no kept row, with any rows: an empty laser is skipped even with all rows).
//! * a `file` case names one of the library's miniatures, a layer by its position, a stride, the rows and which streams are wanted. The library reads each channel of each laser
//!   **twice over**: with `read_selected` for every channel, and with the rows of the laser chosen once (`laser_rows`) and each channel cut with them (`read_rows`, decision D27-17);
//!   both must give Vision's bits. The test the test makes the one decision that is not the selection's, **which lasers the raw tier reports**: a laser is in the answer when a stream that was
//!   asked for has its X and Y rows, and `n` is the shortest of the streams' rows, exactly as Vision's raw tier decides it.
//!
//! **What is not compared:** the wording of Vision's `reason` (its words, not the rule), and the columns of a stream that was not asked for (Vision fills them with `NaN`, which is
//! Vision's loader, not a selection).

use std::collections::BTreeMap;

use build_log_io::dispatch::{open, production};
use build_log_model::{
    Axis, BuildIdentity, BuildMeta, BuildSource, ChannelDType, ChannelDescriptor, ChannelKey,
    ChannelKind, ClockClass, ColumnData, ColumnRequest, EnumLabels, ErrorKind, InMemorySource,
    LaserKey, LaserLayerMeta, LaserMeta, LaserRows, LayerId, NumericType, Precision, Presence,
    ReadContext, ReadSelection, SelectedColumn, Shared, StateRows,
};
use build_log_testkit::{miniature_path, testdata_dir};
use serde_json::Value;

const VECTORS: &str = "vectors/read_selection.json";

// ---------------------------------------------------------------- the values of a vector

#[derive(Debug, Clone, PartialEq)]
enum Col {
    I64(Vec<i64>),
    /// The exact bits of each value.
    F64(Vec<u64>),
    Text(Vec<String>),
}

fn bits(values: &[f64]) -> Col {
    Col::F64(values.iter().map(|v| v.to_bits()).collect())
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

type Columns = BTreeMap<String, Col>;

fn expected_columns(case: &Value) -> Columns {
    case["expected"]["columns"]
        .as_object()
        .expect("expected.columns")
        .iter()
        .map(|(name, doc)| (name.clone(), column_of(doc)))
        .collect()
}

/// FNV-1a, 64 bits, over bytes: the digest that the vectors use, written as a signed integer.
fn fnv1a64(bytes: impl IntoIterator<Item = u8>) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as i64
}

fn digest_f64(values: &[f64]) -> i64 {
    fnv1a64(values.iter().flat_map(|v| v.to_bits().to_le_bytes()))
}

fn digest_f32(values: &[f32]) -> i64 {
    fnv1a64(values.iter().flat_map(|v| v.to_bits().to_le_bytes()))
}

fn digest_i64(values: &[i64]) -> i64 {
    fnv1a64(values.iter().flat_map(|v| v.to_le_bytes()))
}

fn digest_u8(values: &[u8]) -> i64 {
    fnv1a64(values.iter().copied())
}

/// The first and the last four values, or all of them when there are no more than eight.
fn head_and_tail(values: &[f64]) -> (Vec<f64>, Vec<f64>) {
    if values.len() <= 8 {
        (values.to_vec(), Vec::new())
    } else {
        (values[..4].to_vec(), values[values.len() - 4..].to_vec())
    }
}

fn rows_of(text: &str) -> StateRows {
    match text {
        "all" => StateRows::All,
        "on" => StateRows::OnOnly,
        "off" => StateRows::OffOnly,
        other => panic!("rows is all, on or off, not {other:?}"),
    }
}

fn selection_of(inputs: &Value) -> ReadSelection {
    ReadSelection::new()
        .with_stride(inputs["stride"].as_u64().expect("stride"))
        .with_rows(rows_of(inputs["rows"].as_str().expect("rows")))
}

// ---------------------------------------------------------------- a pure case

const LAYER: LayerId = LayerId(1);

fn the_laser() -> LaserKey {
    LaserKey::new("1", 1)
}

/// A source with one laser whose only column is `values`. `as_state` says whether the build describes that column as the laser's state: a laser whose state is missing is a
/// laser with no state channel.
fn one_laser(values: &[u8], as_state: bool) -> InMemorySource {
    let mut build = BuildMeta::new(BuildIdentity {
        build_id: Some("vector".into()),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    });
    let mut laser = LaserMeta::new(the_laser());
    let samples = Presence::from_count(values.len() as u64);
    laser.layers.insert(LAYER, LaserLayerMeta::new(samples));
    let (kind, dtype) = if as_state {
        (
            ChannelKind::LaserState,
            ChannelDType::Enum {
                labels: EnumLabels::Unknown,
            },
        )
    } else {
        (ChannelKind::Sensor, ChannelDType::Numeric(NumericType::U8))
    };
    let mut descriptor = ChannelDescriptor::new(
        ChannelKey::new(["state"]),
        kind,
        ClockClass::LaserSynchronous,
        dtype,
    );
    descriptor.presence.insert(LAYER, samples);
    laser.channels.push(descriptor);
    build.add_laser(laser);
    build.synthesise_missing_layers();
    let mut source = InMemorySource::new();
    source.set_build(build);
    source.insert(
        LAYER,
        Some(the_laser()),
        ChannelKey::new(["state"]),
        ColumnData::U8(Shared::new(values.to_vec())),
    );
    source
}

fn run_pure(inputs: &Value) -> Columns {
    let state: Vec<u8> = inputs["native_state"]
        .as_array()
        .expect("native_state")
        .iter()
        .map(|v| u8::try_from(v.as_u64().expect("a byte")).expect("a byte"))
        .collect();
    let available = inputs["state_available"].as_bool().unwrap_or(true);
    let selection = selection_of(inputs);
    let source = one_laser(&state, available);
    let read = |selection: &ReadSelection| {
        source.read_selected(
            &ColumnRequest::new(LAYER, Some(the_laser()), ChannelKey::new(["state"])),
            selection,
            &ReadContext::new(),
        )
    };

    // the strided length, from the same stride with every row (Vision's `strided_len` and its `kept_points` formula)
    let strided = read(&ReadSelection::new().with_stride(selection.stride()))
        .expect("every row needs no state")
        .len() as i64;
    let mut out = Columns::new();
    out.insert("native_len".into(), Col::I64(vec![state.len() as i64]));
    out.insert("strided_len".into(), Col::I64(vec![strided]));
    out.insert("kept_points_formula".into(), Col::I64(vec![strided]));

    if selection.rows() != StateRows::All && !available {
        let error = read(&selection).expect_err("a filter with no state channel is refused");
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{error}"
        );
        out.insert("laser".into(), Col::Text(vec!["unavailable".into()]));
        out.insert("kept_len".into(), Col::I64(vec![]));
        out.insert("native_index".into(), Col::I64(vec![]));
        out.insert("state_kept".into(), Col::I64(vec![]));
        return out;
    }
    let selected = read(&selection).expect("the selection is answered");
    let native: Vec<i64> = selected.native_index.iter().map(|n| n as i64).collect();
    // the state of each kept row, as the library returned it
    let kept_state: Vec<i64> = match &selected.values {
        ColumnData::U8(v) => v.as_slice().iter().map(|s| i64::from(*s)).collect(),
        other => panic!("the state came back as {other:?}"),
    };
    // the same rows picked by hand from the native state, so the values are checked against the indices as well
    for (value, index) in kept_state.iter().zip(&native) {
        assert_eq!(*value, i64::from(state[*index as usize]));
    }
    // **A laser with no kept row is skipped, whatever rows were asked for:** Vision's loader skips a laser whose position streams are all empty even with all rows (`EmptyLaserData`). The rule used
    // to be `rows != All && no row`, which called an empty laser read with all rows "kept" (a finding of a review of Vision's pull request, 2026-11-19, and a fix of the vector).
    let skipped = selected.is_empty();
    out.insert(
        "laser".into(),
        Col::Text(vec![if skipped { "skipped" } else { "kept" }.into()]),
    );
    out.insert("kept_len".into(), Col::I64(vec![native.len() as i64]));
    out.insert("native_index".into(), Col::I64(native));
    out.insert("state_kept".into(), Col::I64(kept_state));
    out
}

// ---------------------------------------------------------------- a file case

fn f64s_of(column: &ColumnData) -> Vec<f64> {
    column.as_f64().expect("a read at f64").to_vec()
}

fn f32s_of(column: &ColumnData) -> Vec<f32> {
    match column {
        ColumnData::F32(v) => v.as_slice().to_vec(),
        ColumnData::F64(v) => v.as_slice().iter().map(|x| *x as f32).collect(),
        other => panic!("a power that is not a float: {other:?}"),
    }
}

/// One stream of a laser: the X, Y and Z of a position, read at `f64` with the selection.
struct Stream {
    x: SelectedColumn,
    y: SelectedColumn,
    z: Option<Vec<f64>>,
}

/// What the test knows about the columns of one laser in one layer.
struct LaserColumns<'a> {
    source: &'a dyn BuildSource,
    layer: LayerId,
    laser: LaserKey,
    channels: Vec<ChannelDescriptor>,
    selection: ReadSelection,
    /// The rows of the laser, when the case is run with them chosen once.
    rows: Option<LaserRows>,
}

impl LaserColumns<'_> {
    fn key_of(&self, kind: ChannelKind) -> Option<ChannelKey> {
        self.channels
            .iter()
            .find(|channel| channel.kind == kind)
            .map(|channel| channel.key.clone())
    }

    fn read(&self, key: &ChannelKey, precision: Precision) -> SelectedColumn {
        let request = ColumnRequest::new(self.layer, Some(self.laser.clone()), key.clone())
            .with_precision(precision);
        match &self.rows {
            Some(rows) => self.source.read_rows(&request, rows, &ReadContext::new()),
            None => self
                .source
                .read_selected(&request, &self.selection, &ReadContext::new()),
        }
        .unwrap_or_else(|e| panic!("{key:?}: {e}"))
    }

    /// The stream of commanded or actual positions, if the laser has its X and Y and both have rows after the selection.
    fn stream(&self, kind: impl Fn(Axis) -> ChannelKind) -> Option<Stream> {
        let (kx, ky) = (self.key_of(kind(Axis::X))?, self.key_of(kind(Axis::Y))?);
        let (x, y) = (
            self.read(&kx, Precision::F64),
            self.read(&ky, Precision::F64),
        );
        if x.is_empty() || y.is_empty() {
            return None;
        }
        let z = self
            .key_of(kind(Axis::Z))
            .map(|key| f64s_of(&self.read(&key, Precision::F64).values));
        Some(Stream { x, y, z })
    }
}

fn insert_digests(out: &mut Columns, prefix: &str, name: &str, values: &[f64], show: bool) {
    out.insert(
        format!("{prefix}_{name}_digest"),
        Col::I64(vec![digest_f64(values)]),
    );
    if show {
        let (head, tail) = head_and_tail(values);
        out.insert(format!("{prefix}_{name}_head"), bits(&head));
        out.insert(format!("{prefix}_{name}_tail"), bits(&tail));
    }
}

/// The columns that the case does not compare: the streams that were not asked for (or that the laser has no rows of).
fn not_compared(name: &str, skipped: &[String]) -> bool {
    skipped.iter().any(|prefix| name.contains(prefix.as_str()))
}

fn run_file(inputs: &Value, rows_once: bool) -> (Columns, Vec<String>) {
    let name = inputs["miniature"].as_str().expect("miniature");
    let position = usize::try_from(inputs["layer_position"].as_u64().expect("layer_position"))
        .expect("layer_position");
    let selection = selection_of(inputs);
    let wants = |key: &str| inputs[key].as_bool().expect("a flag");
    let (commanded, actual, power) = (wants("commanded"), wants("actual"), wants("power"));

    let opened = open(production(), miniature_path(name)).expect("the miniature opens");
    let source = opened.source();
    let layers = source.layers().expect("the layers");
    let layer = layers[position];
    let detail = source.layer_meta(layer).expect("the layer");

    let mut out = Columns::new();
    out.insert("layer_id".into(), Col::Text(vec![format!("Layer_{layer}")]));
    let mut skipped: Vec<String> = Vec::new();
    let mut present: Vec<i64> = Vec::new();

    for (index, found) in detail.lasers.iter().enumerate() {
        // with the rows chosen once, a laser that has no state for a filter is refused at the choosing, and is skipped as it is in the other way
        let rows = if rows_once {
            match source.laser_rows(layer, &found.key, &selection, &ReadContext::new()) {
                Ok(rows) => Some(rows),
                Err(e) if matches!(e.kind(), ErrorKind::NotPresent { .. }) => continue,
                Err(e) => panic!("{}: {e}", found.key.number),
            }
        } else {
            None
        };
        let columns = LaserColumns {
            source,
            layer,
            laser: found.key.clone(),
            channels: found.channels.clone(),
            selection,
            rows,
        };
        // a filter needs the state, and a laser with no row of it is skipped like an empty one
        let state_key = columns.key_of(ChannelKind::LaserState);
        if selection.rows() != StateRows::All {
            let Some(state_key) = &state_key else {
                continue;
            };
            if columns.read(state_key, Precision::Native).is_empty() {
                continue;
            }
        }
        let commanded_stream = commanded
            .then(|| columns.stream(|axis| ChannelKind::CommandedPosition { axis }))
            .flatten();
        let actual_stream = actual
            .then(|| columns.stream(|axis| ChannelKind::ActualPosition { axis }))
            .flatten();
        // the shortest of the streams that were read, as the raw tier takes it
        let lengths: Vec<usize> = [&commanded_stream, &actual_stream]
            .iter()
            .filter_map(|stream| stream.as_ref())
            .flat_map(|stream| [stream.x.len(), stream.y.len()])
            .collect();
        let Some(n) = lengths.iter().copied().min() else {
            continue;
        };
        present.push(index as i64);
        let prefix = format!("laser{index}");

        // the native index of each row: the one that the first stream's X came back with, cut to n
        let first = commanded_stream
            .as_ref()
            .or(actual_stream.as_ref())
            .expect("a stream");
        let native: Vec<i64> = first
            .x
            .native_index
            .iter()
            .take(n)
            .map(|v| v as i64)
            .collect();
        out.insert(format!("{prefix}_n"), Col::I64(vec![n as i64]));
        out.insert(
            format!("{prefix}_native_index_len"),
            Col::I64(vec![native.len() as i64]),
        );
        out.insert(
            format!("{prefix}_native_index_digest"),
            Col::I64(vec![digest_i64(&native)]),
        );

        for (label, stream) in [("cmd", &commanded_stream), ("act", &actual_stream)] {
            let Some(stream) = stream else {
                skipped.push(format!("{prefix}_{label}_"));
                continue;
            };
            let cut = |values: &[f64]| values[..n.min(values.len())].to_vec();
            insert_digests(
                &mut out,
                &prefix,
                &format!("{label}_x"),
                &cut(&f64s_of(&stream.x.values)),
                true,
            );
            insert_digests(
                &mut out,
                &prefix,
                &format!("{label}_y"),
                &cut(&f64s_of(&stream.y.values)),
                false,
            );
            // a Z that is there and long enough is cut to n; with none, a stream that was read has zeros
            let z = match &stream.z {
                Some(z) if z.len() >= n => z[..n].to_vec(),
                _ => vec![0.0; n],
            };
            insert_digests(&mut out, &prefix, &format!("{label}_z"), &z, false);
        }

        // the power: the values when there are exactly n of them, and what was not asked for is not compared
        let power_values = power
            .then(|| columns.key_of(ChannelKind::LaserPower))
            .flatten()
            .map(|key| f32s_of(&columns.read(&key, Precision::Native).values))
            .filter(|values| values.len() == n);
        match power_values {
            Some(values) => {
                out.insert(
                    format!("{prefix}_power_digest"),
                    Col::I64(vec![digest_f32(&values)]),
                );
                let wide: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
                let (head, tail) = head_and_tail(&wide);
                out.insert(format!("{prefix}_power_head"), bits(&head));
                out.insert(format!("{prefix}_power_tail"), bits(&tail));
            }
            None => skipped.push(format!("{prefix}_power_")),
        }

        // the state: the values when there are exactly n of them, and laser-on otherwise
        let state: Vec<u8> = state_key
            .as_ref()
            .map(|key| columns.read(key, Precision::Native).values)
            .and_then(|values| match values {
                ColumnData::U8(v) => Some(v.as_slice().to_vec()),
                _ => None,
            })
            .filter(|values| values.len() == n)
            .unwrap_or_else(|| vec![1; n]);
        out.insert(
            format!("{prefix}_state_digest"),
            Col::I64(vec![digest_u8(&state)]),
        );
        let wide: Vec<f64> = state.iter().map(|v| f64::from(*v)).collect();
        let (head, tail) = head_and_tail(&wide);
        out.insert(format!("{prefix}_state_head"), bits(&head));
        out.insert(format!("{prefix}_state_tail"), bits(&tail));
    }
    out.insert("lasers_present".into(), Col::I64(present));
    (out, skipped)
}

// ---------------------------------------------------------------- the run

fn load_vectors() -> Value {
    let path = testdata_dir().join(VECTORS);
    let text = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&text).expect("the vectors are JSON")
}

/// Compares what the library did with what Vision recorded, column by column; the lines say which column of which case differs.
fn differences(
    case: &str,
    expected: &Columns,
    actual: &Columns,
    skipped: &[String],
) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, want) in expected {
        // Vision's own words for why a laser is unavailable or skipped are not the rule
        if name == "reason" || not_compared(name, skipped) {
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
        if !expected.contains_key(name) {
            lines.push(format!(
                "{case}: the library gave {name}, which Vision did not record"
            ));
        }
    }
    lines
}

fn run_kind(wanted: &str) -> usize {
    let doc = load_vectors();
    assert_eq!(doc["chunk"], "read_selection");
    let mut ran = 0;
    let mut lines = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        let kind = case["kind"].as_str().expect("kind");
        if kind != wanted {
            continue;
        }
        let label = case["case"].as_str().expect("case");
        let expected = expected_columns(case);
        match kind {
            "pure" => {
                let actual = run_pure(&case["inputs"]);
                lines.extend(differences(label, &expected, &actual, &[]));
            }
            "file" => {
                for (way, rows_once) in [("one call each", false), ("rows chosen once", true)] {
                    let (actual, skipped) = run_file(&case["inputs"], rows_once);
                    let label = format!("{label} [{way}]");
                    lines.extend(differences(&label, &expected, &actual, &skipped));
                }
            }
            other => panic!("unknown kind {other:?}"),
        }
        ran += 1;
    }
    assert!(
        lines.is_empty(),
        "the library's read selection differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    ran
}

#[test]
fn the_pure_cases_of_the_vectors_pass_bit_for_bit() {
    assert_eq!(run_kind("pure"), 15, "the file holds 15 pure cases");
}

#[test]
fn the_cases_on_the_miniatures_pass_bit_for_bit() {
    assert_eq!(
        run_kind("file"),
        20,
        "the file holds 20 cases on miniatures"
    );
}

#[test]
fn the_vectors_name_only_the_miniatures_the_library_holds() {
    let doc = load_vectors();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] == "file" {
            let name = case["inputs"]["miniature"].as_str().expect("miniature");
            assert!(
                miniature_path(name).is_file(),
                "{} names {name}, which is not in testdata/",
                case["case"]
            );
        }
    }
}
