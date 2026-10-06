//! The aarch64 ops the transpiler emits: symbolic ops whose targets are labels, and native ops
//! that encode to exactly one 32-bit instruction word.
//!
//! Op names follow the mnemonic and operand shape, for example `Adds_W_W_Imm` is
//! `ADDS Wd, Wn, #imm`. `xRegTmp`, `wRegTmp`, `xRegTmp2`, `wRegTmp2` and `xRegGas` name the
//! fixed registers `REG_TMP`, `REG_TMP_2` and `REG_GAS`, and `xSp` the stack pointer.
use riscv32::immediate::BadImmediateValue;
use riscv32::immediate::{ImmU5_0, ImmU12_0, Immediate};
use riscv32::{N_BYTES_IN_WORD, define_imm};
use std::fmt;
use strum::FromRepr;

use crate::{
    TranspilerError,
    arch64::registers::ArmRegister,
    layout::{GlobalDataLabel, LocalCodeLabel, SymbolicBranchTarget},
    traits::{NativeOp, PcRelativeOffset, SymbolicOp, TranspiledOp, UnresOp},
};

/// An aarch64 condition code, as encoded in `B.cond`, `CSEL` and `CSET`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, FromRepr)]
pub(crate) enum ConditionCode {
    /// Equal
    Eq = 0,
    /// Not equal
    Ne = 1,
    /// Unsigned greater than or equal
    Hs = 2,
    /// Unsigned strictly less than
    Lo = 3,
    /// Negative
    Mi = 4,
    /// Positive or zero
    Pl = 5,
    /// Unsigned higher
    Hi = 8,
    /// Unsigned lower or same
    Ls = 9,
    /// Signed greater than or equal
    Ge = 10,
    /// Signed strictly less than
    Lt = 11,
    /// Signed strictly greater than
    Gt = 12,
    /// Signed less than or equal
    Le = 13,
}

impl fmt::Display for ConditionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{self:?}").to_uppercase())
    }
}

impl ConditionCode {
    /// Returns the inverse condition, used to encode `CSET`. Conditions come in pairs that
    /// differ only in bit 0.
    pub(crate) fn inverted(&self) -> ConditionCode {
        ConditionCode::from_repr(*self as u32 ^ 1).unwrap_or_else(|| unreachable!())
    }
}

// Unsigned immediates. `define_imm!(Name, storage, bits, trailing_zeros)`.
define_imm!(ImmU5_4, u8, 5, 4); // MOVZ/MOVK shift: 0 or 16
define_imm!(ImmU6_0, u8, 6, 0); // 6-bit shift for 64-bit ops [0..63]
define_imm!(ImmU14_2, u16, 14, 2); // Scaled LDR/STR W [0..16380] step 4
define_imm!(ImmU15_3, u16, 15, 3); // Scaled LDR/STR X [0..32760] step 8

// Signed immediates
define_imm!(ImmI9_0, i16, 9, 0); // Unscaled load/store [-256..255]
define_imm!(ImmI9_2, i16, 9, 2); // LDP/STP W [-256..252] step 4
define_imm!(ImmI12_0, i16, 12, 0); // CMP/MOV immediate [-2048..2047]
define_imm!(ImmI21_0, i32, 21, 0); // ADR PC-relative offset ±1MB (byte aligned)
define_imm!(ImmI21_2, i32, 21, 2); // B.cond/CBZ/CBNZ offset ±1MB
define_imm!(ImmI33_12, i64, 33, 12); // ADRP byte offset ±4GB (page aligned)
define_imm!(ImmI28_2, i32, 28, 2); // Unconditional branch offset ±128MB

/// Signed immediate for `ADD`/`SUB sp, sp, #imm`, in `[-4095, 4095]`: a 12-bit magnitude plus
/// the choice of `ADD` or `SUB`. The range is symmetric, so `define_imm!` cannot express it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImmI13_0(i16);
impl ImmI13_0 {
    /// Creates a 13-bit signed immediate without range validation.
    ///
    /// Callers must ensure `value` is within `[-4095, 4095]`; use
    /// [`ImmI13_0::try_from`] when the value is not statically known to be valid.
    pub(crate) const fn new(value: i16) -> Self {
        Self(value)
    }
}
impl Immediate for ImmI13_0 {
    type StorageType = i16;
    const N_BITS: u32 = 13;
    const N_TRAILING_ZEROS: u32 = 0;
    const IS_UNSIGNED: bool = false;
    const MAX_VAL: i16 = 4095;
    const MIN_VAL: i16 = -4095;
    fn is_valid(value: i16) -> bool {
        (Self::MIN_VAL..=Self::MAX_VAL).contains(&value)
    }
    fn value(&self) -> i16 {
        self.0
    }
}
impl TryFrom<i16> for ImmI13_0 {
    type Error = BadImmediateValue;
    fn try_from(value: i16) -> Result<Self, Self::Error> {
        if Self::is_valid(value) {
            Ok(Self(value))
        } else {
            Err(BadImmediateValue::new(value as i32, "ImmI13_0".to_string()))
        }
    }
}
impl fmt::Display for ImmI13_0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[expect(
    non_camel_case_types,
    reason = "Variant names mirror assembly mnemonics and operands, which are not camel case."
)]
#[derive(Debug, Clone)]
/// An op whose PC-relative operand is still a label or RISC-V address. Branch resolution turns
/// each one into the matching [`Arm64NativeOp`].
pub(crate) enum Arm64SymbolicOp {
    Adrp_xRegTmp_Label(GlobalDataLabel),
    Adr_xRegTmp(GlobalDataLabel),
    Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel),
    Cbz_W(ArmRegister, SymbolicBranchTarget),
    Cbnz_W(ArmRegister, SymbolicBranchTarget),
    B(SymbolicBranchTarget),
    Bcc(SymbolicBranchTarget, ConditionCode),
}
impl SymbolicOp for Arm64SymbolicOp {
    fn size_in_bytes(&self) -> u32 {
        N_BYTES_IN_WORD as u32
    }
    fn is_small_skip_branch(&self) -> bool {
        let small_skip = SymbolicBranchTarget::Local(LocalCodeLabel::SmallSkip);
        match self {
            Self::B(target)
            | Self::Bcc(target, _)
            | Self::Cbz_W(_, target)
            | Self::Cbnz_W(_, target) => *target == small_skip,
            _ => false,
        }
    }
}
impl From<Arm64SymbolicOp> for UnresOp<Arm64Op> {
    fn from(op: Arm64SymbolicOp) -> Self {
        UnresOp::Symbolic(op)
    }
}

#[expect(
    non_camel_case_types,
    reason = "Variant names mirror assembly mnemonics and operands, which are not camel case."
)]
#[derive(Debug, Clone)]
/// A fully resolved aarch64 instruction.
pub(crate) enum Arm64NativeOp {
    Nop,
    Adrp_xRegTmp_Label(ImmI33_12),
    Adr_xRegTmp(ImmI21_0),
    Add_xRegTmp_xRegTmp_Lo12Label(ImmU12_0),
    /// `ADDS Wd, Wn, #imm`. Rn = 31 means `WSP` here, not `WZR`, so `Wn` must never be
    /// [`ArmRegister::Zr`]; the same holds for `Subs_W_W_Imm` and `Cmp_W_Imm`. Use `Mov_W_Imm`
    /// or `Mov_W_W` instead.
    Adds_W_W_Imm(ArmRegister, ArmRegister, ImmU12_0),
    Adds_xRegTmp_xRegTmp_Imm(ImmU12_0),
    Adds_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Adds_xRegTmp_xRegTmp_w(ArmRegister),
    Subs_W_W_Imm(ArmRegister, ArmRegister, ImmU12_0),
    Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0),
    Subs_xRegTmp_xRegTmp_Imm(ImmU12_0),
    Subs_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Eor_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Eon_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Orr_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Orn_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    And_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    /// `AND Wd, Wn, #~3`: clears the two low bits.
    Bic_W_W_3(ArmRegister, ArmRegister),
    /// `TST Wn, #2` - sets the flags from `Wn & 2` (bit 1 of the register).
    Tst_W_2(ArmRegister),
    Bic_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Lsr_W_W_Imm(ArmRegister, ArmRegister, ImmU5_0),
    /// `LSR Xd, xRegTmp, #32`: the high word of a 64-bit product.
    Lsr_X_xRegTmp_32(ArmRegister),
    Lsr_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Asr_W_W_Imm(ArmRegister, ArmRegister, ImmU5_0),
    Asr_xRegTmp_xRegTmp_Imm(ImmU6_0),
    Asr_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Lsl_W_W_Imm(ArmRegister, ArmRegister, ImmU5_0),
    Lsl_xRegTmp_xRegTmp_Imm(ImmU6_0),
    Lsl_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Cmp_W_Imm(ArmRegister, ImmI12_0),
    Cmp_W_W(ArmRegister, ArmRegister),
    Cset_W(ArmRegister, ConditionCode),
    /// `CSEL wRegTmp2, wRegTmp2, wRegTmp, cond`: keeps `wRegTmp2` if `cond` holds.
    Csel_wRegTmp2_wRegTmp2_wRegTmp(ConditionCode),
    Movz_W_Imm_Shift(ArmRegister, u16, ImmU5_4),
    Movk_W_Imm_Shift(ArmRegister, u16, ImmU5_4),
    /// `MOVZ` for a non-negative immediate, `MOVN` for a negative one.
    Mov_W_Imm(ArmRegister, ImmI12_0),
    /// `MOV Wd, Wm`, an alias of `ORR Wd, WZR, Wm`. Either register may be
    /// [`ArmRegister::Zr`].
    Mov_W_W(ArmRegister, ArmRegister),
    Mul_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Mul_xRegTmp_xRegTmp_xRegTmp2,
    Smull_xRegTmp_W_W(ArmRegister, ArmRegister),
    Umull_xRegTmp_W_W(ArmRegister, ArmRegister),
    /// `SXTW Xd, Wn`.
    Sxtw(ArmRegister, ArmRegister),
    Sdiv_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Udiv_W_W_W(ArmRegister, ArmRegister, ArmRegister),
    Cbz_W(ArmRegister, ImmI21_2),
    Cbnz_W(ArmRegister, ImmI21_2),
    B(ImmI28_2),
    Bcc(ImmI21_2, ConditionCode),
    Br_xRegTmp,
    LdrsbW_xRegTmp_0(ArmRegister),
    LdurshW_xRegTmp_0(ArmRegister),
    Ldur_W_xRegTmp_Imm(ArmRegister, ImmI9_0),
    Ldur_X_xRegTmp_Imm(ArmRegister, ImmI9_0),
    Ldr_W_xSp_Imm(ArmRegister, ImmU14_2),
    /// `LDR xRegTmp, [Xn, #imm]`.
    Ldr_xRegTmp_X_Imm(ArmRegister, ImmU15_3),
    LdrbW_xRegTmp_0(ArmRegister),
    LdurhW_xRegTmp_0(ArmRegister),
    Stur_W_xRegTmp_Imm(ArmRegister, ImmI9_0),
    Stur_X_xRegTmp_Imm(ArmRegister, ImmI9_0),
    Str_W_xSp_Imm(ArmRegister, ImmU14_2),
    Str_xRegTmp_xSp_Imm(ImmU15_3),
    Strb_W_xRegTmp_0(ArmRegister),
    Sturh_W_xRegTmp_0(ArmRegister),
    /// `LDP Xt1, Xt2, [sp], #16` (pop a pair).
    Ldp_X_X_xSp_adv16(ArmRegister, ArmRegister),
    /// `STP Xt1, Xt2, [sp, #-16]!` (push a pair).
    Stp_X_X_xSp_minus16(ArmRegister, ArmRegister),
    Ldp_W_W_xSp_Imm(ArmRegister, ArmRegister, ImmI9_2),
    Stp_W_W_xSp_Imm(ArmRegister, ArmRegister, ImmI9_2),
    /// `ADD sp, sp, #imm`, or `SUB` for a negative `imm`. Does not set flags.
    Add_xSp_Imm(ImmI13_0),
    Ret_Lr,
    Subs_xRegGas_xRegGas_X(ArmRegister),
    Subs_xRegGas_xRegGas_Imm(ImmU12_0),
}

/// An immediate field that a resolved PC-relative offset can be narrowed into.
trait PcRelativeImmediate: Sized {
    fn try_from_offset(offset: i32) -> Result<Self, BadImmediateValue>;
}
impl PcRelativeImmediate for ImmI21_2 {
    fn try_from_offset(offset: i32) -> Result<Self, BadImmediateValue> {
        Self::try_from(offset)
    }
}
impl PcRelativeImmediate for ImmI28_2 {
    fn try_from_offset(offset: i32) -> Result<Self, BadImmediateValue> {
        Self::try_from(offset)
    }
}
impl PcRelativeImmediate for ImmI21_0 {
    fn try_from_offset(offset: i32) -> Result<Self, BadImmediateValue> {
        Self::try_from(offset)
    }
}
impl PcRelativeImmediate for ImmI33_12 {
    fn try_from_offset(offset: i32) -> Result<Self, BadImmediateValue> {
        Self::try_from(i64::from(offset))
    }
}
impl PcRelativeImmediate for ImmU12_0 {
    fn try_from_offset(offset: i32) -> Result<Self, BadImmediateValue> {
        let value = u16::try_from(offset)
            .map_err(|_| BadImmediateValue::new(offset, "ImmU12_0".to_string()))?;
        Self::try_from(value)
    }
}

fn pc_relative_imm<I: PcRelativeImmediate>(
    offset: i32,
    op: &'static str,
) -> Result<I, TranspilerError> {
    I::try_from_offset(offset).map_err(|source| TranspilerError::PcRelativeOffsetOutOfRange {
        op,
        offset,
        source,
    })
}

impl From<Arm64NativeOp> for UnresOp<Arm64Op> {
    fn from(op: Arm64NativeOp) -> Self {
        UnresOp::Native(op)
    }
}

impl NativeOp for Arm64NativeOp {
    fn size_in_bytes(&self) -> u32 {
        N_BYTES_IN_WORD as u32
    }
    fn encode(&self, out: &mut Vec<u8>) {
        // Every ARM64 instruction is a single 32-bit little-endian word.
        out.extend_from_slice(&self.arm64_encoding().to_le_bytes());
    }
    #[cfg(test)]
    fn as_assembly(&self) -> String {
        self.arm64_as_asm_string()
    }
}

#[derive(Clone)]
pub(crate) struct Arm64Op;
impl TranspiledOp for Arm64Op {
    type Native = Arm64NativeOp;
    type Symbolic = Arm64SymbolicOp;
    const PC_RELATIVE_FROM_END: bool = false;

    fn new_branch(target: SymbolicBranchTarget) -> Arm64SymbolicOp {
        Arm64SymbolicOp::B(target)
    }
    fn branch_size_in_bytes() -> u32 {
        N_BYTES_IN_WORD as u32
    }

    /// Narrows the resolved offset to each instruction's immediate field, rejecting offsets
    /// the field cannot hold instead of letting the encoder mask them down to a wrong target.
    fn sym_to_native(
        sym_op: Arm64SymbolicOp,
        pc_relative_offset: PcRelativeOffset,
    ) -> Result<Arm64NativeOp, TranspilerError> {
        let offset = pc_relative_offset.as_i32();
        Ok(match sym_op {
            Arm64SymbolicOp::B(_) => Arm64NativeOp::B(pc_relative_imm(offset, "B")?),
            Arm64SymbolicOp::Bcc(_, cond) => {
                Arm64NativeOp::Bcc(pc_relative_imm(offset, "B.cond")?, cond)
            }
            Arm64SymbolicOp::Cbz_W(reg, _) => {
                Arm64NativeOp::Cbz_W(reg, pc_relative_imm(offset, "CBZ")?)
            }
            Arm64SymbolicOp::Cbnz_W(reg, _) => {
                Arm64NativeOp::Cbnz_W(reg, pc_relative_imm(offset, "CBNZ")?)
            }
            Arm64SymbolicOp::Adrp_xRegTmp_Label(_) => {
                Arm64NativeOp::Adrp_xRegTmp_Label(pc_relative_imm(offset, "ADRP")?)
            }
            Arm64SymbolicOp::Adr_xRegTmp(_) => {
                Arm64NativeOp::Adr_xRegTmp(pc_relative_imm(offset, "ADR")?)
            }
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(_) => {
                Arm64NativeOp::Add_xRegTmp_xRegTmp_Lo12Label(pc_relative_imm(offset, "ADD :lo12:")?)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::assert_that;
    use googletest::prelude::{anything, eq, err, ok, pat};

    fn resolve(sym_op: Arm64SymbolicOp, offset: i32) -> Result<Arm64NativeOp, TranspilerError> {
        Arm64Op::sym_to_native(sym_op, PcRelativeOffset::new(offset))
    }

    fn any_target() -> SymbolicBranchTarget {
        LocalCodeLabel::SmallSkip.into()
    }

    #[test]
    fn conditional_branch_accepts_its_full_range() {
        let target = any_target();
        assert_that!(
            resolve(
                Arm64SymbolicOp::Bcc(target, ConditionCode::Eq),
                ImmI21_2::MIN_VAL
            ),
            ok(pat!(Arm64NativeOp::Bcc(
                pat!(ImmI21_2(eq(&ImmI21_2::MIN_VAL))),
                anything()
            )))
        );
        assert_that!(
            resolve(
                Arm64SymbolicOp::Cbz_W(ArmRegister::X0, target),
                ImmI21_2::MAX_VAL - 3
            ),
            ok(anything())
        );
    }

    #[test]
    fn conditional_branch_rejects_offsets_beyond_one_megabyte() {
        let target = any_target();
        for (sym_op, op_name) in [
            (Arm64SymbolicOp::Bcc(target, ConditionCode::Eq), "B.cond"),
            (Arm64SymbolicOp::Cbz_W(ArmRegister::X0, target), "CBZ"),
            (Arm64SymbolicOp::Cbnz_W(ArmRegister::X0, target), "CBNZ"),
        ] {
            for offset in [ImmI21_2::MIN_VAL - 4, ImmI21_2::MAX_VAL + 1] {
                assert_that!(
                    resolve(sym_op.clone(), offset),
                    err(pat!(TranspilerError::PcRelativeOffsetOutOfRange {
                        op: eq(&op_name),
                        offset: eq(&offset),
                        ..
                    }))
                );
            }
        }
    }

    #[test]
    fn misaligned_branch_offset_is_rejected() {
        assert_that!(
            resolve(Arm64SymbolicOp::B(any_target()), 2),
            err(pat!(TranspilerError::PcRelativeOffsetOutOfRange {
                op: eq(&"B"),
                ..
            }))
        );
    }

    #[test]
    fn unconditional_branch_rejects_offsets_beyond_128_megabytes() {
        assert_that!(
            resolve(Arm64SymbolicOp::B(any_target()), ImmI28_2::MAX_VAL + 1),
            err(pat!(TranspilerError::PcRelativeOffsetOutOfRange {
                op: eq(&"B"),
                ..
            }))
        );
        assert_that!(
            resolve(Arm64SymbolicOp::B(any_target()), ImmI28_2::MAX_VAL - 3),
            ok(anything())
        );
    }

    #[test]
    fn adr_accepts_its_full_range_and_rejects_beyond_it() {
        for offset in [ImmI21_0::MIN_VAL, ImmI21_0::MAX_VAL] {
            assert_that!(
                resolve(
                    Arm64SymbolicOp::Adr_xRegTmp(GlobalDataLabel::JumpTable),
                    offset
                ),
                ok(pat!(Arm64NativeOp::Adr_xRegTmp(pat!(ImmI21_0(eq(
                    &offset
                ))))))
            );
        }
        for offset in [ImmI21_0::MIN_VAL - 1, ImmI21_0::MAX_VAL + 1] {
            assert_that!(
                resolve(
                    Arm64SymbolicOp::Adr_xRegTmp(GlobalDataLabel::JumpTable),
                    offset
                ),
                err(pat!(TranspilerError::PcRelativeOffsetOutOfRange {
                    op: eq(&"ADR"),
                    offset: eq(&offset),
                    ..
                }))
            );
        }
    }

    #[test]
    fn adrp_accepts_page_aligned_offsets_only() {
        const PAGE_SIZE: i32 = 0x1000;
        for offset in [
            i32::MIN,
            -PAGE_SIZE,
            0,
            PAGE_SIZE,
            i32::MAX - (PAGE_SIZE - 1),
        ] {
            let widened = i64::from(offset);
            assert_that!(
                resolve(
                    Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::JumpTable),
                    offset
                ),
                ok(pat!(Arm64NativeOp::Adrp_xRegTmp_Label(pat!(ImmI33_12(
                    eq(&widened)
                )))))
            );
        }
        for offset in [1, PAGE_SIZE - 1, PAGE_SIZE + 1, -1] {
            assert_that!(
                resolve(
                    Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::JumpTable),
                    offset
                ),
                err(pat!(TranspilerError::PcRelativeOffsetOutOfRange {
                    op: eq(&"ADRP"),
                    offset: eq(&offset),
                    ..
                }))
            );
        }
    }

    #[test]
    fn lo12_immediate_accepts_page_offsets_only() {
        assert_that!(
            resolve(
                Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::JumpTable),
                0xFFF
            ),
            ok(pat!(Arm64NativeOp::Add_xRegTmp_xRegTmp_Lo12Label(pat!(
                ImmU12_0(eq(&0xFFF))
            ))))
        );
        assert_that!(
            resolve(
                Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::JumpTable),
                0x1000
            ),
            err(pat!(TranspilerError::PcRelativeOffsetOutOfRange {
                op: eq(&"ADD :lo12:"),
                ..
            }))
        );
    }
}
