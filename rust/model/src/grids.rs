//! The correction grids of a laser, as recorded (IP-24 phase 24B; decision D24-11; requirements V:CP-11 and BL-CD-03): the descriptor that the metadata holds, a request for a grid's values, and the values
//! that come back.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** the library (`CorrectionGrid`, `GridData`); caller options (`GridRequest`).
//! * **Requirement:** V:CP-11 (the correction inputs are complete and explicit), BL-CD-03 (bulk data only through an accessor).
//!
//! # What this module claims, and what it does not
//!
//! * **A grid is described by the metadata and its values are read through [`ColumnSource::read_grid`](crate::ColumnSource::read_grid).** A laser's parameters list its grids
//!   ([`LaserParameters::grids`](crate::LaserParameters::grids)); a request names the laser and the grid by **the name the file spells**, and the whole grid comes back. A grid is about a megabyte,
//!   so there is no range, and nothing is cached.
//! * **The values are as stored**: in the type the file holds them (`float64`, `int32`, ...) and **row-major in the extents as the file stores them**, the last index running fastest. Nothing is
//!   converted, scaled, repaired or interpolated. **A value that is not a number is returned as it is**: the V17 grids are mostly NaN (about 85% of each: what lies outside the field that was
//!   calibrated), and that is what the file holds.
//! * **The library does not interpret a grid.** Which way its axes run, and whether its values are bits or millimetres, are stated by no file, so the descriptor says
//!   [`GridInterpretation::Unrecorded`] and a consumer's corrector decides. **Which grid it is** (the forward grid or the inverse) is what the file's own name says, and `None` where the name says
//!   neither (V16's one grid).
//! * **Two versions put different grids in different shapes**, and the descriptor does not choose between them: V17's are `float64` of 257 by 257 by 2 and V16's `int32` of 2 by 257 by 257, and both are
//!   an element type and a list of extents.
//!
//! The policy guards in `tests/policy_guards.rs` scan this file: every public struct says who builds it, and the type that holds bulk data says `// bulk-container:`.

use crate::channel::NumericType;
use crate::column::ColumnData;
use crate::error::{Error, ErrorKind, Location};
use crate::extra::Dims;
use crate::identity::{ChannelKey, LaserKey};

/// Which grid a file's own name says a grid is. A name that says neither has no role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum GridRole {
    /// The grid that the file names without saying "inverse" (V17's `Correction_Data`).
    Forward,
    /// The grid that the file names as the inverse (V17's `Inverse_Correction_Data`).
    Inverse,
}

/// What a file records about the meaning of a grid: its domain (bits or millimetres) and the order of its axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GridInterpretation {
    /// No file states it, so the library does not say: the corrector that uses the grid decides (decision D24-11).
    Unrecorded,
}

/// A correction grid of a laser, as the metadata describes it: its name, which grid the name says it is, the type it is stored as, and its extents in the order the file stores them.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CorrectionGrid {
    /// The name the file spells, which is also the key of a request.
    pub name: String,
    /// Which grid the name says it is, if it says.
    pub role: Option<GridRole>,
    /// The type the values are stored as, and returned in.
    pub stored_as: NumericType,
    /// The extents, in the order the file stores them.
    pub extents: Dims,
    /// What the file records of the grid's meaning: nothing, today.
    pub interpretation: GridInterpretation,
}

impl CorrectionGrid {
    /// A grid whose meaning is unrecorded, which is what every file we hold gives.
    pub fn new(
        name: impl Into<String>,
        role: Option<GridRole>,
        stored_as: NumericType,
        extents: Dims,
    ) -> Self {
        CorrectionGrid {
            name: name.into(),
            role,
            stored_as,
            extents,
            interpretation: GridInterpretation::Unrecorded,
        }
    }

    /// How many values the grid holds: the product of its extents.
    pub fn len(&self) -> u64 {
        self.extents
            .as_slice()
            .iter()
            .fold(1u64, |all, extent| all.saturating_mul(*extent))
    }

    /// Whether the grid holds no value (an extent of zero).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A request for the values of one grid: the laser, and the grid by the name the file spells (the `name` of its [`CorrectionGrid`]).
// constructed-by: caller-options
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GridRequest {
    pub laser: LaserKey,
    /// The name of the grid, as its descriptor gives it.
    pub grid: String,
}

impl GridRequest {
    pub fn new(laser: LaserKey, grid: impl Into<String>) -> Self {
        GridRequest {
            laser,
            grid: grid.into(),
        }
    }

    /// Where this request points, for an error that must say which grid it was about: the laser, the grid's name as a channel, and the operation `read_grid`.
    pub fn location(&self) -> Location {
        Location::new()
            .with_laser(self.laser.clone())
            .with_channel(ChannelKey::new([self.grid.as_str()]))
            .with_operation("read_grid")
    }
}

/// The values of a grid, as stored: row-major in the extents as the file stores them, in the type the file holds them.
// constructed-by: library
// bulk-container: the typed, shared values of one correction grid
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct GridData {
    /// The extents, in the order the file stores them.
    pub extents: Dims,
    /// The values, as many as the extents hold, the last index running fastest.
    pub values: ColumnData,
}

impl GridData {
    /// The values of a grid. A producer that gets the count wrong is told: `values` must hold exactly the product of `extents`.
    pub fn new(extents: Dims, values: ColumnData) -> Result<GridData, Error> {
        let wanted = extents
            .as_slice()
            .iter()
            .fold(1u64, |all, extent| all.saturating_mul(*extent));
        if values.len() as u64 != wanted {
            return Err(Error::new(ErrorKind::InvalidIndex {
                what: format!(
                    "{} values for a grid of the extents {:?}",
                    values.len(),
                    extents.as_slice()
                ),
            }));
        }
        Ok(GridData { extents, values })
    }

    /// How many values the grid holds.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the grid holds no value.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::{ColumnRequest, ColumnSource, InMemorySource, ReadContext, Shared};
    use crate::identity::LayerId;

    fn dims(extents: &[u64]) -> Dims {
        Dims::new(extents).expect("a shape")
    }

    fn laser(number: u32) -> LaserKey {
        LaserKey::new("a laser", number)
    }

    #[test]
    fn a_grid_descriptor_says_its_domain_is_unrecorded_and_its_extents_are_the_files() {
        let grid = CorrectionGrid::new(
            "the inverse",
            Some(GridRole::Inverse),
            NumericType::F64,
            dims(&[257, 257, 2]),
        );
        // no file states the domain or the order of the axes, so the descriptor says so and the library interprets nothing
        assert_eq!(grid.interpretation, GridInterpretation::Unrecorded);
        assert_eq!(grid.extents.as_slice(), [257, 257, 2]);
        assert_eq!(grid.len(), 132_098);
        assert!(!grid.is_empty());
        // a grid whose name says neither has no role, and extents in another order are as stored: neither version's shape is the model's
        let other = CorrectionGrid::new(
            "the only grid",
            None,
            NumericType::I32,
            dims(&[2, 257, 257]),
        );
        assert_eq!(other.role, None);
        assert_eq!(other.extents.as_slice(), [2, 257, 257]);
        assert_eq!(other.len(), grid.len());
    }

    #[test]
    fn grid_data_holds_exactly_the_values_its_extents_hold() {
        let values = ColumnData::F64(Shared::new(vec![1.0, f64::NAN, 3.0, 4.0, 5.0, 6.0]));
        let grid = GridData::new(dims(&[2, 3]), values.clone()).expect("six values for 2 by 3");
        assert_eq!(grid.len(), 6);
        match GridData::new(dims(&[2, 4]), values)
            .expect_err("six values for 2 by 4")
            .kind()
        {
            ErrorKind::InvalidIndex { what } => assert!(
                what.contains("6 values") && what.contains("[2, 4]"),
                "{what}"
            ),
            other => panic!("expected InvalidIndex, got {other:?}"),
        }
        // a grid of no value is a grid of an extent of zero
        assert!(
            GridData::new(dims(&[0, 3]), ColumnData::F64(Shared::new(Vec::new())))
                .expect("empty")
                .is_empty()
        );
    }

    #[test]
    fn a_grid_is_a_descriptor_in_the_metadata_and_bulk_through_the_accessor() {
        // the values come back as stored, a NaN included (a NaN is not equal to itself, so the comparison is of the bits)
        let stored = vec![0.5, f64::NAN, -2.0, f64::NAN];
        let mut source = InMemorySource::new();
        source.insert_grid(
            laser(1),
            "the forward",
            GridData::new(dims(&[2, 2]), ColumnData::F64(Shared::new(stored.clone())))
                .expect("a grid"),
        );
        let request = GridRequest::new(laser(1), "the forward");
        let got = source
            .read_grid(&request, &ReadContext::new())
            .expect("the grid");
        assert_eq!(got.extents.as_slice(), [2, 2]);
        match &got.values {
            ColumnData::F64(values) => {
                let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(values.as_slice()), bits(&stored));
            }
            other => panic!("expected float64, got {other:?}"),
        }
        // a grid that the source does not have, and one of another laser, are not present, and the error says which
        for asked in [
            GridRequest::new(laser(1), "the inverse"),
            GridRequest::new(laser(2), "the forward"),
        ] {
            let error = source
                .read_grid(&asked, &ReadContext::new())
                .expect_err("not present");
            assert!(
                matches!(error.kind(), ErrorKind::NotPresent { .. }),
                "{error}"
            );
            assert_eq!(error.location().laser, Some(asked.laser.clone()));
            assert_eq!(error.location().operation.as_deref(), Some("read_grid"));
        }
    }

    #[test]
    fn a_source_that_holds_no_grid_says_so_and_the_columns_are_not_grids() {
        struct NoGrids;
        impl ColumnSource for NoGrids {
            fn read(&self, _: &ColumnRequest, _: &ReadContext) -> Result<ColumnData, Error> {
                Err(Error::new(ErrorKind::NotPresent {
                    what: "a column".into(),
                }))
            }
        }
        let error = NoGrids
            .read_grid(
                &GridRequest::new(laser(1), "the forward"),
                &ReadContext::new(),
            )
            .expect_err("a source written before grids were readable");
        assert!(
            matches!(error.kind(), ErrorKind::UnsupportedInVersion { .. }),
            "{error}"
        );
        // a column of the same name is a column, not a grid
        let mut source = InMemorySource::new();
        source.insert(
            LayerId(1),
            Some(laser(1)),
            ChannelKey::new(["the forward"]),
            ColumnData::F64(Shared::new(vec![1.0])),
        );
        assert!(source
            .read_grid(
                &GridRequest::new(laser(1), "the forward"),
                &ReadContext::new()
            )
            .is_err());
    }
}
