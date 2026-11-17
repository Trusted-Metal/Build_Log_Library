//! The Z calibration of a laser, as the file and the user give it (IP-27 phase 27D): the user's override over the file's value, each value with where it came from, and **an absent value
//! reported as absent**.
//!
//! * **Layer:** Processing. It works on Model values only (the parameter snapshot, the channel descriptors and the user's overrides), so it runs without HDF5.
//! * **Requirement:** BL-PR-03 (provenance: every produced value says where it came from).
//!
//! # What is resolved
//!
//! | Value | The order of the sources | Without any |
//! |---|---|---|
//! | the Z **field size** (mm) | the user's override, then the file | absent |
//! | the Z **offset** (mm) | the file (**there is no override**, decision D27-18) | absent |
//! | the **bit resolution** of the commanded and of the actual Z | the user's override, then the file | absent |
//!
//! The numbers are read as millimetres and bits whatever unit the file states for them, as Vision reads them; a unit that a file states is in the parameter snapshot for anyone who wants to
//! check it, and the library adds no check of its own.
//!
//! # Where a value came from
//!
//! A [`Resolved`] value carries its [`Origin`]: `None` is the file's own value as recorded (a version's own origin, such as the one value that a V16 file holds for every axis, is kept), and
//! [`Origin::Overridden`] is the user's.
//!
//! # What is not here, and why: the library reports, and Vision decides (decision D27-18, clarified on 2026-11-17)
//!
//! **There are no defaults here.** A value that neither the user nor the file gave is **absent**, and what to do about it (Vision uses 400 mm, 0 mm and 20 bits) is Vision's. The same holds for the
//! two other rules that Vision applies to this calibration: the **narrower-dataset rule** (a series is converted at the width of its stored type when that is smaller than the calibrated
//! resolution) and the **reading of a sample period** whose unit the file does not state. They are policies over the data, not the data: the library gives what it alone has (the stored
//! width of each Z channel, [`StoredWidths`]; the sample period as recorded, in the snapshot) and **a caller passes explicit numbers to the conversion** ([`crate::z_conversion::ZConverter`]).
//! The only checks the library makes are the ones that report a refusal: a number that is not finite, a resolution outside `1..=32`.

use build_log_model::{
    Axis, ChannelDType, ChannelDescriptor, ChannelKind, LaserParameters, NumericType, Origin,
    Quantity, ZCalibrationOverride,
};

/// A resolved value, and where it came from.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved<T> {
    value: T,
    origin: Option<Origin>,
}

impl<T: Copy> Resolved<T> {
    pub(crate) fn new(value: T, origin: Option<Origin>) -> Self {
        Resolved { value, origin }
    }

    /// The value, in the unit this module says (millimetres, bits).
    pub fn value(&self) -> T {
        self.value
    }

    /// Where the value came from; `None` is the file's own value, as recorded.
    pub fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }
}

/// The stored width, in bits, of each Z dataset of a laser in a layer: what the type of the channel says (`int32` is 32, `int16` is 16, a float is the width of the float). **This is data
/// that only the library has** (the channel's stored type); whether a series is converted at that width is Vision's rule.
// constructed-by: library
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredWidths {
    /// The width of the commanded Z, if the laser has that channel.
    pub commanded: Option<u32>,
    /// The width of the actual Z, if the laser has that channel.
    pub actual: Option<u32>,
}

/// The width of a stored type in bits: that of a number, and none for anything else.
fn width_of(dtype: &ChannelDType) -> Option<u32> {
    let ChannelDType::Numeric(number) = dtype else {
        return None;
    };
    Some(match number {
        NumericType::I8 | NumericType::U8 => 8,
        NumericType::I16 | NumericType::U16 => 16,
        NumericType::I32 | NumericType::U32 | NumericType::F32 => 32,
        NumericType::I64 | NumericType::U64 | NumericType::F64 => 64,
        // a stored type that the Model adds later has no width until it is taught here
        _ => return None,
    })
}

impl StoredWidths {
    pub fn new(commanded: Option<u32>, actual: Option<u32>) -> Self {
        StoredWidths { commanded, actual }
    }

    /// The widths of the Z channels among a laser's channels (`BuildSource::layer_meta` gives them for a layer).
    pub fn of_channels(channels: &[ChannelDescriptor]) -> Self {
        let width = |kind: ChannelKind| {
            channels
                .iter()
                .find(|c| c.kind == kind)
                .and_then(|c| width_of(&c.dtype))
        };
        StoredWidths {
            commanded: width(ChannelKind::CommandedPosition { axis: Axis::Z }),
            actual: width(ChannelKind::ActualPosition { axis: Axis::Z }),
        }
    }
}

/// The calibration of a laser's Z as the file and the user give it: every value with its origin, or absent. **Nothing is defaulted and nothing is refused here.**
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct ZCalibration {
    field_size_mm: Option<Resolved<f64>>,
    offset_mm: Option<Resolved<f64>>,
    commanded_resolution: Option<Resolved<i64>>,
    actual_resolution: Option<Resolved<i64>>,
}

impl ZCalibration {
    /// The Z field size, in millimetres, or `None` if neither the user nor the file gave one.
    pub fn field_size_mm(&self) -> Option<&Resolved<f64>> {
        self.field_size_mm.as_ref()
    }

    /// The Z offset, in millimetres, or `None` if the file has none. There is no override of it.
    pub fn offset_mm(&self) -> Option<&Resolved<f64>> {
        self.offset_mm.as_ref()
    }

    /// The calibrated bit resolution of the commanded Z series, or `None`. **Not checked here:** a value outside `1..=32` is for [`crate::z_conversion::ZConverter::new`] to refuse.
    pub fn commanded_resolution(&self) -> Option<&Resolved<i64>> {
        self.commanded_resolution.as_ref()
    }

    /// The calibrated bit resolution of the actual Z series, or `None`; not checked here either.
    pub fn actual_resolution(&self) -> Option<&Resolved<i64>> {
        self.actual_resolution.as_ref()
    }
}

/// The file's origin of a named value of the snapshot, if the adapter recorded one.
fn file_origin(parameters: Option<&LaserParameters>, name: &str) -> Option<Origin> {
    parameters.and_then(|p| p.origins.get(name).cloned())
}

/// A quantity read as the number Vision reads: the value, whatever unit the file states.
fn number(quantity: &Option<Quantity>) -> Option<f64> {
    quantity.as_ref().map(|q| q.value)
}

/// The user's value if there is one, else the file's, else absent.
fn resolve<T: Copy>(
    overridden: Option<T>,
    from_file: Option<T>,
    origin_of_file: impl FnOnce() -> Option<Origin>,
) -> Option<Resolved<T>> {
    match (overridden, from_file) {
        (Some(value), _) => Some(Resolved::new(value, Some(Origin::Overridden))),
        (None, Some(value)) => Some(Resolved::new(value, origin_of_file())),
        (None, None) => None,
    }
}

/// The Z calibration of a laser: the user's overrides over the file's values (`parameters`, `None` for a laser with no snapshot). **A value that neither gives is absent**, and nothing is
/// defaulted, narrowed or refused here (see the module documentation).
pub fn resolve_z_calibration(
    parameters: Option<&LaserParameters>,
    overrides: &ZCalibrationOverride,
) -> ZCalibration {
    ZCalibration {
        field_size_mm: resolve(
            overrides.field_size_z_mm,
            parameters.and_then(|p| number(&p.scanner.field_size_z)),
            || file_origin(parameters, "scanner.field_size_z"),
        ),
        // the offset has no override: the file's value, or absent
        offset_mm: resolve(
            None,
            parameters.and_then(|p| number(&p.scanner.offset_z)),
            || file_origin(parameters, "scanner.offset_z"),
        ),
        commanded_resolution: resolve(
            overrides.commanded_bit_resolution_z,
            parameters.and_then(|p| p.galvo_z.commanded_bit_resolution),
            || file_origin(parameters, "galvo_z.commanded_bit_resolution"),
        ),
        actual_resolution: resolve(
            overrides.actual_bit_resolution_z,
            parameters.and_then(|p| p.galvo_z.actual_bit_resolution),
            || file_origin(parameters, "galvo_z.actual_bit_resolution"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use build_log_model::{ChannelKey, ClockClass};

    fn parameters(
        field: Option<f64>,
        offset: Option<f64>,
        commanded: Option<i64>,
        actual: Option<i64>,
    ) -> LaserParameters {
        let mut p = LaserParameters::new();
        p.scanner.field_size_z = field.map(|v| Quantity::from_file(v, Some("mm")));
        p.scanner.offset_z = offset.map(|v| Quantity::from_file(v, Some("mm")));
        p.galvo_z.commanded_bit_resolution = commanded;
        p.galvo_z.actual_bit_resolution = actual;
        p
    }

    fn none() -> ZCalibrationOverride {
        ZCalibrationOverride::new()
    }

    #[test]
    fn a_laser_with_no_snapshot_has_nothing_and_the_library_makes_nothing_up() {
        let c = resolve_z_calibration(None, &none());
        assert_eq!(c.field_size_mm(), None);
        assert_eq!(c.offset_mm(), None);
        assert_eq!(c.commanded_resolution(), None);
        assert_eq!(c.actual_resolution(), None);
        // a snapshot that holds none of the values is the same
        let c = resolve_z_calibration(Some(&parameters(None, None, None, None)), &none());
        assert_eq!(c.field_size_mm(), None);
        assert_eq!(c.actual_resolution(), None);
    }

    #[test]
    fn the_files_values_are_the_files_and_have_no_origin() {
        let p = parameters(Some(76.5), Some(-1.0), Some(18), Some(16));
        let c = resolve_z_calibration(Some(&p), &none());
        let field = c.field_size_mm().expect("the file has one");
        assert_eq!((field.value(), field.origin()), (76.5, None));
        let offset = c.offset_mm().expect("the file has one");
        assert_eq!((offset.value(), offset.origin()), (-1.0, None));
        let commanded = c.commanded_resolution().expect("the file has one");
        assert_eq!((commanded.value(), commanded.origin()), (18, None));
        assert_eq!(c.actual_resolution().map(Resolved::value), Some(16));
    }

    #[test]
    fn a_value_the_file_lacks_is_absent_next_to_one_that_it_has() {
        // only the field size is in the file: the rest is absent, not zero and not a default
        let c = resolve_z_calibration(Some(&parameters(Some(76.5), None, None, None)), &none());
        assert_eq!(c.field_size_mm().map(Resolved::value), Some(76.5));
        assert_eq!(c.offset_mm(), None);
        assert_eq!(c.commanded_resolution(), None);
    }

    #[test]
    fn a_version_s_own_origin_of_a_file_value_is_kept() {
        let mut p = parameters(None, None, Some(20), Some(20));
        p.origins.record(
            "galvo_z.commanded_bit_resolution",
            Origin::Other(build_log_model::BIT_RESOLUTION_GLOBAL.to_string()),
        );
        let c = resolve_z_calibration(Some(&p), &none());
        assert_eq!(
            c.commanded_resolution().and_then(Resolved::origin),
            Some(&Origin::Other(
                build_log_model::BIT_RESOLUTION_GLOBAL.to_string()
            ))
        );
        // the actual one has no origin recorded, so it is the file's plainly
        assert_eq!(c.actual_resolution().and_then(Resolved::origin), None);
    }

    #[test]
    fn the_users_override_replaces_the_files_value_for_the_field_size_and_both_resolutions() {
        let p = parameters(Some(76.5), Some(-1.0), Some(20), Some(20));
        let ovr = ZCalibrationOverride::new()
            .with_field_size_z_mm(100.0)
            .with_commanded_bit_resolution_z(16)
            .with_actual_bit_resolution_z(18);
        let c = resolve_z_calibration(Some(&p), &ovr);
        let field = c.field_size_mm().expect("overridden");
        assert_eq!(field.value(), 100.0);
        assert_eq!(field.origin(), Some(&Origin::Overridden));
        let commanded = c.commanded_resolution().expect("overridden");
        assert_eq!(commanded.value(), 16);
        assert_eq!(commanded.origin(), Some(&Origin::Overridden));
        assert_eq!(c.actual_resolution().map(Resolved::value), Some(18));
        // there is no override of the offset: it stays the file's, whatever else is overridden
        let offset = c.offset_mm().expect("the file has one");
        assert_eq!((offset.value(), offset.origin()), (-1.0, None));
    }

    #[test]
    fn an_override_is_a_value_even_where_the_file_has_none() {
        let ovr = ZCalibrationOverride::new().with_field_size_z_mm(100.0);
        let c = resolve_z_calibration(None, &ovr);
        assert_eq!(c.field_size_mm().map(Resolved::value), Some(100.0));
        assert_eq!(
            c.field_size_mm().and_then(Resolved::origin),
            Some(&Origin::Overridden)
        );
        assert_eq!(c.offset_mm(), None);
    }

    #[test]
    fn a_resolution_that_makes_no_sense_is_resolved_as_it_is_and_never_refused_here() {
        let ovr = ZCalibrationOverride::new()
            .with_commanded_bit_resolution_z(0)
            .with_actual_bit_resolution_z(-4);
        let c = resolve_z_calibration(None, &ovr);
        assert_eq!(c.commanded_resolution().map(Resolved::value), Some(0));
        assert_eq!(c.actual_resolution().map(Resolved::value), Some(-4));
    }

    #[test]
    fn a_unit_the_file_states_is_not_applied_and_not_checked() {
        // a field size written in metres is read as the number it is: the library adds no check (the unit is in the snapshot for anyone who wants one)
        let mut p = LaserParameters::new();
        p.scanner.field_size_z = Some(Quantity::from_file(0.4, Some("m")));
        let c = resolve_z_calibration(Some(&p), &none());
        assert_eq!(c.field_size_mm().map(Resolved::value), Some(0.4));
    }

    #[test]
    fn the_stored_width_of_a_z_channel_comes_from_its_type() {
        let channel = |kind: ChannelKind, number: NumericType| {
            ChannelDescriptor::new(
                ChannelKey::new(["z"]),
                kind,
                ClockClass::LaserSynchronous,
                ChannelDType::Numeric(number),
            )
        };
        let channels = vec![
            channel(
                ChannelKind::CommandedPosition { axis: Axis::Z },
                NumericType::I16,
            ),
            channel(
                ChannelKind::ActualPosition { axis: Axis::Z },
                NumericType::I32,
            ),
            // an X channel is not Z
            channel(
                ChannelKind::CommandedPosition { axis: Axis::X },
                NumericType::I8,
            ),
        ];
        assert_eq!(
            StoredWidths::of_channels(&channels),
            StoredWidths::new(Some(16), Some(32))
        );
        // a laser with no Z channel has no width, and a type that is not a number has none
        assert_eq!(StoredWidths::of_channels(&[]), StoredWidths::default());
        assert_eq!(width_of(&ChannelDType::Text), None);
        assert_eq!(width_of(&ChannelDType::Numeric(NumericType::F64)), Some(64));
        assert_eq!(width_of(&ChannelDType::Numeric(NumericType::U8)), Some(8));
    }
}
