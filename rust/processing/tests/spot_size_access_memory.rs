//! The spot-size accessors on a source in memory (IP-27 phase 27J; decisions D27-18, D27-21 and D27-24; BL-PR-03, BL-PR-04, V:QE-04).
//!
//! The miniatures cannot reach every branch of the accessor: each of their lasers has Z series of the same length, every laser of a layer has both Z channels, and none records a laser outside
//! the layer. A build made here can, and it needs no HDF5, which is the point of reading through `BuildSource`: the accessor runs on any source.
//!
//! **The answers where the file is silent are this test's** (a caller that is not Vision, with other numbers on purpose): the library has no default, so a value the build lacks must come out
//! as the caller's answer, with the origin that says so.

use build_log_model::{
    Axis, BuildIdentity, BuildMeta, ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind,
    ClockClass, ColumnData, EnumLabels, InMemorySource, LaserKey, LaserLayerMeta, LaserMeta,
    LaserParameters, LayerId, NumericType, Origin, Presence, Quantity, ReadContext, ReadSelection,
    Shared, SpotSizeOverride, StateRows, TimingOverride, ZCalibrationOverride,
};
use build_log_processing::{
    spot_size, spot_size_beam, AngleUnit, BeamAssumptions, BeamUnavailable, CardConvention,
    LaserSpotSize, LengthUnit, OneWaistMissing, SpotSizeAssumptions, SpotSizeFinding,
    SpotSizeRequest, SpotSizeUnavailable, Stage, TimingFallbacks, UnitlessPeriod, UnknownUnitAs,
    ZAssumptions, ZSeries,
};

const LAYER: LayerId = LayerId(1);
const OTHER_LAYER: LayerId = LayerId(2);

fn first() -> LaserKey {
    LaserKey::new("Laser_1", 1)
}

fn second() -> LaserKey {
    LaserKey::new("Laser_2", 2)
}

fn q(value: f64, unit: Option<&str>) -> Quantity {
    Quantity::from_file(value, unit)
}

/// The answers of a caller that is not Vision: a field of 200 mm with an offset of 1, 18 bits, a bare period in seconds, a first sample at 0.5 s.
fn callers() -> SpotSizeAssumptions {
    SpotSizeAssumptions {
        beam: BeamAssumptions {
            waist_unit: LengthUnit::Micrometre,
            length_unit: LengthUnit::Millimetre,
            angle_unit: AngleUnit::Degree,
            wavelength_unit: LengthUnit::Nanometre,
            absent_value: 0.0,
            m2_when_none: 1.0,
            one_waist_missing: OneWaistMissing::AssumeCircular,
        },
        timing: TimingFallbacks {
            sample_period_s: 2e-5,
            absent_offset_s: 0.0,
            unparsed_offset_s: 0.0,
            unknown_unit: UnknownUnitAs::Steps,
            unrecognised_protocol: CardConvention::OppositeToShifter,
        },
        z: ZAssumptions {
            field_size_mm: 200.0,
            offset_mm: 1.0,
            bit_resolution: 18,
            narrow_to_stored_width: true,
        },
        sample_period: UnitlessPeriod::Seconds,
        time_origin_s: 0.5,
    }
}

/// A snapshot with a beam (waists, Rayleigh lengths) and no Z calibration, no timing and no period: everything about the Z and the time is for the caller to answer.
fn beam_only() -> LaserParameters {
    let mut p = LaserParameters::new();
    p.optics.waist_major = Some(q(66.0, Some("\u{3bc}m")));
    p.optics.waist_minor = Some(q(68.0, Some("\u{3bc}m")));
    p.optics.rayleigh_length_major = Some(q(3.0, Some("mm")));
    p.optics.rayleigh_length_minor = Some(q(3.2, Some("mm")));
    p
}

/// What a laser has in layer 1, and how long each series is.
#[derive(Clone)]
struct Spec {
    state: Option<usize>,
    commanded: Option<(usize, NumericType)>,
    actual: Option<(usize, NumericType)>,
    parameters: Option<LaserParameters>,
    period: Option<Quantity>,
}

impl Spec {
    /// Both Z series of `n` 32-bit codes and a state of `n` rows, a beam, nothing else.
    fn of(n: usize) -> Spec {
        Spec {
            state: Some(n),
            commanded: Some((n, NumericType::I32)),
            actual: Some((n, NumericType::I32)),
            parameters: Some(beam_only()),
            period: None,
        }
    }
}

/// The codes of a Z series: a ramp, steeper for the actual series than for the commanded one, so that the tracking error is not zero and the spot depends on the calibration.
fn codes(n: usize, kind: NumericType, actual: bool) -> ColumnData {
    match kind {
        NumericType::I16 => {
            let slope = if actual { 53 } else { 37 };
            ColumnData::I16(Shared::new(
                (0..n).map(|i| 1000 + slope * i as i16).collect(),
            ))
        }
        NumericType::I32 => {
            let slope = if actual { 1_700 } else { 1_300 };
            ColumnData::I32(Shared::new(
                (0..n).map(|i| 500_000 + slope * i as i32).collect(),
            ))
        }
        other => panic!("not a type of this test: {other:?}"),
    }
}

fn descriptor(name: &str, kind: ChannelKind, dtype: ChannelDType, n: usize) -> ChannelDescriptor {
    let mut descriptor = ChannelDescriptor::new(
        ChannelKey::new([name]),
        kind,
        ClockClass::LaserSynchronous,
        dtype,
    );
    descriptor
        .presence
        .insert(LAYER, Presence::from_count(n as u64));
    descriptor
}

/// A source of two layers: `Laser_1` in layer 1 as `spec` says, and `Laser_2` in layer 2 only (so it is in the build and not in layer 1).
fn source(spec: &Spec) -> InMemorySource {
    let mut build = BuildMeta::new(BuildIdentity {
        build_id: Some("memory".into()),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    });
    let mut source = InMemorySource::new();
    let mut laser = LaserMeta::new(first());
    laser.parameters = spec.parameters.clone();
    let rows = [
        spec.state,
        spec.commanded.map(|c| c.0),
        spec.actual.map(|a| a.0),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let mut in_layer = LaserLayerMeta::new(Presence::from_count(rows as u64));
    if let Some(period) = &spec.period {
        in_layer.time_axis = in_layer.time_axis.with_period(period.clone());
    }
    laser.layers.insert(LAYER, in_layer);
    if let Some(n) = spec.state {
        laser.channels.push(descriptor(
            "state",
            ChannelKind::LaserState,
            ChannelDType::Enum {
                labels: EnumLabels::Unknown,
            },
            n,
        ));
        source.insert(
            LAYER,
            Some(first()),
            ChannelKey::new(["state"]),
            // on, on, off, on, off, off, on, ...
            ColumnData::U8(Shared::new(
                (0..n).map(|i| u8::from(i % 3 != 2 && i % 5 != 4)).collect(),
            )),
        );
    }
    for (name, kind, series, actual) in [
        (
            "z_cmd",
            ChannelKind::CommandedPosition { axis: Axis::Z },
            spec.commanded,
            false,
        ),
        (
            "z_act",
            ChannelKind::ActualPosition { axis: Axis::Z },
            spec.actual,
            true,
        ),
    ] {
        if let Some((n, numeric)) = series {
            laser
                .channels
                .push(descriptor(name, kind, ChannelDType::Numeric(numeric), n));
            source.insert(
                LAYER,
                Some(first()),
                ChannelKey::new([name]),
                codes(n, numeric, actual),
            );
        }
    }
    build.add_laser(laser);
    // the second laser records something in layer 2 only
    let mut other = LaserMeta::new(second());
    other.parameters = Some(beam_only());
    other
        .layers
        .insert(OTHER_LAYER, LaserLayerMeta::new(Presence::from_count(4)));
    build.add_laser(other);
    build.synthesise_missing_layers();
    source.set_build(build);
    source
}

fn ask(
    source: &InMemorySource,
    request: &SpotSizeRequest,
) -> Result<LaserSpotSize, SpotSizeUnavailable> {
    spot_size(source, request, &ReadContext::new()).expect("a laser and a layer of the build")
}

fn request() -> SpotSizeRequest {
    SpotSizeRequest::new(LAYER, first(), callers())
}

fn bits_of(values: &[f64]) -> Vec<u64> {
    values.iter().map(|v| v.to_bits()).collect()
}

// ---------------------------------------------------------------- what the file does not say is the caller's, and says so

#[test]
fn what_the_build_lacks_is_the_callers_answer_with_the_origin_that_says_so() {
    let source = source(&Spec::of(12));
    let spot = ask(&source, &request()).expect("a spot size");
    let answers = callers();
    // the Z calibration: nothing in the snapshot, so every value is the caller's
    let caller = Some(&Origin::CallerSupplied);
    assert_eq!(spot.z().field_size_mm().value(), answers.z.field_size_mm);
    assert_eq!(spot.z().field_size_mm().origin(), caller);
    assert_eq!(spot.z().offset_mm().value(), answers.z.offset_mm);
    assert_eq!(spot.z().offset_mm().origin(), caller);
    for resolution in [
        spot.z().commanded_resolution(),
        spot.z().actual_resolution(),
    ] {
        assert_eq!(resolution.value(), answers.z.bit_resolution);
        assert_eq!(resolution.origin(), caller);
    }
    // the sample period: none in the file, so the caller's
    assert_eq!(spot.period().value(), answers.timing.sample_period_s);
    assert_eq!(spot.period().origin(), caller);
    // another answer is another spot: the library has nothing of its own to fall back on
    let mut other = answers;
    other.z.field_size_mm = 400.0;
    let again = ask(&source, &SpotSizeRequest::new(LAYER, first(), other)).expect("a spot size");
    assert_eq!(again.z().field_size_mm().value(), 400.0);
    assert_ne!(
        bits_of(&again.actual().major_radii_um),
        bits_of(&spot.actual().major_radii_um)
    );
    // the result says what it is
    assert_eq!(spot.stage(), Stage::Shifted);
    assert_eq!(spot.commanded().major_radii_um.len(), 12);
    assert!(spot.findings().is_empty());
}

#[test]
fn a_value_the_file_has_keeps_its_own_origin_and_an_override_is_the_users() {
    let mut spec = Spec::of(12);
    let mut p = beam_only();
    p.galvo_z.commanded_bit_resolution = Some(24);
    p.galvo_z.actual_bit_resolution = Some(22);
    p.scanner.field_size_z = Some(q(150.0, Some("mm")));
    spec.parameters = Some(p);
    let source = source(&spec);
    let spot = ask(&source, &request()).expect("a spot size");
    // from the file: no origin, the file's number, not the caller's
    assert_eq!(spot.z().field_size_mm().value(), 150.0);
    assert_eq!(spot.z().field_size_mm().origin(), None);
    assert_eq!(spot.z().commanded_resolution().value(), 24);
    assert_eq!(spot.z().commanded_resolution().origin(), None);
    assert_eq!(spot.z().actual_resolution().value(), 22);
    // the offset is not in the file, so it is the caller's
    assert_eq!(spot.z().offset_mm().origin(), Some(&Origin::CallerSupplied));
    // the user's override is over the file's value
    let over = ZCalibrationOverride::new()
        .with_field_size_z_mm(75.0)
        .with_commanded_bit_resolution_z(20);
    let overridden = ask(&source, &request().with_z_override(over)).expect("a spot size");
    assert_eq!(overridden.z().field_size_mm().value(), 75.0);
    assert_eq!(
        overridden.z().field_size_mm().origin(),
        Some(&Origin::Overridden)
    );
    assert_eq!(overridden.z().commanded_resolution().value(), 20);
    assert_eq!(
        overridden.z().commanded_resolution().origin(),
        Some(&Origin::Overridden)
    );
    assert_eq!(
        overridden.z().actual_resolution().value(),
        22,
        "the actual series keeps the file's"
    );
}

#[test]
fn a_dataset_narrower_than_its_calibration_is_converted_at_its_width_when_the_caller_says_so() {
    let mut spec = Spec::of(12);
    spec.commanded = Some((12, NumericType::I16));
    spec.actual = Some((12, NumericType::I16));
    let source = source(&spec);
    let narrowed = ask(&source, &request()).expect("a spot size");
    // the caller's 18 bits are narrowed to the 16 that are stored, and the origin says the caller's rule did it
    for resolution in [
        narrowed.z().commanded_resolution(),
        narrowed.z().actual_resolution(),
    ] {
        assert_eq!(resolution.value(), 16);
        assert_eq!(resolution.origin(), Some(&Origin::CallerSupplied));
    }
    // a caller that does not narrow gets its 18 bits whatever the type
    let mut keep = callers();
    keep.z.narrow_to_stored_width = false;
    let kept = ask(&source, &SpotSizeRequest::new(LAYER, first(), keep)).expect("a spot size");
    assert_eq!(kept.z().commanded_resolution().value(), 18);
    assert_ne!(
        bits_of(&kept.actual().major_radii_um),
        bits_of(&narrowed.actual().major_radii_um)
    );
}

#[test]
fn a_sample_period_is_read_by_its_unit_or_by_the_callers_rule() {
    let read = |period: Quantity, rule: UnitlessPeriod| {
        let mut spec = Spec::of(12);
        spec.period = Some(period);
        let mut answers = callers();
        answers.sample_period = rule;
        let spot = ask(
            &source(&spec),
            &SpotSizeRequest::new(LAYER, first(), answers),
        )
        .expect("a spot size");
        // the period, its origin, and the origin of the native period that the shift used
        (
            spot.period().value(),
            spot.period().origin().cloned(),
            spot.timing().native_period().origin().cloned(),
        )
    };
    let rule = UnitlessPeriod::MicrosecondsAbove(0.001);
    let callers_reading = Some(Origin::CallerSupplied);
    // a unit the file states is the file's, whatever the caller's rule
    assert_eq!(read(q(10.0, Some("us")), rule), (10.0 * 1e-6, None, None));
    assert_eq!(
        read(q(10.0, Some("us")), UnitlessPeriod::Seconds),
        (10.0 * 1e-6, None, None)
    );
    // a bare number is read by the caller's rule, and the origin says so, for the period and for the native period
    assert_eq!(
        read(q(10.0, None), rule),
        (
            10.0 * 1e-6,
            callers_reading.clone(),
            callers_reading.clone()
        )
    );
    assert_eq!(
        read(q(1e-5, None), rule),
        (1e-5, callers_reading.clone(), callers_reading.clone())
    );
    assert_eq!(
        read(q(10.0, None), UnitlessPeriod::Seconds),
        (10.0, callers_reading.clone(), callers_reading.clone())
    );
    assert_eq!(
        read(q(10.0, None), UnitlessPeriod::Microseconds),
        (10.0 * 1e-6, callers_reading.clone(), callers_reading)
    );
}

#[test]
fn the_native_period_under_a_stride_is_visions_and_the_period_of_the_timestamps_is_not() {
    // Vision stores `period x stride` and divides again, which is one unit in the last place off the plain period for some strides (49 is one for 1e-5). Version 1 keeps it, so
    // that the library and Vision agree to the last bit; it is the owner's to decide at phase 27L. The timestamps use the plain period.
    let period: f64 = 1e-5;
    let stride: u64 = 49;
    let quirk = (period * stride as f64) / stride as f64;
    assert_ne!(
        quirk.to_bits(),
        period.to_bits(),
        "49 is a stride where the two differ"
    );
    let mut spec = Spec::of(200);
    spec.period = Some(q(period, Some("s")));
    let spot = ask(
        &source(&spec),
        &request().with_selection(ReadSelection::new().with_stride(stride)),
    )
    .expect("a spot size");
    assert_eq!(
        spot.timing().native_period().value().to_bits(),
        quirk.to_bits()
    );
    assert_eq!(spot.period().value().to_bits(), period.to_bits());
    // a stride of 1 is exact
    let spot = ask(&source(&spec), &request()).expect("a spot size");
    assert_eq!(
        spot.timing().native_period().value().to_bits(),
        period.to_bits()
    );
}

#[test]
fn the_snapshots_period_is_read_before_the_layers_time_axis() {
    let mut spec = Spec::of(12);
    let mut p = beam_only();
    p.scanner_card.sample_period = Some(q(10.0, Some("us")));
    spec.parameters = Some(p);
    spec.period = Some(q(20.0, Some("us")));
    let spot = ask(&source(&spec), &request()).expect("a spot size");
    assert_eq!(spot.period().value(), 10.0 * 1e-6);
    // with no snapshot period the layer's time axis gives it
    spec.parameters = Some(beam_only());
    let spot = ask(&source(&spec), &request()).expect("a spot size");
    assert_eq!(spot.period().value(), 20.0 * 1e-6);
}

#[test]
fn the_time_of_the_first_sample_is_the_callers_for_each_series() {
    // a shift by a fraction of a step interpolates between samples, and the time of each sample carries the first sample's: another origin is another rounding of every timestamp.
    // Each series is shifted by its own offset, so shifting one series alone shows whether that series' timestamps carry the origin.
    let mut spec = Spec::of(40);
    spec.period = Some(q(1e-5, Some("s")));
    let source = source(&spec);
    let at = |origin: f64, overrides: &TimingOverride| {
        let mut answers = callers();
        answers.time_origin_s = origin;
        ask(
            &source,
            &SpotSizeRequest::new(LAYER, first(), answers).with_timing_override(overrides.clone()),
        )
        .expect("a spot size")
    };
    let commanded_alone = TimingOverride::new().with_commanded_offset_steps(2.5);
    let actual_alone = TimingOverride::new().with_actual_offset_steps(2.5);
    for (name, overrides) in [("commanded", commanded_alone), ("actual", actual_alone)] {
        let near = at(0.0, &overrides);
        let far = at(1.0e9, &overrides);
        assert_eq!(near.stage(), Stage::Shifted);
        assert_ne!(
            bits_of(&near.actual().major_radii_um),
            bits_of(&far.actual().major_radii_um),
            "the time origin reaches the {name} series' shift"
        );
    }
}

// ---------------------------------------------------------------- the reasons, in their order

#[test]
fn a_laser_in_the_build_but_not_in_the_layer_is_a_reason_and_a_stranger_is_an_error() {
    let source = source(&Spec::of(12));
    // Laser_2 is in the build and records nothing in layer 1
    let outside = SpotSizeRequest::new(LAYER, second(), callers());
    assert_eq!(
        ask(&source, &outside).err(),
        Some(SpotSizeUnavailable::LaserNotInLayer)
    );
    // a laser that the build does not hold, and a layer that it does not hold, are errors, not reasons
    let stranger = SpotSizeRequest::new(LAYER, LaserKey::new("Laser_9", 9), callers());
    assert!(spot_size(&source, &stranger, &ReadContext::new()).is_err());
    let nowhere = SpotSizeRequest::new(LayerId(77), first(), callers());
    assert!(spot_size(&source, &nowhere, &ReadContext::new()).is_err());
    // the laser itself must be in the build, whichever laser the beam is taken from
    let stranger_key = LaserKey::new("Laser_9", 9);
    let asked_of =
        SpotSizeRequest::new(LAYER, stranger_key.clone(), callers()).with_beam_from(first());
    assert!(spot_size(&source, &asked_of, &ReadContext::new()).is_err());
    let beam_of = |laser: &LaserKey, from: &LaserKey| {
        spot_size_beam(
            &source,
            laser,
            Some(from),
            &SpotSizeOverride::new(),
            &callers().beam,
        )
    };
    assert!(beam_of(&stranger_key, &first()).is_err());
    assert!(beam_of(&first(), &stranger_key).is_err());
}

#[test]
fn a_laser_without_a_z_channel_is_unavailable_and_the_commanded_series_is_named_first() {
    let mut no_actual = Spec::of(12);
    no_actual.actual = None;
    assert_eq!(
        ask(&source(&no_actual), &request()).err(),
        Some(SpotSizeUnavailable::NoZChannel {
            series: ZSeries::Actual
        })
    );
    let mut no_commanded = Spec::of(12);
    no_commanded.commanded = None;
    assert_eq!(
        ask(&source(&no_commanded), &request()).err(),
        Some(SpotSizeUnavailable::NoZChannel {
            series: ZSeries::Commanded
        })
    );
    let mut neither = Spec::of(12);
    neither.commanded = None;
    neither.actual = None;
    assert_eq!(
        ask(&source(&neither), &request()).err(),
        Some(SpotSizeUnavailable::NoZChannel {
            series: ZSeries::Commanded
        })
    );
}

#[test]
fn the_beam_is_settled_before_the_z_is_looked_at() {
    // a laser with no snapshot and no Z: the reason is the beam's, the first check that fails
    let mut spec = Spec::of(12);
    spec.parameters = None;
    spec.commanded = None;
    spec.actual = None;
    assert_eq!(
        ask(&source(&spec), &request()).err(),
        Some(SpotSizeUnavailable::Beam(BeamUnavailable::NoTrain))
    );
    // the metadata accessor gives the same reason without reading anything, and the caller can ask for the beam of another laser
    let source = source(&spec);
    let own = spot_size_beam(
        &source,
        &first(),
        None,
        &SpotSizeOverride::new(),
        &callers().beam,
    )
    .expect("the laser");
    assert_eq!(own.err(), Some(BeamUnavailable::NoTrain));
    let borrowed = spot_size_beam(
        &source,
        &first(),
        Some(&second()),
        &SpotSizeOverride::new(),
        &callers().beam,
    )
    .expect("the lasers");
    assert!(borrowed.is_ok(), "Laser_2 has a snapshot with a beam");
}

#[test]
fn a_z_that_cannot_be_converted_is_refused_before_any_row_is_looked_for() {
    // no rows at all and a calibration that cannot be used: the refusal is the reason
    let mut spec = Spec::of(0);
    spec.state = Some(0);
    let source = source(&spec);
    let refused =
        request().with_z_override(ZCalibrationOverride::new().with_commanded_bit_resolution_z(0));
    assert!(matches!(
        ask(&source, &refused).err(),
        Some(SpotSizeUnavailable::ZRefused {
            series: ZSeries::Commanded,
            ..
        })
    ));
    // with a usable calibration it is the empty laser that is the reason
    assert_eq!(
        ask(&source, &request()).err(),
        Some(SpotSizeUnavailable::NoRows)
    );
    // one series with no rows is enough: a spot needs both
    for (commanded, actual) in [(12, 0), (0, 12)] {
        let mut spec = Spec::of(12);
        spec.commanded = Some((commanded, NumericType::I32));
        spec.actual = Some((actual, NumericType::I32));
        assert_eq!(
            ask(&self::source(&spec), &request()).err(),
            Some(SpotSizeUnavailable::NoRows),
            "{commanded} commanded and {actual} actual rows"
        );
    }
}

#[test]
fn a_filter_on_a_laser_with_no_state_is_a_reason_and_a_stride_alone_is_not() {
    let mut spec = Spec::of(12);
    spec.state = None;
    let source = source(&spec);
    for rows in [StateRows::OnOnly, StateRows::OffOnly] {
        let filtered = request().with_selection(ReadSelection::new().with_rows(rows));
        assert_eq!(
            ask(&source, &filtered).err(),
            Some(SpotSizeUnavailable::FilterNeedsState)
        );
    }
    let strided = request().with_selection(ReadSelection::new().with_stride(5));
    let spot = ask(&source, &strided).expect("a stride needs no state");
    assert_eq!(spot.commanded_rows().to_vec(), vec![0, 5, 10]);
    // a filter that keeps nothing is the empty case, not a failure
    let mut all_off = Spec::of(12);
    all_off.state = Some(2); // rows 0 and 1 are on, so a filter on `off` finds none among the first two... and the Z has more rows than the state
    let source = self::source(&all_off);
    let off = request().with_selection(ReadSelection::new().with_rows(StateRows::OffOnly));
    assert_eq!(ask(&source, &off).err(), Some(SpotSizeUnavailable::NoRows));
}

// ---------------------------------------------------------------- series of different lengths, and a series too short to shift

#[test]
fn z_series_of_different_lengths_each_keep_their_own_rows_and_the_finding_says_so() {
    let mut spec = Spec::of(12);
    spec.actual = Some((8, NumericType::I32));
    let source = source(&spec);
    let spot = ask(&source, &request()).expect("a spot size");
    // each series has its own rows and its own spot, the commanded spot as long as the commanded series
    assert_eq!(
        spot.commanded_rows().to_vec(),
        (0..12).collect::<Vec<u64>>()
    );
    assert_eq!(spot.actual_rows().to_vec(), (0..8).collect::<Vec<u64>>());
    assert_eq!(spot.commanded().major_radii_um.len(), 12);
    assert_eq!(spot.actual().major_radii_um.len(), 8);
    assert_eq!(
        spot.findings(),
        &[SpotSizeFinding::ZLengthsDiffer {
            commanded: 12,
            actual: 8
        }]
    );
    assert_eq!(spot.stage(), Stage::Shifted);
    // the same lengths say nothing
    assert!(ask(&self::source(&Spec::of(12)), &request())
        .expect("a spot size")
        .findings()
        .is_empty());
}

#[test]
fn a_series_of_fewer_than_two_samples_is_not_shifted_and_the_stage_stays_physical() {
    // the actual series has one sample, the commanded one twelve: only the actual is left as it is, and the stage is the lower of the two
    let mut spec = Spec::of(12);
    spec.actual = Some((1, NumericType::I32));
    let source = source(&spec);
    let spot = ask(&source, &request()).expect("a spot size");
    assert_eq!(spot.stage(), Stage::Physical);
    assert!(spot.findings().contains(&SpotSizeFinding::NotShifted {
        series: ZSeries::Actual,
        samples: 1
    }));
    assert!(!spot.findings().iter().any(|f| matches!(
        f,
        SpotSizeFinding::NotShifted {
            series: ZSeries::Commanded,
            ..
        }
    )));
    assert!(spot.findings().contains(&SpotSizeFinding::ZLengthsDiffer {
        commanded: 12,
        actual: 1
    }));
    // the other way round
    let mut spec = Spec::of(12);
    spec.commanded = Some((1, NumericType::I32));
    let spot = ask(&self::source(&spec), &request()).expect("a spot size");
    assert_eq!(spot.stage(), Stage::Physical);
    assert!(spot.findings().contains(&SpotSizeFinding::NotShifted {
        series: ZSeries::Commanded,
        samples: 1
    }));
}

#[test]
fn a_selection_cuts_every_series_by_the_same_rows() {
    let mut spec = Spec::of(30);
    spec.actual = Some((22, NumericType::I32));
    let source = source(&spec);
    let strided = ask(
        &source,
        &request().with_selection(ReadSelection::new().with_stride(4)),
    )
    .expect("a spot size");
    // every fourth native row below each series' own length
    assert_eq!(
        strided.commanded_rows().to_vec(),
        (0..30).step_by(4).collect::<Vec<u64>>()
    );
    assert_eq!(
        strided.actual_rows().to_vec(),
        (0..22).step_by(4).collect::<Vec<u64>>()
    );
    // a filter chooses its rows once, from the state, and both series are cut with them
    let on = ask(
        &source,
        &request().with_selection(ReadSelection::new().with_rows(StateRows::OnOnly)),
    )
    .expect("a spot size");
    let commanded = on.commanded_rows().to_vec();
    let actual = on.actual_rows().to_vec();
    assert!(!commanded.is_empty() && commanded.len() > actual.len());
    assert_eq!(
        actual,
        commanded
            .iter()
            .copied()
            .filter(|r| *r < 22)
            .collect::<Vec<u64>>()
    );
}
