//! What a stored type is to the engine (IP-06 phases 6B and 6C, decisions D06-4 and D06-17): how a block of it is held, converted and made into a column.
//!
//! The conversions are the Model's own (`ColumnData::with_precision`): `From` where it is exact, and `as` where a value is rounded to the nearest. A test holds the two to each
//! other on every stored type, with values that are not exact in `f32`.

use build_log_model::{ColumnData, Error, ErrorKind, Shared};

use super::cache::Held;
use crate::hdf5::Element;

/// A stored type and what can be made of it.
pub(super) trait Native: Element {
    /// Whether this type is `f32` or `f64`: a column read at the precision it is stored in is returned as it is, sharing its buffer, as the Model's conversion does.
    const IS_F32: bool = false;
    const IS_F64: bool = false;

    /// The column of these values, in the variant of the stored type.
    fn column(values: Vec<Self>) -> ColumnData;
    /// The column of a shared buffer, without a copy.
    fn shared(values: Shared<Self>) -> ColumnData;
    /// A block as the cache holds it.
    fn hold(block: Shared<Self>) -> Held;
    /// The block a cache holds, if it holds this type.
    fn held(block: &Held) -> Option<Shared<Self>>;
    fn to_f32(self) -> f32;
    fn to_f64(self) -> f64;
}

macro_rules! native {
    ($t:ty, $variant:ident, $v:ident, $f32:expr, $f64:expr $(, $flag:ident)?) => {
        impl Native for $t {
            $(const $flag: bool = true;)?
            fn column(values: Vec<Self>) -> ColumnData {
                ColumnData::$variant(Shared::new(values))
            }
            fn shared(values: Shared<Self>) -> ColumnData {
                ColumnData::$variant(values)
            }
            fn hold(block: Shared<Self>) -> Held {
                Held::$variant(block)
            }
            fn held(block: &Held) -> Option<Shared<Self>> {
                match block {
                    Held::$variant(held) => Some(held.clone()),
                    _ => None,
                }
            }
            fn to_f32(self) -> f32 {
                let $v = self;
                $f32
            }
            fn to_f64(self) -> f64 {
                let $v = self;
                $f64
            }
        }
    };
}

native!(i16, I16, v, f32::from(v), f64::from(v));
native!(i32, I32, v, v as f32, f64::from(v));
native!(u64, U64, v, v as f32, v as f64);
native!(f32, F32, v, v, f64::from(v), IS_F32);
native!(f64, F64, v, v as f32, v, IS_F64);
native!(u8, U8, v, f32::from(v), f64::from(v));

/// Room for `additional` more values, or the typed error that says the memory is not there (decision D06-17): a result that cannot be allocated is not an abort.
pub(super) fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), Error> {
    values.try_reserve_exact(additional).map_err(|_| {
        Error::new(ErrorKind::IoError {
            os_code: None,
            detail: format!(
                "there is no room for {} bytes for the values of a column",
                additional.saturating_mul(std::mem::size_of::<T>())
            ),
            transient: true,
        })
    })
}

#[cfg(test)]
mod tests {
    use build_log_model::Precision;

    use super::*;

    #[test]
    fn a_result_that_cannot_be_allocated_is_a_typed_error_and_not_an_abort() {
        let mut values: Vec<u64> = Vec::new();
        let error = reserve(&mut values, usize::MAX / 4).expect_err("no room");
        match error.kind() {
            ErrorKind::IoError {
                os_code: None,
                transient: true,
                detail,
            } => assert!(detail.contains("bytes"), "{detail}"),
            other => panic!("expected a transient IoError, got {other:?}"),
        }
        // and room that is there is reserved
        assert!(reserve(&mut values, 10).is_ok() && values.capacity() >= 10);
    }

    #[test]
    fn the_conversions_are_the_models() {
        // the same value converted by this module and by the Model, for every stored type, at an end where a value is not exact in `f32`
        let check = |native: ColumnData, ours32: Vec<f32>, ours64: Vec<f64>| {
            assert_eq!(
                native
                    .clone()
                    .with_precision(Precision::F32)
                    .expect("numbers"),
                ColumnData::F32(Shared::new(ours32))
            );
            assert_eq!(
                native.with_precision(Precision::F64).expect("numbers"),
                ColumnData::F64(Shared::new(ours64))
            );
        };
        let narrow = [i16::MIN, -1, 0, 1, i16::MAX];
        check(
            ColumnData::I16(Shared::new(narrow.to_vec())),
            narrow.iter().map(|&v| v.to_f32()).collect(),
            narrow.iter().map(|&v| v.to_f64()).collect(),
        );
        let ints = [i32::MIN, -16_777_217, 16_777_217, i32::MAX, 0];
        check(
            ColumnData::I32(Shared::new(ints.to_vec())),
            ints.iter().map(|&v| v.to_f32()).collect(),
            ints.iter().map(|&v| v.to_f64()).collect(),
        );
        let big = [u64::MAX, (1 << 53) + 1, 0, 1];
        check(
            ColumnData::U64(Shared::new(big.to_vec())),
            big.iter().map(|&v| v.to_f32()).collect(),
            big.iter().map(|&v| v.to_f64()).collect(),
        );
        let singles = [0.1f32, -3.5e30, 1.0e-30, 0.0];
        check(
            ColumnData::F32(Shared::new(singles.to_vec())),
            singles.iter().map(|&v| v.to_f32()).collect(),
            singles.iter().map(|&v| v.to_f64()).collect(),
        );
        let doubles = [0.1f64, 1.0e300, -1.0e-300, 16_777_217.0];
        check(
            ColumnData::F64(Shared::new(doubles.to_vec())),
            doubles.iter().map(|&v| v.to_f32()).collect(),
            doubles.iter().map(|&v| v.to_f64()).collect(),
        );
        let bytes = [0u8, 1, 255];
        check(
            ColumnData::U8(Shared::new(bytes.to_vec())),
            bytes.iter().map(|&v| v.to_f32()).collect(),
            bytes.iter().map(|&v| v.to_f64()).collect(),
        );
    }

    #[test]
    fn a_block_is_held_as_its_own_type_and_only_that() {
        let narrow = i16::hold(Shared::new(vec![-1i16, 2, 3]));
        assert_eq!(
            i16::held(&narrow).expect("an i16 block").as_slice(),
            [-1, 2, 3]
        );
        assert!(i32::held(&narrow).is_none() && u8::held(&narrow).is_none());
        let block = Shared::new(vec![1i32, 2, 3]);
        let held = i32::hold(block.clone());
        assert_eq!(
            i32::held(&held).expect("an i32 block").as_slice(),
            [1, 2, 3]
        );
        assert!(f32::held(&held).is_none() && u8::held(&held).is_none());
        // the buffer is shared and not copied: the block and the held copy are two holders of one buffer
        assert_eq!(block.holders(), 2);
    }
}
