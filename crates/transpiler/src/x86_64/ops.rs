//! x86_64 ops: immediates, condition codes, and the symbolic and native ops the transpiler
//! emits before and after branch and label resolution.
use riscv32::common::SignType;
use riscv32::ops::{LoadSize, StoreSize};
use std::fmt::{self};

use crate::layout::{LocalCodeLabel, SymbolicMemoryLocation};
use crate::traits::{PcRelativeOffset, SymbolicOp, TranspiledOp, UnresOp};
use crate::{TranspilerError, layout::SymbolicBranchTarget, traits::NativeOp};

use super::bytecode::ByteCounter;
use super::registers::{REG_CL, X86Reg8Bit, X86Reg32Bit, X86Reg32Or64, X86Reg64Bit, X86TempReg};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct X86Imm8Bit(i8);

impl X86Imm8Bit {
    /// Creates an 8-bit immediate.
    pub(crate) const fn new(value: i8) -> Self {
        Self(value)
    }

    /// Returns the underlying signed 8-bit value.
    pub(crate) const fn get(&self) -> i8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct X86Imm32Bit(i32);

impl X86Imm32Bit {
    /// Creates a 32-bit immediate.
    pub(crate) const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Returns the underlying signed 32-bit value.
    pub(crate) const fn get(&self) -> i32 {
        self.0
    }
}

/// An immediate operand. `From<i32>` picks the 8-bit form when the value fits, so the encoder
/// can use the shorter sign-extended imm8 encodings.
#[expect(
    variant_size_differences,
    reason = "The variants model immediate operands of different sizes."
)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum X86Immediate {
    Imm8(X86Imm8Bit),
    Imm32(X86Imm32Bit),
}
impl fmt::Display for X86Immediate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_i32())
    }
}
impl X86Immediate {
    pub(crate) fn as_i32(&self) -> i32 {
        match self {
            X86Immediate::Imm8(imm) => imm.0 as i32,
            X86Immediate::Imm32(imm) => imm.0,
        }
    }
}
impl From<i32> for X86Immediate {
    fn from(value: i32) -> Self {
        match value {
            -128..=127 => X86Immediate::Imm8(X86Imm8Bit::new(value as i8)),
            _ => X86Immediate::Imm32(X86Imm32Bit::new(value)),
        }
    }
}

/// A source operand: a 32-bit register or an immediate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum X86Reg32OrImm {
    Reg(X86Reg32Bit),
    Imm(X86Immediate),
}
impl From<X86Reg32Bit> for X86Reg32OrImm {
    fn from(r: X86Reg32Bit) -> Self {
        X86Reg32OrImm::Reg(r)
    }
}
impl From<X86Immediate> for X86Reg32OrImm {
    fn from(imm: X86Immediate) -> Self {
        X86Reg32OrImm::Imm(imm)
    }
}
impl fmt::Display for X86Reg32OrImm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            X86Reg32OrImm::Reg(r) => write!(f, "{r}"),
            X86Reg32OrImm::Imm(imm) => write!(f, "{imm}"),
        }
    }
}

/// A shift count: an immediate or the cl register.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum X86ShiftAmount {
    Imm(X86Imm8Bit),
    Cl,
}
impl From<X86Imm8Bit> for X86ShiftAmount {
    fn from(imm: X86Imm8Bit) -> Self {
        X86ShiftAmount::Imm(imm)
    }
}
impl fmt::Display for X86ShiftAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            X86ShiftAmount::Imm(imm) => write!(f, "{}", imm.0),
            X86ShiftAmount::Cl => write!(f, "{REG_CL}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum X86ConditionCode {
    // Equality
    Equal,
    NotEqual,
    // Signed comparisons
    Less,
    Greater,
    GreaterEqual,
    // Unsigned comparisons
    Below,
    Above,
    AboveEqual,
    // Sign flag tests
    Sign,
    NotSign,
}
impl fmt::Display for X86ConditionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            X86ConditionCode::Equal => "e",
            X86ConditionCode::NotEqual => "ne",
            X86ConditionCode::Less => "l",
            X86ConditionCode::Greater => "g",
            X86ConditionCode::GreaterEqual => "ge",
            X86ConditionCode::Below => "b",
            X86ConditionCode::Above => "a",
            X86ConditionCode::AboveEqual => "ae",
            X86ConditionCode::Sign => "s",
            X86ConditionCode::NotSign => "ns",
        };
        write!(f, "{s}")
    }
}
impl X86ConditionCode {
    // The second opcode byte of the near (rel32) `Jcc`, `0F 8x`.
    pub(crate) fn encoding(&self) -> u8 {
        match self {
            X86ConditionCode::Below => 0x82,
            X86ConditionCode::AboveEqual => 0x83,
            X86ConditionCode::Equal => 0x84,
            X86ConditionCode::NotEqual => 0x85,
            X86ConditionCode::Above => 0x87,
            X86ConditionCode::Sign => 0x88,
            X86ConditionCode::NotSign => 0x89,
            X86ConditionCode::Less => 0x8C,
            X86ConditionCode::GreaterEqual => 0x8D,
            X86ConditionCode::Greater => 0x8F,
        }
    }
}

#[expect(
    non_camel_case_types,
    reason = "Variant names mirror assembly mnemonics and operands, which are not camel-case."
)]
/// An op whose PC-relative operand is still a label or RISC-V address. The branch passes turn
/// each one into the `X86NativeOp` of the same shape and size.
#[derive(Clone, Debug)]
pub(crate) enum X86SymbolicOp {
    Lea_Tmp_Mem(X86TempReg, SymbolicMemoryLocation),
    Mov_Reg_Mem(X86Reg32Or64, SymbolicMemoryLocation),
    Mov_Mem_Reg(SymbolicMemoryLocation, X86Reg32Or64),
    Mov_Mem_Imm(SymbolicMemoryLocation, X86Immediate),
    Jmp(SymbolicBranchTarget),
    Jcc(X86ConditionCode, SymbolicBranchTarget),
}
impl X86SymbolicOp {
    /// The native op this resolves to at `offset`. Every x86_64 PC-relative operand is a full
    /// rel32, so any offset fits.
    fn resolve(self, offset: PcRelativeOffset) -> X86NativeOp {
        match self {
            X86SymbolicOp::Jmp(_) => X86NativeOp::Jmp(offset),
            X86SymbolicOp::Jcc(cond, _) => X86NativeOp::Jcc(cond, offset),
            X86SymbolicOp::Lea_Tmp_Mem(tmp, _) => X86NativeOp::Lea_Tmp_Mem(tmp, offset),
            X86SymbolicOp::Mov_Reg_Mem(reg, _) => X86NativeOp::Mov_Reg_Mem(reg, offset),
            X86SymbolicOp::Mov_Mem_Reg(_, reg) => X86NativeOp::Mov_Mem_Reg(offset, reg),
            X86SymbolicOp::Mov_Mem_Imm(_, imm) => X86NativeOp::Mov_Mem_Imm(offset, imm),
        }
    }
}
impl SymbolicOp for X86SymbolicOp {
    // Measured from the encoding of the resolved op, so it cannot drift from the encoder. The
    // offset does not change the size: every PC-relative operand is a rel32.
    fn size_in_bytes(&self) -> u32 {
        self.clone()
            .resolve(PcRelativeOffset::new(0))
            .size_in_bytes()
    }
    fn is_small_skip_branch(&self) -> bool {
        let small_skip = SymbolicBranchTarget::Local(LocalCodeLabel::SmallSkip);
        match self {
            Self::Jmp(target) | Self::Jcc(_, target) => *target == small_skip,
            Self::Lea_Tmp_Mem(..)
            | Self::Mov_Reg_Mem(..)
            | Self::Mov_Mem_Reg(..)
            | Self::Mov_Mem_Imm(..) => false,
        }
    }
}
impl From<X86SymbolicOp> for UnresOp<X86Op> {
    fn from(op: X86SymbolicOp) -> Self {
        UnresOp::Symbolic(op)
    }
}

#[expect(
    non_camel_case_types,
    reason = "Variant names mirror assembly mnemonics and operands, which are not camel-case."
)]
/// A fully resolved x86_64 instruction. Variant names follow the mnemonic and operand kinds:
/// `Reg` is a register, `RoI` a register or immediate, `Mem` a RIP-relative location,
/// `MemTmp` the address in a temporary register, and `Tmp` a temporary register.
#[derive(Debug, Clone)]
pub(crate) enum X86NativeOp {
    // Address calculation
    Lea_Tmp_Mem(X86TempReg, PcRelativeOffset),
    Add_Tmp64_Reg64(X86TempReg, X86Reg64Bit), // Used along with LEA for base + offset calculations
    // Data movement
    Mov_Reg_Mem(X86Reg32Or64, PcRelativeOffset),
    Mov_Mem_Reg(PcRelativeOffset, X86Reg32Or64),
    Mov_Mem_Imm(PcRelativeOffset, X86Immediate),
    Mov_Reg_MemTmp(X86Reg32Bit, X86TempReg),
    Mov_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    // Arithmetic
    Add_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    Sub_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    Mul_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    MulClobber(SignType, X86Reg32Bit), // Clobbers edx:eax
    DivClobber(SignType, X86Reg32Bit), // Clobbers edx:eax
    // Bitwise
    And_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    Or_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    Xor_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    Xor_Self(X86Reg32Or64),
    Shl_Reg_Sft(X86Reg32Bit, X86ShiftAmount),
    Shr_Reg_Sft(X86Reg32Bit, X86ShiftAmount),
    Sar_Reg_Sft(X86Reg32Bit, X86ShiftAmount),
    Neg_Reg(X86Reg32Bit),
    // Comparison
    Cmp_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    Test_Reg_RoI(X86Reg32Bit, X86Reg32OrImm),
    // Conditional checks and jumps
    Jmp(PcRelativeOffset),
    Jcc(X86ConditionCode, PcRelativeOffset),
    Jmp_Tmp(X86TempReg), // Jump to a temporary register
    // Set & sign extension
    SetLow8(SignType, X86Reg8Bit),
    Movzx_Self(X86Reg32Bit),
    Movsxd_Tmp(X86TempReg),
    Cdq, // sign-extend EAX into edx:eax for division
    // Load/store operations
    Load_Reg_MemTmp(LoadSize, X86Reg32Bit, X86TempReg),
    Store_MemTmp_Reg(StoreSize, X86TempReg, X86Reg32Bit),
    // Direct stack operations
    Push(X86Reg64Bit),
    Pop(X86Reg64Bit),
    // Gas-specific operations
    Sub_Gas_Imm(X86Immediate),
    Sub_Gas_Tmp(X86TempReg),
    Ret(),
}

impl From<X86NativeOp> for UnresOp<X86Op> {
    fn from(op: X86NativeOp) -> Self {
        UnresOp::Native(op)
    }
}

impl NativeOp for X86NativeOp {
    fn size_in_bytes(&self) -> u32 {
        // x86 encodings are variable-length (optional REX prefix, immediate width), so the
        // size is derived from the encoding itself. A `ByteCounter` measures the length without
        // materializing the bytes or allocating.
        let mut counter = ByteCounter::default();
        self.x86_encode(&mut counter);
        counter.count()
    }
    fn encode(&self, out: &mut Vec<u8>) {
        self.x86_encode(out);
    }
    #[cfg(test)]
    fn as_assembly(&self) -> String {
        self.x86_as_asm_string()
    }
}

#[derive(Clone)]
pub(crate) struct X86Op;
impl TranspiledOp for X86Op {
    type Native = X86NativeOp;
    type Symbolic = X86SymbolicOp;
    const PC_RELATIVE_FROM_END: bool = true;

    fn new_branch(target: SymbolicBranchTarget) -> X86SymbolicOp {
        X86SymbolicOp::Jmp(target)
    }
    fn branch_size_in_bytes() -> u32 {
        X86NativeOp::Jmp(PcRelativeOffset::new(0)).size_in_bytes()
    }

    fn sym_to_native(
        sym_op: X86SymbolicOp,
        pc_relative_offset: PcRelativeOffset,
    ) -> Result<X86NativeOp, TranspilerError> {
        Ok(sym_op.resolve(pc_relative_offset))
    }
}

#[cfg(test)]
mod tests {
    use riscv32_elf::data_layout::StoredRegister;

    use super::{X86ConditionCode, X86NativeOp, X86Op, X86SymbolicOp, X86TempReg};
    use crate::layout::GlobalCodeLabel;
    use crate::traits::{NativeOp as _, PcRelativeOffset, SymbolicOp as _, TranspiledOp};
    use crate::x86_64::registers::{REG_EAX, REG_R8D, REG_R12, X86_REG_GAS};

    fn symbolic_ops() -> Vec<X86SymbolicOp> {
        let memory = || StoredRegister::Gas.into();
        let mut ops = vec![
            X86SymbolicOp::Jmp(GlobalCodeLabel::ExitPanic.into()),
            X86SymbolicOp::Jcc(X86ConditionCode::Below, GlobalCodeLabel::ExitPanic.into()),
            X86SymbolicOp::Lea_Tmp_Mem(X86TempReg::Tmp1, memory()),
            X86SymbolicOp::Lea_Tmp_Mem(X86TempReg::Tmp2, memory()),
            X86SymbolicOp::Mov_Mem_Imm(memory(), 1.into()),
            X86SymbolicOp::Mov_Mem_Imm(memory(), 0x1234_5678.into()),
        ];
        for reg in [
            REG_EAX.into(),
            REG_R8D.into(),
            REG_R12.into(),
            X86_REG_GAS.into(),
        ] {
            ops.push(X86SymbolicOp::Mov_Reg_Mem(reg, memory()));
            ops.push(X86SymbolicOp::Mov_Mem_Reg(memory(), reg));
        }
        ops
    }

    /// The resolution passes size symbolic ops before their offsets are known, so the size
    /// must not depend on the offset.
    #[test]
    fn symbolic_op_size_does_not_depend_on_the_offset() {
        for op in symbolic_ops() {
            let size = op.size_in_bytes();
            for offset in [0, 1, -1, 0x7F, -0x80, 0x1_0000, i32::MAX, i32::MIN] {
                let native = X86Op::sym_to_native(op.clone(), PcRelativeOffset::new(offset))
                    .expect("every offset fits a rel32");
                let mut bytes = Vec::new();
                native.encode(&mut bytes);
                assert_eq!(bytes.len() as u32, size, "{op:?} at offset {offset}");
            }
        }
        assert_eq!(
            X86Op::branch_size_in_bytes(),
            X86NativeOp::Jmp(PcRelativeOffset::new(i32::MIN)).size_in_bytes()
        );
    }
}
