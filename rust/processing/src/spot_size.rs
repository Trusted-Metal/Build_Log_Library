//! The spot size of a laser at every sample: the defocus and the Gaussian-beam kernel (IP-27 phase 27I, decisions D27-6 and D27-14; algorithm version 1).
//!
//! * **Layer:** Processing. It works on plain numbers and on the resolved beam of [`crate::beam`], so it runs without HDF5.
//! * **Requirement:** BL-PR-03 (the result says the algorithm version and the beam it came from), BL-PR-04 (the arithmetic is fixed once, documented and tested), BL-PR-07 (the vectors are the
//!   golden file, and the checksum guard fails when one changes without it).
//!
//! # What it does (algorithm version 1: Vision's CPU `f64` path, bit for bit)
//!
//! The Z axis is a dynamic-focus command that follows the field curvature, so **the commanded Z is in focus by definition**: the commanded spot is displaced from the waist only by the beam's focus
//! shift, and **the commanded Z values are not read**. The actual spot also carries the Z tracking error.
//!
//! 1. **The defocus**, the axial distance from the beam waist in millimetres. [`commanded_defocus`] is a constant `-focus_shift` at each commanded sample (**a focus shift of 0 gives `-0.0`**: the sign
//!    of zero is kept). [`actual_defocus`] at sample *i* is the tracking error `actual_z[i] - commanded_z[i]` minus the focus shift; the tracking error is **0 where either Z does not exist or the
//!    difference is not finite** (the test is on the difference, so `1e308 - (-1e308)`, which overflows, has none).
//! 2. **The kernel**, per sample and per axis: `w(dz) = w0 * sqrt(1 + (dz / zR)^2)`, except that a Rayleigh length with `|zR| <= f64::EPSILON` leaves the radius at the waist radius; and the area is
//!    `PI * w_major * w_minor` (multiplied in that order). The sign of the defocus never reaches the output (it is squared).
//! 3. **The counts decide the lengths**, not the slices: a longer slice is cut, a shorter one leaves samples with no tracking error, and the commanded and the actual counts may differ.
//!
//! Units: Z in millimetres, radii in micrometres, areas in square micrometres, angles in degrees. A NaN or an infinity in a Z pair gives a tracking error of 0 and so a finite spot.
//!
//! # What is here, and what is not
//!
//! * [`compute_spot_size`] is Vision's `compute_spot_size` with its CPU backend: a [`BeamShape`] and a [`SpotKernelInput`] in, the commanded and the actual [`SpotSeries`] and the angle out.
//! * [`spot_size_from_z`] composes it with a [`ResolvedBeam`] and the **converted and shifted** Z slices ([`crate::z_conversion`], [`crate::time_shift`]) and returns a [`SpotSize`] that says the
//!   algorithm version and the beam, with each value's origin and the findings.
//! * **The GPU `f32` backend is not here:** it is not an oracle (D26-7), and what a user sees today. Whether the library's CPU kernel replaces it is decided at phase 27L (Q27-3).

use std::f64::consts::PI;

use crate::beam::ResolvedBeam;

/// The version of the algorithm of this module (and of the beam resolver and the stages it composes). A change that alters a result is a new version, with regenerated vectors (decision D27-6).
pub const SPOT_SIZE_ALGORITHM_VERSION: u32 = 1;

/// The numbers of a beam that the kernel reads: the waist radii in micrometres, the Rayleigh lengths and the focus shift in millimetres, and the axis angle in degrees.
// constructed-by: caller-options
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamShape {
    /// The waist radius along the major axis, in micrometres.
    pub waist_radius_major_um: f64,
    /// The waist radius along the minor axis, in micrometres.
    pub waist_radius_minor_um: f64,
    /// The Rayleigh length along the major axis, in millimetres.
    pub rayleigh_major_mm: f64,
    /// The Rayleigh length along the minor axis, in millimetres.
    pub rayleigh_minor_mm: f64,
    /// The distance between the commanded focus and the beam waist, in millimetres.
    pub focus_shift_mm: f64,
    /// The orientation of the major axis in the build frame, in `[0, 180)` degrees.
    pub axis_angle_deg: f64,
}

/// The spot radii and the area at each sample. The three lists are the same length.
// constructed-by: library
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpotSeries {
    /// The major-axis radius, in micrometres.
    pub major_radii_um: Vec<f64>,
    /// The minor-axis radius, in micrometres.
    pub minor_radii_um: Vec<f64>,
    /// The elliptical area, in square micrometres.
    pub areas_um2: Vec<f64>,
}

impl SpotSeries {
    /// How many samples.
    pub fn len(&self) -> usize {
        self.major_radii_um.len()
    }

    pub fn is_empty(&self) -> bool {
        self.major_radii_um.is_empty()
    }
}

/// The Z of one laser, as the kernel reads it: the counts of the two series and the Z in millimetres of each (converted and shifted).
#[derive(Debug, Clone, Copy)]
pub struct SpotKernelInput<'a> {
    /// How many commanded samples (the length of the commanded series).
    pub commanded_samples: usize,
    /// How many actual samples (the length of the actual series).
    pub actual_samples: usize,
    /// The commanded Z in millimetres. **Not read** by the commanded spot: it is only the other half of the tracking error.
    pub commanded_z_mm: &'a [f64],
    /// The actual Z in millimetres.
    pub actual_z_mm: &'a [f64],
}

/// The commanded and the actual spot of one laser, and the major-axis angle.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct SpotSizeResult {
    pub commanded: SpotSeries,
    pub actual: SpotSeries,
    /// The orientation of the major axis in the build frame, in `[0, 180)` degrees.
    pub axis_angle_deg: f64,
}

/// The axial distance from the beam waist, in millimetres, of the commanded spot: `-focus_shift` at each of `n` samples. **A focus shift of 0 gives `-0.0`.**
#[must_use]
pub fn commanded_defocus(n: usize, focus_shift_mm: f64) -> Vec<f64> {
    vec![-focus_shift_mm; n]
}

/// The axial distance from the beam waist, in millimetres, of the actual spot at each of `n` samples: the Z tracking error (`actual - commanded`) minus the focus shift. A sample with no Z pair, or
/// whose difference is not finite, has no tracking error.
#[must_use]
pub fn actual_defocus(
    n: usize,
    actual_z_mm: &[f64],
    commanded_z_mm: &[f64],
    focus_shift_mm: f64,
) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let tracking = match (actual_z_mm.get(i), commanded_z_mm.get(i)) {
                (Some(a), Some(c)) if (a - c).is_finite() => a - c,
                _ => 0.0,
            };
            tracking - focus_shift_mm
        })
        .collect()
}

/// The radius at an axial distance `dz` from the waist: `w0 * sqrt(1 + (dz / zR)^2)`, or the waist radius where `|zR| <= f64::EPSILON`.
fn radius(w0: f64, zr: f64, dz: f64) -> f64 {
    if zr.abs() > f64::EPSILON {
        w0 * (1.0 + (dz / zr).powi(2)).sqrt()
    } else {
        w0
    }
}

/// The spot radii and the area at each axial distance from the waist (millimetres), one entry for each element of `defocus_mm`.
#[must_use]
pub fn propagate(defocus_mm: &[f64], beam: &BeamShape) -> SpotSeries {
    let n = defocus_mm.len();
    let mut major = Vec::with_capacity(n);
    let mut minor = Vec::with_capacity(n);
    let mut areas = Vec::with_capacity(n);
    for &dz in defocus_mm {
        let r_major = radius(beam.waist_radius_major_um, beam.rayleigh_major_mm, dz);
        let r_minor = radius(beam.waist_radius_minor_um, beam.rayleigh_minor_mm, dz);
        major.push(r_major);
        minor.push(r_minor);
        areas.push(PI * r_major * r_minor);
    }
    SpotSeries {
        major_radii_um: major,
        minor_radii_um: minor,
        areas_um2: areas,
    }
}

/// The commanded and the actual spot of one laser: the defocus of each ([`commanded_defocus`], [`actual_defocus`]) and the kernel on it ([`propagate`]). **Vision's `compute_spot_size` with its CPU
/// backend, to the last bit.**
#[must_use]
pub fn compute_spot_size(beam: &BeamShape, request: &SpotKernelInput<'_>) -> SpotSizeResult {
    let commanded = commanded_defocus(request.commanded_samples, beam.focus_shift_mm);
    let actual = actual_defocus(
        request.actual_samples,
        request.actual_z_mm,
        request.commanded_z_mm,
        beam.focus_shift_mm,
    );
    SpotSizeResult {
        commanded: propagate(&commanded, beam),
        actual: propagate(&actual, beam),
        axis_angle_deg: beam.axis_angle_deg,
    }
}

/// The spot size of a laser: the series, the angle, the version of the algorithm and the beam that it came from.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
pub struct SpotSize {
    result: SpotSizeResult,
    beam: ResolvedBeam,
}

impl SpotSize {
    /// The spot at each commanded sample.
    pub fn commanded(&self) -> &SpotSeries {
        &self.result.commanded
    }

    /// The spot at each actual sample.
    pub fn actual(&self) -> &SpotSeries {
        &self.result.actual
    }

    /// The orientation of the major axis in the build frame, in `[0, 180)` degrees.
    pub fn axis_angle_deg(&self) -> f64 {
        self.result.axis_angle_deg
    }

    /// The beam the spot was calculated from, with the origin of each value and what was found when it was resolved.
    pub fn beam(&self) -> &ResolvedBeam {
        &self.beam
    }

    /// The version of the algorithm: [`SPOT_SIZE_ALGORITHM_VERSION`].
    pub fn algorithm_version(&self) -> u32 {
        SPOT_SIZE_ALGORITHM_VERSION
    }
}

/// The spot size of a laser from its resolved beam and its Z in millimetres (**converted and shifted**: the caller has applied [`crate::z_conversion`] and [`crate::time_shift`]). The counts are the
/// lengths of the two slices.
#[must_use]
pub fn spot_size_from_z(
    beam: &ResolvedBeam,
    commanded_z_mm: &[f64],
    actual_z_mm: &[f64],
) -> SpotSize {
    let request = SpotKernelInput {
        commanded_samples: commanded_z_mm.len(),
        actual_samples: actual_z_mm.len(),
        commanded_z_mm,
        actual_z_mm,
    };
    SpotSize {
        result: compute_spot_size(&beam.shape(), &request),
        beam: beam.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beam::{resolve_beam, BeamAssumptions, BeamInput, OneWaistMissing};
    use crate::beam_units::{AngleUnit, LengthUnit};
    use build_log_model::SpotSizeOverride;

    /// The sample build's beam: radii of 33 and 34 um and a Rayleigh length of 3 mm, in focus.
    fn sample() -> BeamShape {
        BeamShape {
            waist_radius_major_um: 33.0,
            waist_radius_minor_um: 34.0,
            rayleigh_major_mm: 3.0,
            rayleigh_minor_mm: 3.0,
            focus_shift_mm: 0.0,
            axis_angle_deg: 0.0,
        }
    }

    /// Z as a focus command: about -9.5 mm, with a tracking error of micrometres.
    fn z() -> (Vec<f64>, Vec<f64>) {
        let commanded: Vec<f64> = (0..8).map(|i| -9.5 + 0.001 * f64::from(i)).collect();
        let actual = commanded
            .iter()
            .enumerate()
            .map(|(i, c)| c + 0.0005 * (i as f64 - 3.0))
            .collect();
        (commanded, actual)
    }

    fn resolved(shift_mm: f64) -> ResolvedBeam {
        let mut input = BeamInput::default();
        input.waist_major_um = 66.0;
        input.waist_minor_um = 68.0;
        input.rayleigh_major_mm = 3.0;
        input.rayleigh_minor_mm = 3.0;
        input.thermal_shift_mm = shift_mm;
        let assumptions = BeamAssumptions {
            waist_unit: LengthUnit::Micrometre,
            length_unit: LengthUnit::Millimetre,
            angle_unit: AngleUnit::Degree,
            wavelength_unit: LengthUnit::Nanometre,
            absent_value: 0.0,
            m2_when_none: 1.0,
            one_waist_missing: OneWaistMissing::AssumeCircular,
        };
        resolve_beam(&input, &SpotSizeOverride::new(), &assumptions).expect("a beam")
    }

    #[test]
    fn the_radius_at_zero_defocus_is_the_waist_radius() {
        for zero in [0.0, -0.0] {
            let series = propagate(&[zero], &sample());
            assert_eq!(series.major_radii_um, vec![33.0]);
            assert_eq!(series.minor_radii_um, vec![34.0]);
        }
    }

    #[test]
    fn the_spot_is_symmetric_in_the_defocus_and_grows_with_its_size() {
        let beam = sample();
        let sizes = [0.0, 0.01, 0.1, 0.5, 1.0, 3.0, 10.0, 100.0];
        let mut last = (0.0_f64, 0.0_f64);
        for dz in sizes {
            let up = propagate(&[dz], &beam);
            let down = propagate(&[-dz], &beam);
            assert_eq!(up, down, "{dz}");
            let now = (up.major_radii_um[0], up.minor_radii_um[0]);
            assert!(now.0 >= last.0 && now.1 >= last.1, "{dz}");
            last = now;
        }
        // strictly, once the defocus is not nothing
        assert!(
            propagate(&[1.0], &beam).major_radii_um[0] > propagate(&[0.5], &beam).major_radii_um[0]
        );
    }

    #[test]
    fn the_area_is_pi_times_the_two_radii_in_that_order() {
        let beam = BeamShape {
            rayleigh_major_mm: 2.0,
            rayleigh_minor_mm: 5.0,
            ..sample()
        };
        let series = propagate(&[0.0, 0.7, -1.3, 4.0], &beam);
        for i in 0..series.len() {
            assert_eq!(
                series.areas_um2[i],
                PI * series.major_radii_um[i] * series.minor_radii_um[i]
            );
        }
    }

    #[test]
    fn nothing_in_gives_nothing_out_and_the_counts_decide_the_lengths() {
        let beam = sample();
        assert!(propagate(&[], &beam).is_empty());
        let empty = compute_spot_size(
            &beam,
            &SpotKernelInput {
                commanded_samples: 0,
                actual_samples: 0,
                commanded_z_mm: &[],
                actual_z_mm: &[],
            },
        );
        assert!(empty.commanded.is_empty() && empty.actual.is_empty());
        // the counts, not the slices: a longer slice is cut, a shorter one leaves samples with no tracking error
        let (commanded, actual) = z();
        let result = compute_spot_size(
            &beam,
            &SpotKernelInput {
                commanded_samples: 3,
                actual_samples: 10,
                commanded_z_mm: &commanded,
                actual_z_mm: &actual,
            },
        );
        assert_eq!(result.commanded.len(), 3);
        assert_eq!(result.actual.len(), 10);
        let defocus = actual_defocus(10, &actual, &commanded, 0.0);
        assert_eq!(&defocus[8..], &[0.0, 0.0], "no Z at samples 8 and 9");
    }

    #[test]
    fn a_z_pair_that_is_not_finite_gives_a_finite_spot() {
        let (mut commanded, mut actual) = z();
        actual[2] = f64::NAN;
        commanded[4] = f64::INFINITY;
        actual[6] = f64::NEG_INFINITY;
        commanded[7] = f64::NAN;
        let beam = BeamShape {
            focus_shift_mm: 0.4,
            ..sample()
        };
        let result = compute_spot_size(
            &beam,
            &SpotKernelInput {
                commanded_samples: 8,
                actual_samples: 8,
                commanded_z_mm: &commanded,
                actual_z_mm: &actual,
            },
        );
        for series in [&result.commanded, &result.actual] {
            for value in series
                .major_radii_um
                .iter()
                .chain(&series.minor_radii_um)
                .chain(&series.areas_um2)
            {
                assert!(value.is_finite(), "{value}");
            }
        }
        // two finite values whose difference overflows have no tracking error either
        let defocus = actual_defocus(1, &[1e308], &[-1e308], 0.0);
        assert_eq!(defocus, vec![0.0]);
    }

    #[test]
    fn the_commanded_defocus_is_the_focus_shift_negated_and_a_zero_shift_keeps_its_sign() {
        assert_eq!(commanded_defocus(3, 1.5), vec![-1.5; 3]);
        assert_eq!(commanded_defocus(2, -0.75), vec![0.75; 2]);
        for value in commanded_defocus(2, 0.0) {
            assert_eq!(value.to_bits(), (-0.0_f64).to_bits());
        }
        // the commanded Z values are not read: a layer of NaN gives the same commanded spot
        let beam = BeamShape {
            focus_shift_mm: 0.5,
            ..sample()
        };
        let (_, actual) = z();
        let request = |commanded: &[f64]| {
            compute_spot_size(
                &beam,
                &SpotKernelInput {
                    commanded_samples: 8,
                    actual_samples: 8,
                    commanded_z_mm: commanded,
                    actual_z_mm: &actual,
                },
            )
            .commanded
        };
        assert_eq!(request(&[f64::NAN; 8]), request(&z().0));
    }

    #[test]
    fn a_rayleigh_length_at_the_machine_epsilon_keeps_the_waist_radius_and_a_negative_one_is_squared(
    ) {
        for zr in [0.0, 1e-17, -1e-17] {
            let beam = BeamShape {
                rayleigh_major_mm: zr,
                rayleigh_minor_mm: zr,
                ..sample()
            };
            let series = propagate(&[1.0, -2.0], &beam);
            assert_eq!(series.major_radii_um, vec![33.0, 33.0], "{zr}");
            assert_eq!(series.minor_radii_um, vec![34.0, 34.0], "{zr}");
        }
        let positive = propagate(&[1.0], &sample());
        let negative = propagate(
            &[1.0],
            &BeamShape {
                rayleigh_major_mm: -3.0,
                rayleigh_minor_mm: -3.0,
                ..sample()
            },
        );
        assert_eq!(positive, negative);
    }

    #[test]
    fn the_spot_of_a_resolved_beam_is_the_kernel_on_its_shape_and_says_its_version_and_its_beam() {
        let beam = resolved(1.0);
        let (commanded, actual) = z();
        let spot = spot_size_from_z(&beam, &commanded, &actual);
        let by_hand = compute_spot_size(
            &beam.shape(),
            &SpotKernelInput {
                commanded_samples: 8,
                actual_samples: 8,
                commanded_z_mm: &commanded,
                actual_z_mm: &actual,
            },
        );
        assert_eq!(spot.commanded(), &by_hand.commanded);
        assert_eq!(spot.actual(), &by_hand.actual);
        assert_eq!(spot.axis_angle_deg(), 0.0);
        assert_eq!(spot.algorithm_version(), SPOT_SIZE_ALGORITHM_VERSION);
        assert_eq!(SPOT_SIZE_ALGORITHM_VERSION, 1);
        // the beam is the one resolved, with its focus shift of 1 mm: the waist grown by sqrt(1 + (1/3)^2)
        assert_eq!(spot.beam(), &beam);
        let grown = 33.0 * (1.0_f64 + (1.0_f64 / 3.0).powi(2)).sqrt();
        assert!((spot.commanded().major_radii_um[0] - grown).abs() < 1e-12);
    }

    #[test]
    fn the_shape_of_a_resolved_beam_is_its_radii_lengths_shift_and_angle() {
        let beam = resolved(0.25);
        let shape = beam.shape();
        assert_eq!(shape.waist_radius_major_um, 33.0);
        assert_eq!(shape.waist_radius_minor_um, 34.0);
        assert_eq!(shape.rayleigh_major_mm, 3.0);
        assert_eq!(shape.focus_shift_mm, 0.25);
        assert_eq!(shape.axis_angle_deg, 0.0);
    }
}
