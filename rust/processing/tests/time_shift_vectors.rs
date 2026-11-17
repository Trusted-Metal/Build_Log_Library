//! The time shift against the pure cases of Vision's vectors (IP-27 phase 27F; decisions D27-6, D27-14, D27-15, D27-18 and D27-21; requirements BL-PR-03 and BL-PR-04).
//!
//! `testdata/vectors/time_shift.json` was generated in Vision from its own code at the commit it names, and is held to its hash by the test kit. Each **pure** case gives inputs and the outputs that
//! Vision's code returned; **a float input carries its exact bits beside the decimal text, and this test reads the bits.** Each case is run **through the library** and compared **bit for bit**:
//!
//! * `shift`: the shifter on a time axis and values (`shift_channel`);
//! * `parse`: the reading of a list of offset texts (`parse_offset_text`) at a native period;
//! * `resolve`: the user's override over the file's offsets, in seconds (`apply_override`);
//! * `tier1`: a small synthetic layer: the galvo protocols, the offsets as the card holds them, the native period under a stride, the user's override, and the six position series, taken
//!   through `resolve_timing`, `timestamps` and `shift_series`.
//!
//! **What Vision decides, and the library does not, is made here as the adapter of phase 27K will make it** (D27-18): the answers for everything the file does not say (a sample period of
//! 1e-5 s, an offset of 0 when absent or unreadable, an unknown unit read as steps, an unrecognised protocol negated like SL2), **the mirroring of a commanded override onto the actual offset**, the
//! reading of a refusal (series of different lengths are returned as they are; a laser of fewer than two samples is not shifted), and the native period under a stride, which Vision takes as
//! `(native x stride) / stride`. The twelve cases on the miniatures are run by `rust/build-log/tests/time_shift_vectors.rs`, which needs a file to open.

use build_log_conformance::vectors::{
    bits, differences, exact_f64, exact_f64s, expected_columns, load_vectors, opt_f64, whole,
    Columns,
};
use build_log_model::{Axis, ChannelKind, GalvoProtocol, NativeIndex, Shared, TimingOverride};
use build_log_processing::{
    apply_override, parse_offset_text, resolve_timing, shift_channel, shift_series, timestamps,
    CardConvention, ParsedOffset, RecordedOffset, RecordedTiming, ShiftRefusal, TimingFallbacks,
    UnknownUnitAs, SHIFT_FIT_N,
};
use serde_json::Value;

const CHUNK: &str = "time_shift";

// ---------------------------------------------------------------- what Vision does, and the library does not (decision D27-18)

/// Vision's answers where the file is silent. **They are Vision's:** the library reports each case and has no default, and the adapter of phase 27K will carry these, as this test does.
fn vision_fallbacks() -> TimingFallbacks {
    TimingFallbacks {
        // no optical train, or a period that is not above 0
        sample_period_s: 1e-5,
        // no offset (no optical train)
        absent_offset_s: 0.0,
        // empty, blank or unparsable text is 0 seconds, silently
        unparsed_offset_s: 0.0,
        // an unknown unit is read as steps, "rather than as seconds (which would be off by ~1e5)"
        unknown_unit: UnknownUnitAs::Steps,
        // a protocol that is neither XY2 nor SL2, or none, is treated as SL2
        unrecognised_protocol: CardConvention::OppositeToShifter,
    }
}

/// Vision's mirroring: **a commanded override alone is also the actual offset** (its code says both channels usually share the delay); an actual override alone is not mirrored the other way.
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

/// The native sample period under a stride as Vision takes it: the application builds the build for the stride with a period of `native x stride`, and Tier 1 divides it again.
fn vision_native_period(native_s: f64, stride: u64) -> f64 {
    let stride = stride.max(1) as f64;
    (native_s * stride) / stride
}

/// A protocol as the reader types it: the two spellings the library names, and any other text as it is.
fn protocol_of(text: &str) -> GalvoProtocol {
    match text {
        "XY2-100" => GalvoProtocol::Xy2100,
        "SL2-100" => GalvoProtocol::Sl2100,
        other => GalvoProtocol::Other(other.to_string()),
    }
}

// ---------------------------------------------------------------- the operations

fn shift(inputs: &Value) -> Columns {
    let timestamps = exact_f64s(inputs, "timestamps");
    let values = exact_f64s(inputs, "values");
    let fit_n = usize::try_from(inputs["fit_n"].as_u64().expect("fit_n")).expect("fit_n");
    let shifted = match shift_channel(
        &timestamps,
        &values,
        exact_f64(inputs, "shift_seconds"),
        fit_n,
    ) {
        Ok(shifted) => shifted,
        // Vision's reading of a refusal: series of different lengths come back as they are
        Err(ShiftRefusal::LengthMismatch { .. }) => values,
        Err(other) => panic!("{other}"),
    };
    let mut out = Columns::new();
    out.insert("len".into(), whole(&[shifted.len() as i64]));
    out.insert("shifted".into(), bits(&shifted));
    out
}

fn parse(inputs: &Value) -> Columns {
    let period = exact_f64(inputs, "sample_period");
    let seconds: Vec<f64> = inputs["texts"]
        .as_array()
        .expect("texts")
        .iter()
        .map(
            |text| match parse_offset_text(text.as_str().expect("text")) {
                ParsedOffset::Steps(steps) => steps * period,
                ParsedOffset::Time { seconds } => seconds,
                // Vision reads an unknown unit as steps, and text that is not a number as 0
                ParsedOffset::UnknownUnit { value, .. } => value * period,
                ParsedOffset::Unparsed => 0.0,
                other => panic!("{other:?}"),
            },
        )
        .collect();
    let mut out = Columns::new();
    out.insert("seconds".into(), bits(&seconds));
    out
}

fn resolve(inputs: &Value) -> Columns {
    let period = exact_f64(inputs, "native_period");
    let user = vision_mirrored(&override_of(&inputs["override"]));
    let commanded = apply_override(
        exact_f64(inputs, "file_commanded_s"),
        user.commanded_offset_steps,
        period,
    );
    let actual = apply_override(
        exact_f64(inputs, "file_actual_s"),
        user.actual_offset_steps,
        period,
    );
    let mut out = Columns::new();
    out.insert("commanded_s".into(), bits(&[commanded.value()]));
    out.insert("actual_s".into(), bits(&[actual.value()]));
    out
}

fn tier1(inputs: &Value) -> Columns {
    let stride = inputs["stride"].as_u64().expect("stride");
    let card_period = exact_f64(inputs, "native_period");
    let recorded = RecordedTiming::new()
        .with_commanded(RecordedOffset::text(
            inputs["commanded_offset"]
                .as_str()
                .expect("commanded_offset"),
        ))
        .with_actual(RecordedOffset::text(
            inputs["actual_offset"].as_str().expect("actual_offset"),
        ))
        .with_protocols(
            Some(protocol_of(
                inputs["protocol_x"].as_str().expect("protocol_x"),
            )),
            Some(protocol_of(
                inputs["protocol_y"].as_str().expect("protocol_y"),
            )),
        )
        .with_native_period_s(vision_native_period(card_period, stride));
    let user = vision_mirrored(&override_of(&inputs["override"]));
    let timing = resolve_timing(&recorded, &user, &vision_fallbacks());

    // the layer: one laser whose samples sit at the native indices given
    let indices = exact_f64s(inputs, "native_indices");
    let native: Vec<u64> = indices.iter().map(|i| *i as u64).collect();
    let times = timestamps(&NativeIndex::Listed(Shared::new(native)), 0.0, card_period);
    let zero = vec![0.0; indices.len()];
    let commanded_z: Vec<f64> = indices.iter().map(|i| 0.5 * i + 1.0).collect();
    let actual_z: Vec<f64> = indices.iter().map(|i| 0.25 * i).collect();
    let series = [
        (
            "commanded_x",
            ChannelKind::CommandedPosition { axis: Axis::X },
            &indices,
        ),
        (
            "commanded_y",
            ChannelKind::CommandedPosition { axis: Axis::Y },
            &zero,
        ),
        (
            "commanded_z",
            ChannelKind::CommandedPosition { axis: Axis::Z },
            &commanded_z,
        ),
        (
            "actual_x",
            ChannelKind::ActualPosition { axis: Axis::X },
            &indices,
        ),
        (
            "actual_y",
            ChannelKind::ActualPosition { axis: Axis::Y },
            &zero,
        ),
        (
            "actual_z",
            ChannelKind::ActualPosition { axis: Axis::Z },
            &actual_z,
        ),
    ];

    let mut out = Columns::new();
    for (name, kind, values) in series {
        let shifted = match shift_series(&timing, &kind, &times, values) {
            Ok(shifted) => shifted.into_values(),
            // Vision's use case does not shift a laser of fewer than two samples
            Err(ShiftRefusal::TooFewSamples { .. }) => values.clone(),
            Err(other) => panic!("{name}: {other}"),
        };
        if matches!(
            name,
            "commanded_x" | "actual_x" | "commanded_z" | "actual_z"
        ) {
            out.insert(name.into(), bits(&shifted));
        }
    }
    // power and state are not shifted: the library refuses to shift them, and they come out as they went in
    let power: Vec<f64> = exact_f64s(inputs, "power")
        .into_iter()
        .map(|p| f64::from(p as f32))
        .collect();
    let state: Vec<i64> = inputs["state"]
        .as_array()
        .expect("state")
        .iter()
        .map(|s| s.as_i64().expect("a state"))
        .collect();
    for kind in [ChannelKind::LaserPower, ChannelKind::LaserState] {
        assert!(matches!(
            shift_series(&timing, &kind, &times, &power),
            Err(ShiftRefusal::NotAPosition { .. })
        ));
    }
    out.insert("power".into(), bits(&power));
    out.insert("state".into(), whole(&state));
    out
}

fn run(inputs: &Value) -> Columns {
    match inputs["operation"].as_str().expect("operation") {
        "shift" => shift(inputs),
        "parse" => parse(inputs),
        "resolve" => resolve(inputs),
        "tier1" => tier1(inputs),
        other => panic!("unknown operation {other:?}"),
    }
}

#[test]
fn the_pure_cases_pass_bit_for_bit() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut ran = 0;
    let mut by_operation = std::collections::BTreeMap::<String, usize>::new();
    let mut lines = Vec::new();
    for case in doc["cases"].as_array().expect("cases") {
        if case["kind"] != "pure" {
            continue;
        }
        let label = case["case"].as_str().expect("case");
        let expected = expected_columns(case);
        let actual = run(&case["inputs"]);
        lines.extend(differences(label, &expected, &actual, &|_| false));
        ran += 1;
        *by_operation
            .entry(
                case["inputs"]["operation"]
                    .as_str()
                    .expect("operation")
                    .to_string(),
            )
            .or_default() += 1;
    }
    assert!(
        lines.is_empty(),
        "the library's time shift differs from Vision's vectors in {} places:\n{}",
        lines.len(),
        lines.join("\n")
    );
    assert_eq!(ran, 67, "the file holds 67 pure cases");
    assert_eq!(by_operation["shift"], 21);
    assert_eq!(by_operation["parse"], 17);
    assert_eq!(by_operation["resolve"], 7);
    assert_eq!(by_operation["tier1"], 22);
}

// ---------------------------------------------------------------- the sign rule has a test of its own (BL-PR-04)

/// The cases of the sign by protocol, read from the vectors, each through the library's `card_convention` **and** through the whole resolution: SL2-100 negates a bare offset, XY2-100 takes it
/// as it is, an XY2 in either galvo counts in any case, and a protocol that is neither is not recognised, so that the caller's answer is used.
#[test]
fn the_sign_by_protocol_is_the_one_the_vectors_record() {
    let doc = load_vectors(CHUNK).expect("the vectors");
    let mut checked = 0;
    for case in doc["cases"].as_array().expect("cases") {
        let label = case["case"].as_str().expect("case");
        if case["kind"] != "pure" || case["inputs"]["operation"] != "tier1" {
            continue;
        }
        let inputs = &case["inputs"];
        // only the cases whose offsets are bare numbers and have no override isolate the sign
        let commanded = inputs["commanded_offset"].as_str().expect("offset");
        let has_override = !inputs["override"].as_object().expect("override").is_empty();
        let bare = commanded.parse::<f64>().is_ok();
        if has_override || !bare {
            continue;
        }
        let x = protocol_of(inputs["protocol_x"].as_str().expect("protocol_x"));
        let y = protocol_of(inputs["protocol_y"].as_str().expect("protocol_y"));
        let recorded = RecordedTiming::new()
            .with_commanded(RecordedOffset::text(commanded))
            .with_protocols(Some(x.clone()), Some(y.clone()))
            .with_native_period_s(1e-5);
        let timing = resolve_timing(&recorded, &TimingOverride::new(), &vision_fallbacks());
        let steps: f64 = commanded.parse().unwrap();
        let text = |p: &GalvoProtocol| match p {
            GalvoProtocol::Xy2100 => "XY2-100".to_string(),
            GalvoProtocol::Sl2100 => "SL2-100".to_string(),
            GalvoProtocol::Other(t) => t.clone(),
            _ => unreachable!(),
        };
        let any_xy2 = [&x, &y]
            .iter()
            .any(|p| text(p).to_uppercase().contains("XY2"));
        let expected = if any_xy2 {
            steps * 1e-5
        } else {
            -(steps * 1e-5)
        };
        assert_eq!(
            timing.commanded().seconds().value().to_bits(),
            expected.to_bits(),
            "{label}"
        );
        assert_eq!(
            timing.sign().value(),
            if any_xy2 { 1.0 } else { -1.0 },
            "{label}"
        );
        checked += 1;
    }
    assert!(
        checked >= 6,
        "the vectors have at least six cases that isolate the sign, found {checked}"
    );
}

#[test]
fn the_constant_of_the_fit_is_vision_s() {
    assert_eq!(SHIFT_FIT_N, 50);
}
