//! RISC-V gas cost associated with executing one or more RISC-V instructions in the transpiled code.
//! RvGasCost is a thin wrapper around ImmU12_0, mainly for ARM64's sake (x86 cares not for such
//! primitive limitations).
//! RvGasCostScheme defines how to calculate the gas cost of a RISC-V instruction, either as a fixed
//! per instruction group or a more nuanced approach.
//!
//! By default, each RISC-V instruction costs 1 gas unit.

use crate::common::ValueError;
use crate::common::checked_add_u16;
use crate::immediate::ImmU12_0;
use crate::immediate::Immediate as _;
use crate::ops::RiscvOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RvGasCost(ImmU12_0);
impl RvGasCost {
    pub const fn new(cost: ImmU12_0) -> Self {
        Self(cost)
    }
    pub const fn zero() -> Self {
        Self(ImmU12_0(0))
    }
    pub fn value(&self) -> u16 {
        self.0.value()
    }
    pub fn checked_add(&self, other: &Self, context: &str) -> Result<Self, ValueError> {
        let cum_value = checked_add_u16(self.0.value(), other.0.value(), context)?;
        Ok(RvGasCost(ImmU12_0::try_from(cum_value).map_err(|_| {
            ValueError::Overflow {
                base: self.0.value() as u64,
                add: other.0.value() as u64,
                context: context.to_string(),
            }
        })?))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RvGasCostScheme {
    Fixed(RvGasCost),
    PerInstruction,
}
impl RvGasCostScheme {
    pub fn cost(&self, inst: &RiscvOp) -> RvGasCost {
        match self {
            RvGasCostScheme::Fixed(cost) => *cost,
            RvGasCostScheme::PerInstruction => per_riscv_instruction_gas_cost(inst),
        }
    }
}

// Define const value per instruction type (modify as needed)
const RV_GAS_COST_BASIC: RvGasCost = RvGasCost(ImmU12_0(1));
const RV_GAS_COST_BRANCH: RvGasCost = RvGasCost(ImmU12_0(1));
const RV_GAS_COST_STORE_LOAD_JALR: RvGasCost = RvGasCost(ImmU12_0(1));
const RV_GAS_COST_ATOMIC: RvGasCost = RvGasCost(ImmU12_0(1));
const RV_GAS_COST_ECALL: RvGasCost = RvGasCost(ImmU12_0(1));

fn per_riscv_instruction_gas_cost(inst: &RiscvOp) -> RvGasCost {
    match inst {
        RiscvOp::OpImm(_) => RV_GAS_COST_BASIC,
        RiscvOp::OpReg(_) => RV_GAS_COST_BASIC,
        RiscvOp::Lui(_) => RV_GAS_COST_BASIC,
        RiscvOp::Auipc(_) => RV_GAS_COST_BASIC,
        RiscvOp::MulDivRem(_) => RV_GAS_COST_BASIC,
        RiscvOp::Jal(_) => RV_GAS_COST_BRANCH,
        RiscvOp::Jalr(_) => RV_GAS_COST_STORE_LOAD_JALR,
        RiscvOp::Branch(_) => RV_GAS_COST_BRANCH,
        RiscvOp::Load(_) => RV_GAS_COST_STORE_LOAD_JALR,
        RiscvOp::Store(_) => RV_GAS_COST_STORE_LOAD_JALR,
        RiscvOp::Ecall => RV_GAS_COST_ECALL,
        RiscvOp::Atomic(_) => RV_GAS_COST_ATOMIC,
        RiscvOp::Fence(_) => RV_GAS_COST_BASIC,
    }
}

// For now, use fixed gas cost per instruction. Later - switch to RvGasCostScheme::PerInstruction and adjust the gas costs as needed.
pub const DEFAULT_GAS_COST_SCHEME: RvGasCostScheme =
    RvGasCostScheme::Fixed(RvGasCost::new(ImmU12_0(1)));

#[cfg(test)]
mod tests {
    use super::*;

    // ── RvGasCost::add ─────────────────────────────────────────────────────

    #[test]
    fn gas_cost_add_zero_is_identity() {
        let cost = RvGasCost(ImmU12_0(42));
        assert_eq!(
            cost.checked_add(&RvGasCost::zero(), "test")
                .unwrap()
                .value(),
            42
        );
        assert_eq!(
            RvGasCost::zero()
                .checked_add(&cost, "test")
                .unwrap()
                .value(),
            42
        );
        assert_eq!(
            RvGasCost::zero()
                .checked_add(&RvGasCost::zero(), "test")
                .unwrap()
                .value(),
            0
        );
    }

    #[test]
    fn gas_cost_add_combines_costs() {
        assert_eq!(
            RvGasCost(ImmU12_0(10))
                .checked_add(&RvGasCost(ImmU12_0(20)), "test")
                .unwrap()
                .0,
            ImmU12_0(30)
        );
        assert_eq!(
            RvGasCost(ImmU12_0(1))
                .checked_add(&RvGasCost(ImmU12_0(1)), "test")
                .unwrap()
                .0,
            ImmU12_0(2)
        );
    }

    #[test]
    fn gas_cost_overflow() {
        assert!(
            RvGasCost(ImmU12_0(ImmU12_0::MAX_VAL))
                .checked_add(&RvGasCost(ImmU12_0(1)), "test")
                .is_err()
        );
    }

    // ── RvGasCostScheme ──────────────────────────────────────────────────

    use crate::ops::{
        ArithmeticOp, BasicOp, RegisterAndImmediate, RvImmI12, RvImmU20s12, RvLui, RvOpImm,
        TwoRegistersAndImmediate,
    };
    use crate::registers::RiscvRegister;

    /// Helper: build a LUI instruction (arbitrary register and immediate).
    fn lui_op() -> RiscvOp {
        let imm = RvImmU20s12::try_from(0x1000_i32).expect("valid U-type immediate");
        RiscvOp::Lui(RvLui::new(RegisterAndImmediate::new(
            RiscvRegister::A0,
            imm,
        )))
    }

    /// Helper: build an ADDI instruction (OpImm / Basic(Add)).
    fn addi_op() -> RiscvOp {
        let imm = RvImmI12::try_from(42_i16).expect("valid I-type immediate");
        RiscvOp::OpImm(RvOpImm::new(
            ArithmeticOp::Basic(BasicOp::Add),
            TwoRegistersAndImmediate::new(RiscvRegister::A0, RiscvRegister::T0, imm),
        ))
    }

    #[test]
    fn fixed_scheme_returns_same_cost_for_any_instruction() {
        let cost = RvGasCost::new(ImmU12_0(7));
        let scheme = RvGasCostScheme::Fixed(cost);

        assert_eq!(scheme.cost(&RiscvOp::Ecall).value(), 7);
        assert_eq!(scheme.cost(&lui_op()).value(), 7);
        assert_eq!(scheme.cost(&addi_op()).value(), 7);
    }

    #[test]
    fn per_instruction_scheme_returns_defined_cost_for_ecall() {
        let scheme = RvGasCostScheme::PerInstruction;
        assert_eq!(
            scheme.cost(&RiscvOp::Ecall).value(),
            RV_GAS_COST_ECALL.value()
        );
    }

    #[test]
    fn per_instruction_scheme_returns_defined_cost_for_lui() {
        let scheme = RvGasCostScheme::PerInstruction;
        assert_eq!(scheme.cost(&lui_op()).value(), RV_GAS_COST_BASIC.value());
    }

    #[test]
    fn per_instruction_scheme_returns_defined_cost_for_addi() {
        let scheme = RvGasCostScheme::PerInstruction;
        assert_eq!(scheme.cost(&addi_op()).value(), RV_GAS_COST_BASIC.value());
    }
}
