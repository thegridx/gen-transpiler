use crate::runner::UserCodeRunnerError;

/// An amount of gas. Gas is a signed 64-bit count: the program runs while it is not negative,
/// and each pseudo basic block subtracts its cost on entry.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct UserCodeGasAmount(i64);

impl UserCodeGasAmount {
    /// An amount of `gas` units.
    pub const fn new(gas: i64) -> Self {
        Self(gas)
    }

    /// The amount as a plain integer.
    pub fn as_i64(&self) -> i64 {
        self.0
    }

    /// No gas: a call started with it runs out at its first block.
    pub const fn zero() -> Self {
        Self::new(0)
    }

    /// The most gas there is, for a run that should not be limited in practice.
    pub const fn max() -> Self {
        Self::new(i64::MAX)
    }

    /// The lowest gas `set_gas` stores: -2^62. Any negative gas already means out of gas, and
    /// keeping stored gas this far from `i64::MIN` means subtracting a block's cost can never
    /// wrap around to a large positive value.
    pub const fn min_stored() -> Self {
        Self::new(-(1 << 62))
    }

    /// This amount, raised to [`UserCodeGasAmount::min_stored`] if it is lower.
    pub fn clamped_to_min_stored(self) -> Self {
        Self::new(self.0.max(Self::min_stored().0))
    }

    /// Whether the amount is below zero, which means the program ran out of gas.
    pub fn is_negative(&self) -> bool {
        self.0 < 0
    }

    /// `self - other`, for example the gas a call used.
    ///
    /// # Errors
    ///
    /// Returns [`UserCodeRunnerError::OverflowError`] if the result does not fit an `i64`.
    pub fn sub(&self, other: &UserCodeGasAmount) -> Result<UserCodeGasAmount, UserCodeRunnerError> {
        match self.0.checked_sub(other.0) {
            Some(res) => Ok(UserCodeGasAmount::new(res)),
            None => Err(UserCodeRunnerError::OverflowError(format!(
                "sub {:#x} - {:#x} overflow",
                self.0, other.0
            ))),
        }
    }
}
