//! A typed immediate value wrapper with bit-width and alignment constraints.
//!
//! Used by RISC-V and ARM64 transpilers to enforce compile-time type safety
//! for instruction immediates. Each implementing type wraps a primitive storage
//! type and validates that values fit within the specified bit range with
//! required trailing zero alignment.
//!
//! # Associated Constants
//! - `N_BITS`: Total number of significant bits in the immediate
//! - `N_TRAILING_ZEROS`: Number of least-significant bits that must be zero
//!
//! # Valid Ranges
//! - Signed types: `[-2^(N_BITS-1)..2^(N_BITS-1))` with last `N_TRAILING_ZEROS` bits = 0
//! - Unsigned types: `[0..2^N_BITS)` with last `N_TRAILING_ZEROS` bits = 0
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub struct BadImmediateValue {
    value: i32,
    context: String,
}
impl BadImmediateValue {
    pub fn new(value: i32, context: String) -> Self {
        BadImmediateValue { value, context }
    }
}
impl fmt::Display for BadImmediateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.value)
    }
}

pub trait Immediate: Sized + fmt::Display + Copy {
    type StorageType: Copy + fmt::Debug + fmt::Display + PartialEq + PartialOrd + Sized + 'static;
    const N_BITS: u32;
    const N_TRAILING_ZEROS: u32;
    const IS_UNSIGNED: bool;
    const MAX_VAL: Self::StorageType;
    const MIN_VAL: Self::StorageType;
    /// Returns true iff the value is within the allowed range and alignment for this type
    fn is_valid(value: Self::StorageType) -> bool;
    /// Returns the raw internal value
    fn value(&self) -> Self::StorageType;
}

#[macro_export]
macro_rules! define_imm {
    ($name:ident, $storage:ty, $bits:expr, $trailing_zeros:expr) => {
        #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
        pub struct $name(pub $storage);
        #[expect(
            clippy::allow_attributes,
            reason = "Permits the #[allow] below within this macro-generated impl."
        )]
        #[allow(
            trivial_numeric_casts, clippy::bad_bit_mask,
            reason = "Macro-generated Immediate impl performs storage casts and bit masks that are trivial for some instantiations."
        )]
        impl $crate::immediate::Immediate for $name {
            type StorageType = $storage;
            const N_BITS: u32 = $bits;
            const N_TRAILING_ZEROS: u32 = $trailing_zeros;
            const IS_UNSIGNED: bool = (-1i64 as $storage) > 0;
            // Unsigned: [0..2^bits), Signed: [-2^(bits-1)..2^(bits-1))
            const MAX_VAL: $storage =
                (1 << (if Self::IS_UNSIGNED { $bits } else { $bits - 1 })) - 1;
            const MIN_VAL: $storage = if Self::IS_UNSIGNED {
                0
            } else {
                (0 as $storage).wrapping_sub(Self::MAX_VAL) - 1
            };
            fn is_valid(value: $storage) -> bool {
                const ALIGN_MASK: $storage = (1 << $trailing_zeros) - 1;
                let in_range = (Self::MIN_VAL..=Self::MAX_VAL).contains(&value);
                if ALIGN_MASK == 0 {
                    in_range
                } else {
                    in_range && (value & ALIGN_MASK) == 0
                }
            }
            fn value(&self) -> $storage {
                self.0
            }
        }
        #[expect(
            clippy::allow_attributes,
            reason = "Permits the #[allow] below within this macro-generated impl."
        )]
        #[allow(
            trivial_numeric_casts,
            reason = "The storage-to-i32 cast in the error path is trivial when the storage type is i32."
        )]
        impl TryFrom<$storage> for $name {
            type Error = $crate::immediate::BadImmediateValue;
            // Create a new instance WITH validation
            fn try_from(value: $storage) -> Result<Self, Self::Error> {
                if Self::is_valid(value) {
                    Ok(Self(value))
                } else {
                    Err($crate::immediate::BadImmediateValue::new(
                        value as i32,
                        stringify!($name).to_string(),
                    ))
                }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

// 5-bit shift for 32-bit ops [0..31]
define_imm!(ImmU5_0, u8, 5, 0);
// 12-bit immediate [0..4095]
define_imm!(ImmU12_0, u16, 12, 0);
