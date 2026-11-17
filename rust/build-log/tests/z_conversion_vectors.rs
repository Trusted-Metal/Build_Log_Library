//! The Z calibration and conversion against the cases of Vision's vectors that are on the library's miniatures (IP-27 phase 27D; decisions D27-6, D27-14, D27-15 and D27-18; BL-PR-03).
//!
//! `testdata/vectors/z_conversion.json` was generated in Vision from its own code at the commit it names. Each **file** case names one of the miniatures, a layer by its position, a
//! selection and optional overrides of the Z calibration; Vision recorded, for each laser it keeps, the count and **a digest of its converted Z series (commanded and actual)** and their first and last
//! values. Here each is run **through the library**: the file is opened, the rows of each laser are chosen (27C), the calibration that the file and the user give is resolved, Vision's defaults and rules are applied to it
//! as its adapter will, the Z codes are read for those rows and converted by the library, and everything is compared **bit for bit**. The 24 pure cases are run by `rust/processing/tests/z_conversion_vectors.rs`; this file is
//! here because it needs the IO layer (a file to open) and the processing layer together.
//!
//! **The decisions in this test are Vision's and are made here as the adapter of phase 27K will make them** (D27-18): **Vision's defaults and its narrower-dataset rule** (the library reports a missing value as missing); which lasers the raw tier reports (a laser is in the answer when a stream that
//! was asked for has its X and Y rows, and `n` is the shortest of them); that a laser whose Z cannot be converted is **dropped whole**, X and Y included, the commanded series being checked first;
//! and that a Z series that is shorter than `n` is zeros. The library reports each of those facts and decides none of them.

use build_log::testing::miniature_path;
use build_log::{
    open, Axis, BuildSource, ChannelKind, ColumnRequest, ErrorKind, LaserKey, LayerId, Precision,
    ReadContext, ReadSelection, StateRows, ZCalibrationOverride,
};
use build_log_conformance::vectors::{
    bits, differences, digest_f64, expected_columns, head_and_tail, load_vectors, opt_f64, opt_i64,
    whole, words, Columns,
};
use build_log_processing::{resolve_z_calibration, Resolved, StoredWidths, ZConverter, ZSeries};
use serde_json::Value;

const CHUNK: &str = "z_conversion";

// ---------------------------------------------------------------- what Vision does, and the library does not (decision D27-18)

/// Vision's defaults for a Z calibration that the file and the user do not give (Vision's `CalibrationParams::default()`). **They are Vision's:** the library reports a missing value as
/// missing, and the adapter of phase 27K will carry these, as this test does.
const VISION_FIELD_SIZE_Z_MM: f64 = 400.0;
const VISION_OFFSET_Z_MM: f64 = 0.0;
const VISION_BIT_RESOLUTION: i64 = 20;

/// Vision's narrower-dataset rule: a series is converted at the width of its stored type when that is smaller than the calibrated resolution (itself Vision's 20 when none is given).
fn vision_resolution(calibrated: Option<i64>, stored_width: Option<u32>) -> i64 {
    let calibrated = calibrated.unwrap_or(VISION_BIT_RESOLUTION);
    match stored_width {
        Some(width) if i64::from(width) < calibrated => i64::from(width),
        _ => calibrated,
    }
}

fn override_of(value: &Value) -> ZCalibrationOverride {
    let mut o = ZCalibrationOverride::new();
    if let Some(v) = opt_f64(&value["field_size_z"]) {
        o = o.with_field_size_z_mm(v);
    }
    if let Some(v) = opt_i64(&value["commanded_bit_res_z"]) {
        o = o.with_commanded_bit_resolution_z(v);
    }
    if let Some(v) = opt_i64(&value["actual_bit_res_z"]) {
        o = o.with_actual_bit_resolution_z(v);
    }
    o
}

fn rows_of(text: &str) -> StateRows {
    match text {
        "all" => StateRows::All,
        "on" => StateRows::OnOnly,
        "off" => StateRows::OffOnly,
        other => panic!("rows is all, on or off, not {other:?}"),
    }
}

/// What the test reads of one laser: the channels of the laser in the layer, and the rows chosen for them.
struct Laser<'a> {
    source: &'a dyn BuildSource,
    layer: LayerId,
    key: LaserKey,
    channels: Vec<build_log::ChannelDescriptor>,
    rows: build_log::LaserRows,
}

impl Laser<'_> {
    /// The values of the channel of this kind at `f64`, for the rows chosen; `None` if the laser has no such channel.
    fn read(&self, kind: ChannelKind) -> Option<Vec<f64>> {
        let channel = self.channels.iter().find(|c| c.kind == kind)?;
        let request = ColumnRequest::new(self.layer, Some(self.key.clone()), channel.key.clone())
            .with_precision(Precision::F64);
        let cut = self
            .source
            .read_rows(&request, &self.rows, &ReadContext::new())
            .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        Some(cut.values.as_f64().expect("a read at f64").to_vec())
    }

    /// The length of a stream of positions: the shorter of its X and Y, when both have rows after the selection.
    fn stream(&self, kind: fn(Axis) -> ChannelKind) -> Option<usize> {
        let x = self.read(kind(Axis::X))?;
        let y = self.read(kind(Axis::Y))?;
        (!x.is_empty() && !y.is_empty()).then(|| x.len().min(y.len()))
    }
}

fn run_file(inputs: &Value) -> Columns {
    let name = inputs["miniature"].as_str().expect("miniature");
    let position = usize::try_from(inputs["layer_position"].as_u64().expect("layer_position"))
        .expect("layer_position");
    let selection = ReadSelection::new()
        .with_stride(inputs["stride"].as_u64().expect("stride"))
        .with_rows(rows_of(inputs["rows"].as_str().expect("rows")));
    let overrides = override_of(&inputs["override"]);

    let opened = open(miniature_path(name)).expect("the miniature opens");
    let source = opened.source();
    let layer = source.layers().expect("the layers")[position];
    let detail = source.layer_meta(layer).expect("the layer");
    let build = source.build().expect("the build");

    let mut out = Columns::new();
    out.insert("layer_id".into(), words(&[&format!("Layer_{layer}")]));
    let mut present: Vec<i64> = Vec::new();
    for (index, found) in detail.lasers.iter().enumerate() {
        // the rows of the laser; a filter on a laser that has no state is refused, and Vision drops that laser
        let rows = match source.laser_rows(layer, &found.key, &selection, &ReadContext::new()) {
            Ok(rows) => rows,
            Err(e) if matches!(e.kind(), ErrorKind::NotPresent { .. }) => continue,
            Err(e) => panic!("laser {}: {e}", found.key.number),
        };
        let laser = Laser {
            source,
            layer,
            key: found.key.clone(),
            channels: found.channels.clone(),
            rows,
        };
        // a laser is in the answer when a stream that was asked for has its X and Y rows, and n is the shortest of them
        let n = [
            laser.stream(|axis| ChannelKind::CommandedPosition { axis }),
            laser.stream(|axis| ChannelKind::ActualPosition { axis }),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(n) = n else {
            continue;
        };
        // what the library reports: the user's override over the file's value, or nothing; and the stored width of each Z dataset
        let parameters = build.laser(&found.key).and_then(|l| l.parameters.as_ref());
        let calibration = resolve_z_calibration(parameters, &overrides);
        let widths = StoredWidths::of_channels(&found.channels);
        // what Vision does with it: its defaults, its narrower-dataset rule, and the converter of each series
        let field_size = calibration
            .field_size_mm()
            .map_or(VISION_FIELD_SIZE_Z_MM, Resolved::value);
        let offset = calibration
            .offset_mm()
            .map_or(VISION_OFFSET_Z_MM, Resolved::value);
        let commanded = ZConverter::new(
            ZSeries::Commanded,
            field_size,
            offset,
            vision_resolution(
                calibration.commanded_resolution().map(Resolved::value),
                widths.commanded,
            ),
        );
        let actual = ZConverter::new(
            ZSeries::Actual,
            field_size,
            offset,
            vision_resolution(
                calibration.actual_resolution().map(Resolved::value),
                widths.actual,
            ),
        );
        // Vision's decision: a series that cannot be converted drops the whole laser, the commanded one being checked first
        let (Ok(commanded), Ok(actual)) = (commanded, actual) else {
            continue;
        };
        let convert = |kind: ChannelKind, converter: ZConverter| {
            match laser.read(kind) {
                // a series with at least n codes is converted, cut to n; a shorter or missing one is zeros
                Some(codes) if codes.len() >= n => {
                    codes[..n].iter().map(|c| converter.to_mm(*c)).collect()
                }
                _ => vec![0.0; n],
            }
        };
        let commanded_mm: Vec<f64> =
            convert(ChannelKind::CommandedPosition { axis: Axis::Z }, commanded);
        let actual_mm: Vec<f64> = convert(ChannelKind::ActualPosition { axis: Axis::Z }, actual);

        present.push(index as i64);
        let p = format!("laser{index}");
        out.insert(format!("{p}_n"), whole(&[n as i64]));
        out.insert(
            format!("{p}_cmd_z_digest"),
            whole(&[digest_f64(&commanded_mm)]),
        );
        out.insert(
            format!("{p}_act_z_digest"),
            whole(&[digest_f64(&actual_mm)]),
        );
        for (column, values) in [("cmd_z", &commanded_mm), ("act_z", &actual_mm)] {
            let (head, tail) = head_and_tail(values);
            out.insert(format!("{p}_{column}_head"), bits(&head));
            out.insert(format!("{p}_{column}_tail"), bits(&tail));
        }
    }
    out.insert("lasers_present".into(), whole(&present));
    out
}

#[test]
fn the_cases_on_the_miniatures_pass_bit_for_bit() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut ran = 0;
    let mut lines = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] != "file" {
            continue;
        }
        let label = case["case"].as_str().expect("case");
        let expected = expected_columns(case);
        let actual = run_file(&case["inputs"]);
        lines.extend(differences(label, &expected, &actual, &|_| false));
        ran += 1;
    }
    assert!(
        lines.is_empty(),
        "the library's Z conversion differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 12, "the file holds 12 cases on miniatures");
}

#[test]
fn an_override_changes_the_converted_z_and_the_laser_that_cannot_convert_is_dropped_whole() {
    // the same layer with and without an override gives different millimetres (so the cases above are not all the same number), and an invalid resolution drops the laser
    let doc = load_vectors(CHUNK).expect("the vectors");
    let inputs = |label: &str| -> Value {
        doc["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|c| c["case"] == label)
            .unwrap_or_else(|| panic!("no case {label}"))["inputs"]
            .clone()
    };
    let plain = run_file(&inputs("file_v17_mock_default"));
    let wide = run_file(&inputs("file_v17_mock_override_field_size_z_100"));
    assert_ne!(plain["laser0_cmd_z_digest"], wide["laser0_cmd_z_digest"]);
    assert_eq!(plain["laser0_n"], wide["laser0_n"]);
    let dropped = run_file(&inputs(
        "file_v17_mock_invalid_override_resolution_makes_the_laser_drop_out",
    ));
    assert_eq!(dropped["lasers_present"], whole(&[]));
}
