//! The time shift against the cases of Vision's vectors that are on the library's miniatures (IP-27 phase 27F; decisions D27-6, D27-14, D27-15, D27-18 and D27-21; BL-PR-03 and BL-PR-04).
//!
//! `testdata/vectors/time_shift.json` was generated in Vision from its own code at the commit it names. Each **file** case names one of the miniatures, a layer by its position, a selection
//! and optional overrides of the timing offsets; Vision recorded, for each laser it keeps, the count and a digest of its timestamps and of its shifted commanded and actual Z, and the first
//! and last values of each. Here each is run **through the library**: the file is opened, the rows of each laser are chosen (27C), the Z is converted to millimetres (27D), the timestamps are
//! made from the native index of the kept rows, the timing offsets are resolved from what the file records and the user gives (27F), the Z series are shifted, and everything is compared
//! **bit for bit**. The 67 pure cases are run by `rust/processing/tests/time_shift_vectors.rs`; this file is here because it needs a file to open.
//!
//! **The decisions are Vision's and are made here as the adapter of phase 27K will make them** (D27-18), on top of those in `vision_adapter`: the answers for what the file does not say, the
//! mirroring of a commanded override onto the actual offset, the reading of a sample period whose unit the file does not state, the native period under a stride (`(native x stride) /
//! stride`), the time origin of the timestamps (`0`: Vision reads `Start_Time`, which no miniature has, and V17 writes `Start_Laser_Time`), and that a laser of fewer than two samples is not
//! shifted.

mod vision_adapter;

use build_log::{
    Axis, ChannelKind, NativeIndex, ReadSelection, TimingOverride, ZCalibrationOverride,
};
use build_log_conformance::vectors::{
    bits, differences, digest_f64, expected_columns, head_and_tail, load_vectors, opt_f64, whole,
    words, Columns,
};
use build_log_processing::{
    resolve_timing, shift_series, timestamps, CardConvention, RecordedTiming, ShiftRefusal,
    TimingFallbacks, TimingResolution, UnknownUnitAs,
};
use serde_json::Value;

use vision_adapter::{kept_layer, rows_of, vision_period_seconds, KeptLaser};

const CHUNK: &str = "time_shift";

/// Vision's answers where the file is silent (see `rust/processing/tests/time_shift_vectors.rs`, which carries the same decisions for the pure cases).
fn vision_fallbacks() -> TimingFallbacks {
    TimingFallbacks {
        sample_period_s: 1e-5,
        absent_offset_s: 0.0,
        unparsed_offset_s: 0.0,
        unknown_unit: UnknownUnitAs::Steps,
        unrecognised_protocol: CardConvention::OppositeToShifter,
    }
}

/// Vision's mirroring: a commanded override alone is also the actual offset.
fn vision_mirrored(user: &TimingOverride) -> TimingOverride {
    let mut mirrored = user.clone();
    if user.commanded_offset_steps.is_some() && user.actual_offset_steps.is_none() {
        mirrored.actual_offset_steps = user.commanded_offset_steps;
    }
    mirrored
}

fn override_of(value: &Value) -> TimingOverride {
    let mut o = TimingOverride::new();
    if let Some(steps) = opt_f64(&value["commanded_offset_steps"]) {
        o = o.with_commanded_offset_steps(steps);
    }
    if let Some(steps) = opt_f64(&value["actual_offset_steps"]) {
        o = o.with_actual_offset_steps(steps);
    }
    o
}

/// The native sample period under a stride as Vision takes it: `(native x stride) / stride`.
fn vision_native_period(native_s: f64, stride: u64) -> f64 {
    let stride = stride.max(1) as f64;
    (native_s * stride) / stride
}

/// What the library resolves for one laser: what the file records, the native period as Vision reads it, and the user's overrides as Vision mirrors them.
fn timing_of(
    laser: &KeptLaser,
    period_s: Option<f64>,
    stride: u64,
    user: &TimingOverride,
) -> TimingResolution {
    let mut recorded = RecordedTiming::from_parameters(laser.parameters.as_ref());
    if let Some(period) = period_s {
        recorded = recorded.with_native_period_s(vision_native_period(period, stride));
    }
    resolve_timing(&recorded, &vision_mirrored(user), &vision_fallbacks())
}

/// The shifted Z of a laser in millimetres. A laser of fewer than two samples is not shifted (Vision's use case skips it).
fn shifted(
    timing: &TimingResolution,
    kind: ChannelKind,
    times: &[f64],
    values: &[f64],
) -> Vec<f64> {
    match shift_series(timing, &kind, times, values) {
        Ok(series) => series.into_values(),
        Err(ShiftRefusal::TooFewSamples { .. }) => values.to_vec(),
        Err(other) => panic!("{kind:?}: {other}"),
    }
}

fn run_file(inputs: &Value) -> Columns {
    let stride = inputs["stride"].as_u64().expect("stride");
    let selection = ReadSelection::new()
        .with_stride(stride)
        .with_rows(rows_of(inputs["rows"].as_str().expect("rows")));
    let user = override_of(&inputs["override"]);
    let layer = kept_layer(
        inputs["miniature"].as_str().expect("miniature"),
        usize::try_from(inputs["layer_position"].as_u64().expect("layer_position"))
            .expect("layer_position"),
        &selection,
        &ZCalibrationOverride::new(),
    );

    let mut out = Columns::new();
    out.insert("layer_id".into(), words(&[&layer.layer_id]));
    let mut present: Vec<i64> = Vec::new();
    for laser in &layer.lasers {
        let period = laser.recorded_period.as_ref().map(vision_period_seconds);
        // the time of each kept row: origin 0, the native index and the period of the file
        let first_n: Vec<u64> = laser.native_index.iter().take(laser.n).collect();
        let times = timestamps(
            &NativeIndex::Listed(build_log::Shared::new(first_n)),
            0.0,
            period.unwrap_or(1e-5),
        );
        let timing = timing_of(laser, period, stride, &user);
        let commanded = shifted(
            &timing,
            ChannelKind::CommandedPosition { axis: Axis::Z },
            &times,
            &laser.commanded_mm,
        );
        let actual = shifted(
            &timing,
            ChannelKind::ActualPosition { axis: Axis::Z },
            &times,
            &laser.actual_mm,
        );

        present.push(laser.index as i64);
        let p = format!("laser{}", laser.index);
        out.insert(format!("{p}_n"), whole(&[laser.n as i64]));
        out.insert(
            format!("{p}_timestamps_digest"),
            whole(&[digest_f64(&times)]),
        );
        out.insert(
            format!("{p}_cmd_z_digest"),
            whole(&[digest_f64(&commanded)]),
        );
        out.insert(format!("{p}_act_z_digest"), whole(&[digest_f64(&actual)]));
        for (column, values) in [
            ("timestamps", &times),
            ("cmd_z", &commanded),
            ("act_z", &actual),
        ] {
            let (head, tail) = head_and_tail(values);
            out.insert(format!("{p}_{column}_head"), bits(&head));
            out.insert(format!("{p}_{column}_tail"), bits(&tail));
        }
    }
    out.insert("lasers_present".into(), whole(&present));
    out
}

fn case_inputs(doc: &Value, label: &str) -> Value {
    doc["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["case"] == label)
        .unwrap_or_else(|| panic!("no case {label}"))["inputs"]
        .clone()
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
        "the library's time shift differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 15, "the file holds 15 cases on miniatures");
}

/// The cases above would pass if the shift did nothing and the vectors recorded that. They do not: an override moves the shifted Z, a commanded override alone moves the actual Z too
/// (Vision's mirroring, made in the adapter and not in the library), and an actual override alone leaves the commanded Z where the file's offset puts it.
#[test]
fn the_offsets_move_the_z_and_the_mirroring_is_the_adapters() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let plain = run_file(&case_inputs(&doc, "file_v17_oct15_default"));
    let commanded_alone = run_file(&case_inputs(
        &doc,
        "file_v17_oct15_a_commanded_override_alone_is_mirrored_to_the_actual_channels",
    ));
    let actual_alone = run_file(&case_inputs(
        &doc,
        "file_v17_oct15_an_actual_override_alone_leaves_the_commanded_offset_the_files",
    ));
    assert_ne!(
        plain["laser0_cmd_z_digest"],
        commanded_alone["laser0_cmd_z_digest"]
    );
    assert_ne!(
        plain["laser0_act_z_digest"],
        commanded_alone["laser0_act_z_digest"]
    );
    assert_eq!(
        plain["laser0_cmd_z_digest"],
        actual_alone["laser0_cmd_z_digest"]
    );
    assert_ne!(
        plain["laser0_act_z_digest"],
        actual_alone["laser0_act_z_digest"]
    );
    // the timestamps are the same in all three: the shift moves the values, not the axis
    assert_eq!(
        plain["laser0_timestamps_digest"],
        commanded_alone["laser0_timestamps_digest"]
    );
}

/// The library itself does not mirror: given the commanded override alone, the actual offset stays the file's, and what the file records is reported whether or not it was overridden.
#[test]
fn the_library_gives_each_offset_only_its_own_override() {
    let selection = ReadSelection::new();
    let layer = kept_layer("v17_oct15.h5", 0, &selection, &ZCalibrationOverride::new());
    let laser = &layer.lasers[0];
    let period = laser.recorded_period.as_ref().map(vision_period_seconds);
    let recorded = RecordedTiming::from_parameters(laser.parameters.as_ref())
        .with_native_period_s(period.expect("a V17 laser records its sample period"));
    let user = TimingOverride::new().with_commanded_offset_steps(12.0);
    let timing = resolve_timing(&recorded, &user, &vision_fallbacks());
    assert_eq!(
        timing.commanded().seconds().origin(),
        Some(&build_log::Origin::Overridden)
    );
    // the actual offset is the file's own (-8 steps of an SL2 card, negated), with no origin
    assert_eq!(timing.actual().seconds().origin(), None);
    assert_eq!(
        timing.actual().seconds().value().to_bits(),
        (-(-8.0 * period.unwrap())).to_bits()
    );
    // what the miniature records is reported
    assert_eq!(
        timing.commanded().file(),
        &build_log_processing::OffsetReading::Steps
    );
    assert_eq!(timing.convention(), Some(CardConvention::OppositeToShifter));
    assert_eq!(timing.sign().origin(), None);
    // a V17 file states no unit for its sample period, so the period reaches the library from the adapter, and it says so
    assert_eq!(timing.native_period().origin(), None);
    let without = resolve_timing(
        &RecordedTiming::from_parameters(laser.parameters.as_ref()),
        &TimingOverride::new(),
        &vision_fallbacks(),
    );
    assert_eq!(
        without.native_period().origin(),
        Some(&build_log::Origin::CallerSupplied)
    );
}
