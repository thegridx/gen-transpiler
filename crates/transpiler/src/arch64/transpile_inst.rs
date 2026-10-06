//! Translates single RV32IMA instructions into aarch64 ops.
//!
//! Guest registers are 32-bit values in W registers (see `registers`). An instruction that uses
//! `t3` to `t6` loads them from the host stack into `REG_TMP` or `REG_TMP_2` and stores the
//! result back. Guest addresses are offsets from `.original_risc_prog` (the `RoDataStart`
//! label), so a memory access adds the guest address to that label's native address.
//!
//! In untrusted mode, loads, stores, atomics and `JALR` check their target first and branch to
//! the block's `PanicIndirection` label when a check fails. That label is the `B ExitPanic` in
//! the gas trampoline at the start of each pseudo basic block: `B.cond` reaches only ±1 MiB,
//! while `B` reaches ±128 MiB. A block's gas is charged up front, assuming no check fails.
//!
//! Two encoding rules shape the code. In the immediate forms of `ADDS`, `SUBS` and `CMP`,
//! Rn = 31 means `WSP`, so guest x0 (`WZR`) is never passed there. Panic and illegal exits are
//! entered with `B`, not `BL`, so x30, which holds the guest `ra`, survives until it is saved.
use riscv32::N_BYTES_IN_WORD;
use riscv32::common::SignType;
use riscv32::immediate::{ImmU5_0, ImmU12_0, Immediate as _};
use riscv32::ops::{
    ArithmeticOp, AtomicOp, BasicOp, BranchCondition, DivRemOp, LoadSize, MulDivRemOp, MulHighOp,
    RegArithmeticOp, RvAtomic, RvAuipc, RvBranch, RvImmI12, RvImmS12, RvJal, RvJalr, RvLoad, RvLui,
    RvMulDivRem, RvOpImm, RvOpReg, RvStore, ShiftOp, StoreSize, TwoRegistersAndImmediate,
};
use riscv32_elf::types::{RiscvAddress, VirtualDataRange};

use crate::arch64::ops::{
    Arm64NativeOp, Arm64Op, Arm64SymbolicOp, ConditionCode, ImmI9_0, ImmI12_0, ImmU5_4, ImmU6_0,
    ImmU14_2, ImmU15_3,
};
use crate::arch64::registers::{
    ArmMappedRegister, ArmMemoryMappedRegister, ArmRegister, REG_TMP, REG_TMP_2,
    RISC_MEMORY_MAPPED_REGISTER_PAIRS,
};
use crate::layout::{GlobalCodeLabel, GlobalDataLabel, LocalCodeLabel, SymbolicBranchTarget};
use crate::traits::{InstructionTranspiler, UnresOp};

// Callers pass a non-negative `imm`.
fn immi12_to_immu12(imm: ImmI12_0) -> ImmU12_0 {
    ImmU12_0(imm.value() as u16)
}
// Callers pass a negative `imm`; its magnitude, at most 2048, fits in 12 bits.
fn neg_immi12_to_immu12(imm: ImmI12_0) -> ImmU12_0 {
    ImmU12_0((-imm.value()) as u16)
}
// Both are 12-bit signed immediates.
fn rvimmi12_to_immi12(imm: RvImmI12) -> ImmI12_0 {
    ImmI12_0(imm.value())
}
// Both hold a 12-bit signed value; `RvImmS12` stores it in an `i32`.
fn rvimms12_to_immi12(imm: RvImmS12) -> ImmI12_0 {
    ImmI12_0(imm.value() as i16)
}

/// `reg_d = reg_1 + imm`. Uses `MOV` when `reg_1` is x0, since `ADDS`/`SUBS` (immediate) would
/// read `WSP`. The `MOV` sets no flags.
fn adds_or_subs_or_mov(reg_d: ArmRegister, reg_1: ArmRegister, imm: ImmI12_0) -> Arm64NativeOp {
    if reg_1 == ArmRegister::Zr {
        Arm64NativeOp::Mov_W_Imm(reg_d, imm)
    } else {
        if imm.value() >= 0 {
            Arm64NativeOp::Adds_W_W_Imm(reg_d, reg_1, immi12_to_immu12(imm))
        } else {
            Arm64NativeOp::Subs_W_W_Imm(reg_d, reg_1, neg_immi12_to_immu12(imm))
        }
    }
}

/// The number of valid start addresses in `range`, whose `end` is the last valid start address
/// for the access (inclusive). A bounds check subtracts this from the address's offset into the
/// range and panics when the result is not negative, so an access may start at `end` but not
/// past it. The value can exceed 12 bits, so checks subtract it in two `SUBS`: bits 12-23
/// shifted by 12, then bits 0-11.
fn n_valid_start_addresses(range: VirtualDataRange) -> u32 {
    range.size().as_u32().saturating_add(1)
}

fn adds_or_subs_x_reg_tmp(imm: ImmI12_0) -> Arm64NativeOp {
    if imm.value() >= 0 {
        Arm64NativeOp::Adds_xRegTmp_xRegTmp_Imm(immi12_to_immu12(imm))
    } else {
        Arm64NativeOp::Subs_xRegTmp_xRegTmp_Imm(neg_immi12_to_immu12(imm))
    }
}

/// Returns `arm_reg_tmp` and the op that loads the stack-held guest register into it.
fn load_rv32_t_reg(
    arm_reg_tmp: ArmRegister,
    non_mapped_r: ArmMemoryMappedRegister,
) -> (ArmRegister, Vec<Arm64NativeOp>) {
    (
        arm_reg_tmp,
        vec![Arm64NativeOp::Ldr_W_xSp_Imm(
            arm_reg_tmp,
            ImmU14_2(non_mapped_r as u16 * N_BYTES_IN_WORD as u16),
        )],
    )
}

/// Returns `arm_reg_tmp` and the op that stores it to the stack-held guest register. The caller
/// writes the result to `arm_reg_tmp` first.
fn store_rv32_t_reg(
    arm_reg_tmp: ArmRegister,
    non_mapped_r: ArmMemoryMappedRegister,
) -> (ArmRegister, Vec<Arm64NativeOp>) {
    (
        arm_reg_tmp,
        vec![Arm64NativeOp::Str_W_xSp_Imm(
            arm_reg_tmp,
            ImmU14_2(non_mapped_r as u16 * N_BYTES_IN_WORD as u16),
        )],
    )
}

/// `MOVZ` of `imm`, or of `!imm` when `imm` is negative. Callers then use the inverting form
/// (`EON`, `ORN`, `BIC`) for a negative `imm`.
fn movzw_imm_or_not_imm(reg_d: ArmRegister, imm: i32) -> Arm64NativeOp {
    Arm64NativeOp::Movz_W_Imm_Shift(
        reg_d,
        (if imm >= 0 { imm } else { !imm }) as u16,
        ImmU5_4(0),
    )
}

/// Emits the tail that, given `xRegTmp` pointing at a jump-table entry, follows the entry's
/// self-relative offset to the target transpiled address and branches there.
///
/// In untrusted mode each 8-byte entry starts with the gas cost of the rest of the block, so
/// this first deducts it, panicking on underflow, and steps 4 bytes to the offset. In trusted
/// mode an entry is just the 4-byte offset.
pub(crate) fn jump_via_jump_table_stub(trusted: bool) -> Vec<UnresOp<Arm64Op>> {
    // Untrusted only: deduct the entry's gas cost, then step xRegTmp to the offset field.
    let mut ops = if trusted {
        vec![]
    } else {
        vec![
            Arm64NativeOp::Ldur_W_xRegTmp_Imm(REG_TMP_2, ImmI9_0(0)).into(),
            // Loading the 32-bit cost into w28 zeroes the top half of x28, so the 64-bit
            // subtraction is correct.
            Arm64NativeOp::Subs_xRegGas_xRegGas_X(REG_TMP_2).into(),
            Arm64SymbolicOp::Bcc(LocalCodeLabel::SmallSkip.into(), ConditionCode::Pl).into(),
            Arm64SymbolicOp::B(LocalCodeLabel::PanicIndirection.into()).into(),
            UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_Imm(ImmU12_0(4)).into(),
        ]
    };
    // Shared tail: load the self-relative offset at xRegTmp, add it to the entry address, jump.
    ops.extend([
        Arm64NativeOp::Ldur_W_xRegTmp_Imm(REG_TMP_2, ImmI9_0(0)).into(),
        Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(REG_TMP_2).into(),
        Arm64NativeOp::Br_xRegTmp.into(),
    ]);
    ops
}

fn unres_native_ops(ops: Vec<Arm64NativeOp>) -> Vec<UnresOp<Arm64Op>> {
    ops.into_iter()
        .map(Into::<UnresOp<Arm64Op>>::into)
        .collect()
}

/// Host stack bytes for the AMO scratch slot, which holds the native address across an AMO.
/// 8 bytes are used; 16 keep `sp` 16-byte aligned.
pub(crate) const RESERVED_STACK_BYTES_AMO_SCRATCH: i16 = 16;
/// Host stack bytes for the stack-held guest registers `t3` to `t6`.
pub(crate) const RESERVED_STACK_BYTES_MEMORY_MAPPED_REGISTERS: i16 =
    RISC_MEMORY_MAPPED_REGISTER_PAIRS.len() as i16 * 8;

/// Translates RISC-V instructions into aarch64 ops.
pub(crate) struct Arm64InstructionTranspiler {}

impl InstructionTranspiler<Arm64Op> for Arm64InstructionTranspiler {
    fn transpile_op_imm(inst: &RvOpImm) -> Vec<UnresOp<Arm64Op>> {
        let (reg_1, header) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let (reg_d, footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let middle = match inst.op() {
            ArithmeticOp::Basic(BasicOp::Add) => {
                vec![adds_or_subs_or_mov(
                    reg_d,
                    reg_1,
                    rvimmi12_to_immi12(inst.imm()),
                )]
            }
            // aarch64 logical immediates are bit patterns, not arbitrary values, so the
            // immediate goes through REG_TMP.
            ArithmeticOp::Basic(BasicOp::Xor) => vec![
                movzw_imm_or_not_imm(REG_TMP, inst.imm().value() as i32),
                if inst.imm().value() >= 0 {
                    Arm64NativeOp::Eor_W_W_W(reg_d, reg_1, REG_TMP)
                } else {
                    Arm64NativeOp::Eon_W_W_W(reg_d, reg_1, REG_TMP)
                },
            ],
            ArithmeticOp::Basic(BasicOp::Or) => vec![
                movzw_imm_or_not_imm(REG_TMP, inst.imm().value() as i32),
                if inst.imm().value() >= 0 {
                    Arm64NativeOp::Orr_W_W_W(reg_d, reg_1, REG_TMP)
                } else {
                    Arm64NativeOp::Orn_W_W_W(reg_d, reg_1, REG_TMP)
                },
            ],
            ArithmeticOp::Basic(BasicOp::And) => vec![
                movzw_imm_or_not_imm(REG_TMP, inst.imm().value() as i32),
                if inst.imm().value() >= 0 {
                    Arm64NativeOp::And_W_W_W(reg_d, reg_1, REG_TMP)
                } else {
                    Arm64NativeOp::Bic_W_W_W(reg_d, reg_1, REG_TMP)
                },
            ],
            ArithmeticOp::Shift(ShiftOp::Sll) => {
                vec![Arm64NativeOp::Lsl_W_W_Imm(
                    reg_d,
                    reg_1,
                    ImmU5_0(inst.imm().value() as u8),
                )]
            }
            ArithmeticOp::Shift(ShiftOp::Srl) => {
                vec![Arm64NativeOp::Lsr_W_W_Imm(
                    reg_d,
                    reg_1,
                    ImmU5_0(inst.imm().value() as u8),
                )]
            }
            ArithmeticOp::Shift(ShiftOp::Sra) => {
                vec![Arm64NativeOp::Asr_W_W_Imm(
                    reg_d,
                    reg_1,
                    ImmU5_0(inst.imm().value() as u8),
                )]
            }
            // `CMP Wn, #imm` reads Rn = 31 as WSP, so x0 cannot be its operand. Comparing 0
            // with a constant has a constant result: signed 0 < imm, or unsigned 0 < imm
            // (sign-extended), which holds for every non-zero imm.
            ArithmeticOp::SetLessThan(sign_type) if reg_1 == ArmRegister::Zr => {
                let imm = inst.imm().value();
                let is_less = match sign_type {
                    SignType::Signed => imm > 0,
                    SignType::Unsigned => imm != 0,
                };
                vec![Arm64NativeOp::Mov_W_Imm(
                    reg_d,
                    ImmI12_0(i16::from(is_less)),
                )]
            }
            ArithmeticOp::SetLessThan(SignType::Signed) => vec![
                Arm64NativeOp::Cmp_W_Imm(reg_1, ImmI12_0(inst.imm().value())),
                Arm64NativeOp::Cset_W(reg_d, ConditionCode::Lt),
            ],
            ArithmeticOp::SetLessThan(SignType::Unsigned) => vec![
                Arm64NativeOp::Cmp_W_Imm(reg_1, ImmI12_0(inst.imm().value())),
                Arm64NativeOp::Cset_W(reg_d, ConditionCode::Lo),
            ],
        };
        [
            unres_native_ops(header),
            unres_native_ops(middle),
            unres_native_ops(footer),
        ]
        .concat()
    }

    fn transpile_op_reg(inst: &RvOpReg) -> Vec<UnresOp<Arm64Op>> {
        let (reg_1, header1) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP, non_mapped_r),
        };
        let (reg_2, header2) = match ArmMappedRegister::from(inst.rs2()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let (reg_d, footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP, non_mapped_r),
        };
        let middle = match inst.op() {
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::Add)) => {
                vec![Arm64NativeOp::Adds_W_W_W(reg_d, reg_1, reg_2)]
            }
            RegArithmeticOp::Sub => vec![Arm64NativeOp::Subs_W_W_W(reg_d, reg_1, reg_2)],
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::Xor)) => {
                vec![Arm64NativeOp::Eor_W_W_W(reg_d, reg_1, reg_2)]
            }
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::Or)) => {
                vec![Arm64NativeOp::Orr_W_W_W(reg_d, reg_1, reg_2)]
            }
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::And)) => {
                vec![Arm64NativeOp::And_W_W_W(reg_d, reg_1, reg_2)]
            }
            RegArithmeticOp::NonSub(ArithmeticOp::Shift(ShiftOp::Sll)) => {
                vec![Arm64NativeOp::Lsl_W_W_W(reg_d, reg_1, reg_2)]
            }
            RegArithmeticOp::NonSub(ArithmeticOp::Shift(ShiftOp::Srl)) => {
                vec![Arm64NativeOp::Lsr_W_W_W(reg_d, reg_1, reg_2)]
            }
            RegArithmeticOp::NonSub(ArithmeticOp::Shift(ShiftOp::Sra)) => {
                vec![Arm64NativeOp::Asr_W_W_W(reg_d, reg_1, reg_2)]
            }
            RegArithmeticOp::NonSub(ArithmeticOp::SetLessThan(SignType::Signed)) => vec![
                Arm64NativeOp::Cmp_W_W(reg_1, reg_2),
                Arm64NativeOp::Cset_W(reg_d, ConditionCode::Lt),
            ],
            RegArithmeticOp::NonSub(ArithmeticOp::SetLessThan(SignType::Unsigned)) => vec![
                Arm64NativeOp::Cmp_W_W(reg_1, reg_2),
                Arm64NativeOp::Cset_W(reg_d, ConditionCode::Lo),
            ],
        };
        [
            unres_native_ops(header1),
            unres_native_ops(header2),
            unres_native_ops(middle),
            unres_native_ops(footer),
        ]
        .concat()
    }

    fn transpile_lui(inst: &RvLui) -> Vec<UnresOp<Arm64Op>> {
        let (reg_d, footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let ret = if (0..0x10_000).contains(&inst.imm().value()) {
            vec![Arm64NativeOp::Movz_W_Imm_Shift(
                reg_d,
                inst.imm().value() as u16,
                ImmU5_4(0),
            )]
        } else {
            vec![
                Arm64NativeOp::Movz_W_Imm_Shift(
                    reg_d,
                    ((inst.imm().value() >> 16) & 0xFFFF) as u16,
                    ImmU5_4(16),
                ),
                Arm64NativeOp::Movk_W_Imm_Shift(
                    reg_d,
                    (inst.imm().value() & 0xFFFF) as u16,
                    ImmU5_4(0),
                ),
            ]
        };
        [unres_native_ops(ret), unres_native_ops(footer)].concat()
    }

    fn transpile_auipc(inst: &RvAuipc, target: RiscvAddress) -> Vec<UnresOp<Arm64Op>> {
        let (reg_d, footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let ret = vec![
            Arm64NativeOp::Movz_W_Imm_Shift(
                reg_d,
                ((target.as_u32() >> 16) & 0xFFFF) as u16,
                ImmU5_4(16),
            ),
            Arm64NativeOp::Movk_W_Imm_Shift(reg_d, (target.as_u32() & 0xFFFF) as u16, ImmU5_4(0)),
        ];
        [unres_native_ops(ret), unres_native_ops(footer)].concat()
    }

    // RV32M. Division by zero gives all ones and remainder by zero gives the dividend, as the
    // RISC-V spec requires. For signed overflow (`MIN / -1`), `SDIV` already returns `MIN` and
    // the remainder comes out as 0, matching the spec.
    fn transpile_mul_div_rem(
        inst: &RvMulDivRem,
        next_place: RiscvAddress,
    ) -> Vec<UnresOp<Arm64Op>> {
        let (reg_1, header1) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP, non_mapped_r),
        };
        let (reg_2, header2) = match ArmMappedRegister::from(inst.rs2()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let (reg_d, footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP, non_mapped_r),
        };
        let middle = match inst.op() {
            MulDivRemOp::MulLow => {
                unres_native_ops(vec![Arm64NativeOp::Mul_W_W_W(reg_d, reg_1, reg_2)])
            }
            MulDivRemOp::MulHigh(MulHighOp::Mulh) => unres_native_ops(vec![
                Arm64NativeOp::Smull_xRegTmp_W_W(reg_1, reg_2),
                Arm64NativeOp::Lsr_X_xRegTmp_32(reg_d),
            ]),
            MulDivRemOp::MulHigh(MulHighOp::Mulhu) => unres_native_ops(vec![
                Arm64NativeOp::Umull_xRegTmp_W_W(reg_1, reg_2),
                Arm64NativeOp::Lsr_X_xRegTmp_32(reg_d),
            ]),
            MulDivRemOp::MulHigh(MulHighOp::Mulhsu) => unres_native_ops(vec![
                Arm64NativeOp::Sxtw(REG_TMP, reg_1),
                // Writing a W register zero-extends the upper 32 bits of X.
                Arm64NativeOp::Mov_W_W(REG_TMP_2, reg_2),
                Arm64NativeOp::Mul_xRegTmp_xRegTmp_xRegTmp2,
                Arm64NativeOp::Lsr_X_xRegTmp_32(reg_d),
            ]),
            MulDivRemOp::DivRem(DivRemOp::Div(SignType::Signed)) => [
                vec![
                    Arm64SymbolicOp::Cbz_W(reg_2, LocalCodeLabel::SmallSkip.into()).into(),
                    Arm64NativeOp::Sdiv_W_W_W(reg_d, reg_1, reg_2).into(),
                ],
                unres_native_ops(footer.clone()),
                vec![
                    Arm64SymbolicOp::B(next_place.into()).into(),
                    UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
                    Arm64NativeOp::Mov_W_Imm(reg_d, ImmI12_0(-1)).into(),
                ],
            ]
            .concat(),
            MulDivRemOp::DivRem(DivRemOp::Div(SignType::Unsigned)) => [
                vec![
                    Arm64SymbolicOp::Cbz_W(reg_2, LocalCodeLabel::SmallSkip.into()).into(),
                    Arm64NativeOp::Udiv_W_W_W(reg_d, reg_1, reg_2).into(),
                ],
                unres_native_ops(footer.clone()),
                vec![
                    Arm64SymbolicOp::B(next_place.into()).into(),
                    UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
                    Arm64NativeOp::Mov_W_Imm(reg_d, ImmI12_0(-1)).into(),
                ],
            ]
            .concat(),
            // rem = rs1 - (rs1 / rs2) * rs2. The quotient lands in REG_TMP, so a stack-held rs1
            // is reloaded before the subtraction.
            MulDivRemOp::DivRem(DivRemOp::Rem(SignType::Signed)) => [
                vec![
                    Arm64SymbolicOp::Cbz_W(reg_2, LocalCodeLabel::SmallSkip.into()).into(),
                    Arm64NativeOp::Sdiv_W_W_W(REG_TMP, reg_1, reg_2).into(),
                    Arm64NativeOp::Mul_W_W_W(REG_TMP_2, REG_TMP, reg_2).into(),
                ],
                unres_native_ops(header1.clone()),
                unres_native_ops(vec![Arm64NativeOp::Subs_W_W_W(reg_d, reg_1, REG_TMP_2)]),
                unres_native_ops(footer.clone()),
                vec![
                    Arm64SymbolicOp::B(next_place.into()).into(),
                    UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
                    Arm64NativeOp::Mov_W_W(reg_d, reg_1).into(),
                ],
            ]
            .concat(),
            MulDivRemOp::DivRem(DivRemOp::Rem(SignType::Unsigned)) => [
                vec![
                    Arm64SymbolicOp::Cbz_W(reg_2, LocalCodeLabel::SmallSkip.into()).into(),
                    Arm64NativeOp::Udiv_W_W_W(REG_TMP, reg_1, reg_2).into(),
                    Arm64NativeOp::Mul_W_W_W(REG_TMP_2, REG_TMP, reg_2).into(),
                ],
                unres_native_ops(header1.clone()),
                unres_native_ops(vec![Arm64NativeOp::Subs_W_W_W(reg_d, reg_1, REG_TMP_2)]),
                unres_native_ops(footer.clone()),
                vec![
                    Arm64SymbolicOp::B(next_place.into()).into(),
                    UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
                    Arm64NativeOp::Mov_W_W(reg_d, reg_1).into(),
                ],
            ]
            .concat(),
        };
        [
            unres_native_ops(header1),
            unres_native_ops(header2),
            middle,
            unres_native_ops(footer),
        ]
        .concat()
    }

    fn transpile_jal(
        inst: &RvJal,
        target: RiscvAddress,
        next_place: RiscvAddress,
    ) -> Vec<UnresOp<Arm64Op>> {
        let (reg_d, pre_footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let mut ret = Vec::new();
        if reg_d != ArmRegister::Zr {
            ret.extend(unres_native_ops(vec![
                Arm64NativeOp::Movz_W_Imm_Shift(
                    reg_d,
                    ((next_place.as_u32() >> 16) & 0xFFFF) as u16,
                    ImmU5_4(16),
                ),
                Arm64NativeOp::Movk_W_Imm_Shift(
                    reg_d,
                    (next_place.as_u32() & 0xFFFF) as u16,
                    ImmU5_4(0),
                ),
            ]));
        };
        ret.extend(unres_native_ops(pre_footer));
        ret.push(Arm64SymbolicOp::B(target.into()).into());
        ret
    }

    fn transpile_branch(inst: &RvBranch, target: SymbolicBranchTarget) -> Vec<UnresOp<Arm64Op>> {
        let (reg_1, header1) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP, non_mapped_r),
        };
        let (reg_2, header2) = match ArmMappedRegister::from(inst.rs2()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let middle = match inst.condition() {
            BranchCondition::Beq => {
                if reg_2 == ArmRegister::Zr {
                    vec![Arm64SymbolicOp::Cbz_W(reg_1, target).into()]
                } else if reg_1 == ArmRegister::Zr {
                    vec![Arm64SymbolicOp::Cbz_W(reg_2, target).into()]
                } else {
                    vec![
                        Arm64NativeOp::Cmp_W_W(reg_1, reg_2).into(),
                        Arm64SymbolicOp::Bcc(target, ConditionCode::Eq).into(),
                    ]
                }
            }
            BranchCondition::Bne => {
                if reg_2 == ArmRegister::Zr {
                    vec![Arm64SymbolicOp::Cbnz_W(reg_1, target).into()]
                } else if reg_1 == ArmRegister::Zr {
                    vec![Arm64SymbolicOp::Cbnz_W(reg_2, target).into()]
                } else {
                    vec![
                        Arm64NativeOp::Cmp_W_W(reg_1, reg_2).into(),
                        Arm64SymbolicOp::Bcc(target, ConditionCode::Ne).into(),
                    ]
                }
            }
            BranchCondition::Blt => vec![
                Arm64NativeOp::Cmp_W_W(reg_1, reg_2).into(),
                Arm64SymbolicOp::Bcc(target, ConditionCode::Lt).into(),
            ],
            BranchCondition::Bge => vec![
                Arm64NativeOp::Cmp_W_W(reg_1, reg_2).into(),
                Arm64SymbolicOp::Bcc(target, ConditionCode::Ge).into(),
            ],
            BranchCondition::Bltu => vec![
                Arm64NativeOp::Cmp_W_W(reg_1, reg_2).into(),
                Arm64SymbolicOp::Bcc(target, ConditionCode::Lo).into(),
            ],
            BranchCondition::Bgeu => vec![
                Arm64NativeOp::Cmp_W_W(reg_1, reg_2).into(),
                Arm64SymbolicOp::Bcc(target, ConditionCode::Hs).into(),
            ],
        };
        [unres_native_ops(header1), unres_native_ops(header2), middle].concat()
    }

    fn transpile_jalr(
        inst: &RvJalr,
        next_place: RiscvAddress,
        risc_code_end: RiscvAddress,
        trusted: bool,
    ) -> Vec<UnresOp<Arm64Op>> {
        let (reg_1, header) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let (reg_d, pre_footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let n_risc_instructions = risc_code_end.as_u32() >> 2;
        // RISC-V JALR clears bit 0 of `rs1 + imm` and, with IALIGN=32, traps with
        // instruction-address-misaligned when bit 1 of the result is set. Both modes trap on
        // bit 1; bit 0 is dropped by the `Asr #2` that turns the target into a jump-table index.
        let mut middle = if trusted {
            // Trusted: no range check, and no gas trampoline holding `PanicIndirection`, so the
            // alignment trap branches to `ExitPanic` directly. With the 4-byte stride,
            // xRegTmp = JumpTable + ((rs1 + imm) & ~3). The jump table is segment-aligned, so
            // the low two bits of the sum are those of the target.
            vec![
                Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::JumpTable).into(),
                Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::JumpTable).into(),
                Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(reg_1).into(),
                adds_or_subs_x_reg_tmp(rvimmi12_to_immi12(inst.imm())).into(),
                Arm64NativeOp::Tst_W_2(REG_TMP).into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::SmallSkip.into(), ConditionCode::Eq).into(),
                Arm64SymbolicOp::B(GlobalCodeLabel::ExitPanic.into()).into(),
                UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
                Arm64NativeOp::Asr_xRegTmp_xRegTmp_Imm(ImmU6_0(2)).into(),
                Arm64NativeOp::Lsl_xRegTmp_xRegTmp_Imm(ImmU6_0(2)).into(),
            ]
        } else {
            vec![
                // Lower bound: rs1 + imm must not be negative. With rs1 = x0 the sum is the
                // constant imm, so either panic outright or skip the check (`MOV` sets no flags).
                if reg_1 == ArmRegister::Zr && inst.imm().value() < 0 {
                    Arm64SymbolicOp::B(LocalCodeLabel::PanicIndirection.into()).into()
                } else {
                    adds_or_subs_or_mov(REG_TMP, reg_1, rvimmi12_to_immi12(inst.imm())).into()
                },
                if reg_1 == ArmRegister::Zr {
                    Arm64NativeOp::Nop.into()
                } else {
                    Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Mi)
                        .into()
                },
                // Alignment: bit 1 set means a misaligned instruction address.
                Arm64NativeOp::Tst_W_2(REG_TMP).into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Ne)
                    .into(),
                // Drop bits 0 and 1 to get the instruction index.
                Arm64NativeOp::Asr_W_W_Imm(REG_TMP, REG_TMP, ImmU5_0(2)).into(),
                // Upper bound: the index may be at most the exit sentinel's.
                Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0(
                    (((n_risc_instructions + 1) >> 12) & 0xFFF) as u16,
                ))
                .into(),
                Arm64NativeOp::Subs_W_W_Imm(
                    REG_TMP,
                    REG_TMP,
                    ImmU12_0(((n_risc_instructions + 1) & 0xFFF) as u16),
                )
                .into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Pl)
                    .into(),
                Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::JumpTable).into(),
                Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::JumpTable).into(),
                // xRegTmp = JumpTable + 2 * ((rs1 + imm) & ~3), the target's 8-byte entry:
                // ((JumpTable / 2 + target) >> 2) << 3. This needs JumpTable / 2 to be a
                // multiple of 4, which holds because the jump table is segment-aligned.
                Arm64NativeOp::Asr_xRegTmp_xRegTmp_Imm(ImmU6_0(1)).into(),
                Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(reg_1).into(),
                adds_or_subs_x_reg_tmp(rvimmi12_to_immi12(inst.imm())).into(),
                Arm64NativeOp::Asr_xRegTmp_xRegTmp_Imm(ImmU6_0(2)).into(),
                Arm64NativeOp::Lsl_xRegTmp_xRegTmp_Imm(ImmU6_0(3)).into(),
            ]
        };
        if reg_d != ArmRegister::Zr {
            middle.extend(unres_native_ops(vec![
                Arm64NativeOp::Movz_W_Imm_Shift(
                    reg_d,
                    ((next_place.as_u32() >> 16) & 0xFFFF) as u16,
                    ImmU5_4(16),
                ),
                Arm64NativeOp::Movk_W_Imm_Shift(
                    reg_d,
                    (next_place.as_u32() & 0xFFFF) as u16,
                    ImmU5_4(0),
                ),
            ]));
        }
        [
            unres_native_ops(header),
            middle,
            unres_native_ops(pre_footer),
            jump_via_jump_table_stub(trusted),
        ]
        .concat()
    }

    fn transpile_load(
        inst: &RvLoad,
        readable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<Arm64Op>> {
        let (reg_1, header) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let (reg_d, footer) = match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let load_op: Arm64NativeOp = match inst.size() {
            LoadSize::OneByte(SignType::Signed) => Arm64NativeOp::LdrsbW_xRegTmp_0(reg_d),
            LoadSize::OneByte(SignType::Unsigned) => Arm64NativeOp::LdrbW_xRegTmp_0(reg_d),
            LoadSize::HalfWord(SignType::Signed) => Arm64NativeOp::LdurshW_xRegTmp_0(reg_d),
            LoadSize::HalfWord(SignType::Unsigned) => Arm64NativeOp::LdurhW_xRegTmp_0(reg_d),
            LoadSize::OneWord => Arm64NativeOp::Ldur_W_xRegTmp_Imm(reg_d, ImmI9_0(0)),
        };
        // Untrusted only: bounds-check the access. The lower bound of `readable_range` is 0, so
        // the check only rejects a negative address.
        let mut middle = if trusted {
            vec![]
        } else {
            vec![
                // Lower bound, handled as in `transpile_jalr`.
                if reg_1 == ArmRegister::Zr && inst.imm().value() < 0 {
                    Arm64SymbolicOp::B(LocalCodeLabel::PanicIndirection.into()).into()
                } else {
                    adds_or_subs_or_mov(REG_TMP, reg_1, rvimmi12_to_immi12(inst.imm())).into()
                },
                if reg_1 == ArmRegister::Zr {
                    Arm64NativeOp::Nop.into()
                } else {
                    Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Mi)
                        .into()
                },
                // Upper bound.
                Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0(
                    (n_valid_start_addresses(readable_range) >> 12) as u16,
                ))
                .into(),
                Arm64NativeOp::Subs_W_W_Imm(
                    REG_TMP,
                    REG_TMP,
                    ImmU12_0((n_valid_start_addresses(readable_range) & 0xFFF) as u16),
                )
                .into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Pl)
                    .into(),
            ]
        };
        // Translate the guest address to a native address and load.
        middle.extend([
            Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::RoDataStart).into(),
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::RoDataStart).into(),
            adds_or_subs_x_reg_tmp(rvimmi12_to_immi12(inst.imm())).into(),
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(reg_1).into(),
            load_op.into(),
        ]);
        [unres_native_ops(header), middle, unres_native_ops(footer)].concat()
    }

    fn transpile_store(
        inst: &RvStore,
        writable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<Arm64Op>> {
        let (reg_1, header) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        let (reg_2, mid_header) = match ArmMappedRegister::from(inst.rs2()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        // Untrusted only: bounds-check the access.
        let mut middle = if trusted {
            vec![]
        } else {
            vec![
                // Lower bound: address - start must not be negative.
                adds_or_subs_or_mov(REG_TMP, reg_1, rvimms12_to_immi12(inst.imm())).into(),
                Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0(
                    (writable_range.start().as_u32() >> 12) as u16,
                ))
                .into(),
                Arm64NativeOp::Subs_W_W_Imm(
                    REG_TMP,
                    REG_TMP,
                    ImmU12_0((writable_range.start().as_u32() & 0xFFF) as u16),
                )
                .into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Mi)
                    .into(),
                // Upper bound.
                Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0(
                    (n_valid_start_addresses(writable_range) >> 12) as u16,
                ))
                .into(),
                Arm64NativeOp::Subs_W_W_Imm(
                    REG_TMP,
                    REG_TMP,
                    ImmU12_0((n_valid_start_addresses(writable_range) & 0xFFF) as u16),
                )
                .into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Pl)
                    .into(),
            ]
        };
        // Translate the guest address to a native address. A stack-held rs2 is loaded after
        // this, since rs1 may have used REG_TMP_2 too.
        middle.extend([
            Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::RoDataStart).into(),
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::RoDataStart).into(),
            adds_or_subs_x_reg_tmp(rvimms12_to_immi12(inst.imm())).into(),
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(reg_1).into(),
        ]);
        let footer = vec![match inst.size() {
            StoreSize::OneByte => Arm64NativeOp::Strb_W_xRegTmp_0(reg_2),
            StoreSize::HalfWord => Arm64NativeOp::Sturh_W_xRegTmp_0(reg_2),
            StoreSize::OneWord => Arm64NativeOp::Stur_W_xRegTmp_Imm(reg_2, ImmI9_0(0)),
        }];
        [
            unres_native_ops(header),
            middle,
            unres_native_ops(mid_header),
            unres_native_ops(footer),
        ]
        .concat()
    }

    fn transpile_system_call(next_place: RiscvAddress) -> Vec<UnresOp<Arm64Op>> {
        // Pass the resume address in REG_TMP_2; `EcallExit` checks the ecall number in a7, and
        // `SaveLocalState` stores the address and returns to the host.
        vec![
            Arm64NativeOp::Movz_W_Imm_Shift(
                REG_TMP_2,
                ((next_place.as_u32() >> 16) & 0xFFFF) as u16,
                ImmU5_4(16),
            )
            .into(),
            Arm64NativeOp::Movk_W_Imm_Shift(
                REG_TMP_2,
                (next_place.as_u32() & 0xFFFF) as u16,
                ImmU5_4(0),
            )
            .into(),
            Arm64SymbolicOp::B(GlobalCodeLabel::EcallExit.into()).into(),
        ]
    }

    fn transpile_atomic(
        inst: &RvAtomic,
        readable_range: VirtualDataRange,
        writable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<Arm64Op>> {
        // LR and SC are a plain load and store, after the same 4-byte alignment check AMOs make.
        let lr_sc_alignment_check = || {
            if trusted {
                return vec![];
            }
            let (reg_1, header) = match ArmMappedRegister::from(inst.rs1()) {
                ArmMappedRegister::Register(r) => (r, vec![]),
                ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
            };
            let mut ops = unres_native_ops(header);
            ops.extend([
                Arm64NativeOp::Bic_W_W_3(REG_TMP, reg_1).into(),
                Arm64NativeOp::Cmp_W_W(REG_TMP, reg_1).into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Ne)
                    .into(),
            ]);
            ops
        };
        if matches!(inst.op(), AtomicOp::Sc) {
            // Transpiled code is single-threaded, so SC always succeeds: store rs2, then set
            // rd to 0. The store comes first in case rd == rs2.
            let mut ret = lr_sc_alignment_check();
            ret.extend(Self::transpile_store(
                &RvStore::from(inst),
                writable_range,
                trusted,
            ));
            ret.extend(Self::transpile_op_imm(&RvOpImm::new(
                ArithmeticOp::Basic(BasicOp::And),
                TwoRegistersAndImmediate::new(inst.rd(), inst.rd(), RvImmI12(0)),
            )));
            return ret;
        }
        if matches!(inst.op(), AtomicOp::Lr) {
            let mut ret = lr_sc_alignment_check();
            ret.extend(Self::transpile_load(
                &RvLoad::from(inst),
                readable_range,
                trusted,
            ));
            return ret;
        }
        // An AMO computes `old = M[rs1]; M[rs1] = op(old, rs2); rd = old`. With only two
        // scratch registers, shared with stack-held operands and the address check, and with
        // rs1, rs2 and rd possibly equal, it runs as:
        // 1. Check that rs1 is 4-aligned and writable, and put the native address in REG_TMP.
        // 2. Save the address in the stack scratch slot and load `old` into REG_TMP_2.
        // 3. Read rs2 into REG_TMP, then write `old` to rd (rs2 is read first in case rd == rs2).
        // 4. Compute `op(old, rs2)` into REG_TMP_2.
        // 5. Reload the address into REG_TMP and store REG_TMP_2.
        let (reg_1, get_1) = match ArmMappedRegister::from(inst.rs1()) {
            ArmMappedRegister::Register(r) => (r, vec![]),
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP_2, non_mapped_r),
        };
        // Untrusted only: check 4-byte alignment and bounds. The writable range lies inside the
        // readable one, so one check covers both the read and the write.
        let mut verify_m_at_reg_1 = if trusted {
            vec![]
        } else {
            vec![
                Arm64NativeOp::Bic_W_W_3(REG_TMP, reg_1).into(),
                Arm64NativeOp::Cmp_W_W(REG_TMP, reg_1).into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Ne)
                    .into(),
                // Lower bound.
                Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0(
                    (writable_range.start().as_u32() >> 12) as u16,
                ))
                .into(),
                Arm64NativeOp::Subs_W_W_Imm(
                    REG_TMP,
                    REG_TMP,
                    ImmU12_0((writable_range.start().as_u32() & 0xFFF) as u16),
                )
                .into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Mi)
                    .into(),
                // Upper bound.
                Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0(
                    (n_valid_start_addresses(writable_range) >> 12) as u16,
                ))
                .into(),
                Arm64NativeOp::Subs_W_W_Imm(
                    REG_TMP,
                    REG_TMP,
                    ImmU12_0((n_valid_start_addresses(writable_range) & 0xFFF) as u16),
                )
                .into(),
                Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Pl)
                    .into(),
            ]
        };
        // Translate the guest address to a native address.
        verify_m_at_reg_1.extend([
            Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::RoDataStart).into(),
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::RoDataStart).into(),
        ]);
        let load_reg_tmp_2_from_mem_and_save_addr = unres_native_ops(vec![
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(reg_1),
            // A stack-held rs1 no longer needs REG_TMP_2 after this.
            Arm64NativeOp::Str_xRegTmp_xSp_Imm(ImmU15_3(
                RESERVED_STACK_BYTES_MEMORY_MAPPED_REGISTERS as u16,
            )),
            Arm64NativeOp::Ldur_W_xRegTmp_Imm(REG_TMP_2, ImmI9_0(0)),
        ]);
        let mov_reg_2_to_reg_tmp = unres_native_ops(match ArmMappedRegister::from(inst.rs2()) {
            ArmMappedRegister::Register(r) => {
                vec![Arm64NativeOp::Mov_W_W(REG_TMP, r)]
            }
            ArmMappedRegister::Memory(non_mapped_r) => load_rv32_t_reg(REG_TMP, non_mapped_r).1,
        });
        let mov_reg_tmp2_to_reg_d = unres_native_ops(match ArmMappedRegister::from(inst.rd()) {
            ArmMappedRegister::Register(r) => {
                vec![Arm64NativeOp::Mov_W_W(r, REG_TMP_2)]
            }
            ArmMappedRegister::Memory(non_mapped_r) => store_rv32_t_reg(REG_TMP_2, non_mapped_r).1,
        });
        let ops = unres_native_ops(match inst.op() {
            AtomicOp::AmoSwap => {
                vec![Arm64NativeOp::Mov_W_W(REG_TMP_2, REG_TMP)]
            }
            AtomicOp::AmoAdd => {
                vec![Arm64NativeOp::Adds_W_W_W(REG_TMP_2, REG_TMP_2, REG_TMP)]
            }
            AtomicOp::AmoXor => {
                vec![Arm64NativeOp::Eor_W_W_W(REG_TMP_2, REG_TMP_2, REG_TMP)]
            }
            AtomicOp::AmoOr => vec![Arm64NativeOp::Orr_W_W_W(REG_TMP_2, REG_TMP_2, REG_TMP)],
            AtomicOp::AmoAnd => {
                vec![Arm64NativeOp::And_W_W_W(REG_TMP_2, REG_TMP_2, REG_TMP)]
            }
            _ => vec![
                Arm64NativeOp::Cmp_W_W(REG_TMP_2, REG_TMP),
                Arm64NativeOp::Csel_wRegTmp2_wRegTmp2_wRegTmp(match inst.op() {
                    AtomicOp::AmoMin => ConditionCode::Lt,
                    AtomicOp::AmoMax => ConditionCode::Gt,
                    AtomicOp::AmoMinU => ConditionCode::Lo,
                    AtomicOp::AmoMaxU => ConditionCode::Hi,
                    _ => unreachable!(),
                }),
            ],
        });
        [
            unres_native_ops(get_1),
            verify_m_at_reg_1,
            load_reg_tmp_2_from_mem_and_save_addr,
            mov_reg_2_to_reg_tmp,
            mov_reg_tmp2_to_reg_d,
            ops,
            unres_native_ops(vec![
                Arm64NativeOp::Ldr_xRegTmp_X_Imm(
                    ArmRegister::Sp,
                    ImmU15_3(RESERVED_STACK_BYTES_MEMORY_MAPPED_REGISTERS as u16),
                ),
                Arm64NativeOp::Stur_W_xRegTmp_Imm(REG_TMP_2, ImmI9_0(0)),
            ]),
        ]
        .concat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riscv32::ops::{BranchFunct3, RvImmB13, ThreeRegisters};
    use riscv32::registers::RiscvRegister;
    use riscv32_elf::types::VirtualDataPointer;
    use strum::IntoEnumIterator as _;

    /// Source registers covering x0, a register-mapped register and a memory-mapped one.
    const SOURCES: [RiscvRegister; 3] = [RiscvRegister::Zero, RiscvRegister::A0, RiscvRegister::T3];
    /// Destinations the dispatcher can pass for ops whose `rd == x0` is skipped upstream.
    const DESTINATIONS: [RiscvRegister; 2] = [RiscvRegister::A1, RiscvRegister::T4];

    fn data_range() -> VirtualDataRange {
        VirtualDataRange::new(
            VirtualDataPointer::new(0x1000),
            VirtualDataPointer::new(0x8000),
        )
    }

    /// Returns the ops that encode `Zr` in an Rn slot where register 31 means WSP.
    fn ops_reading_wsp(ops: &[UnresOp<Arm64Op>]) -> Vec<String> {
        ops.iter()
            .filter_map(|op| match op {
                UnresOp::Native(
                    native @ (Arm64NativeOp::Adds_W_W_Imm(_, ArmRegister::Zr, _)
                    | Arm64NativeOp::Subs_W_W_Imm(_, ArmRegister::Zr, _)
                    | Arm64NativeOp::Cmp_W_Imm(ArmRegister::Zr, _)),
                ) => Some(format!("{native:?}")),
                _ => None,
            })
            .collect()
    }

    fn assert_no_wsp_reads(what: &str, ops: &[UnresOp<Arm64Op>]) {
        let offending = ops_reading_wsp(ops);
        assert!(
            offending.is_empty(),
            "{what} reads WSP instead of WZR: {offending:?}"
        );
    }

    fn op_imm_cases() -> Vec<(ArithmeticOp, i16)> {
        let mut cases = Vec::new();
        for op in BasicOp::iter() {
            for imm in [0, 1, -1, 2047, -2048] {
                cases.push((ArithmeticOp::Basic(op), imm));
            }
        }
        for sign_type in [SignType::Signed, SignType::Unsigned] {
            for imm in [0, 1, -1, 2047, -2048] {
                cases.push((ArithmeticOp::SetLessThan(sign_type), imm));
            }
        }
        for shamt in [0, 1, 31] {
            cases.push((ArithmeticOp::Shift(ShiftOp::Sll), shamt));
            cases.push((ArithmeticOp::Shift(ShiftOp::Srl), shamt));
            cases.push((ArithmeticOp::Shift(ShiftOp::Sra), 0x400 | shamt));
        }
        cases
    }

    fn mul_div_rem_ops() -> Vec<MulDivRemOp> {
        let mut ops = vec![
            MulDivRemOp::MulLow,
            MulDivRemOp::MulHigh(MulHighOp::Mulh),
            MulDivRemOp::MulHigh(MulHighOp::Mulhu),
            MulDivRemOp::MulHigh(MulHighOp::Mulhsu),
        ];
        for sign_type in [SignType::Signed, SignType::Unsigned] {
            ops.push(MulDivRemOp::DivRem(DivRemOp::Div(sign_type)));
            ops.push(MulDivRemOp::DivRem(DivRemOp::Rem(sign_type)));
        }
        ops
    }

    #[test]
    fn op_imm_never_reads_wsp() {
        for (op, imm) in op_imm_cases() {
            for rs1 in SOURCES {
                for rd in DESTINATIONS {
                    let inst =
                        RvOpImm::new(op, TwoRegistersAndImmediate::new(rs1, rd, RvImmI12(imm)));
                    assert_no_wsp_reads(
                        &format!("{inst:?}"),
                        &Arm64InstructionTranspiler::transpile_op_imm(&inst),
                    );
                }
            }
        }
    }

    #[test]
    fn op_reg_never_reads_wsp() {
        let mut ops = vec![RegArithmeticOp::Sub];
        ops.extend(
            op_imm_cases()
                .into_iter()
                .map(|(op, _)| RegArithmeticOp::NonSub(op)),
        );
        for op in ops {
            for rs1 in SOURCES {
                for rs2 in SOURCES {
                    for rd in DESTINATIONS {
                        let inst = RvOpReg::new(op, ThreeRegisters::new(rs1, rs2, rd));
                        assert_no_wsp_reads(
                            &format!("{inst:?}"),
                            &Arm64InstructionTranspiler::transpile_op_reg(&inst),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mul_div_rem_never_reads_wsp() {
        for op in mul_div_rem_ops() {
            for rs1 in SOURCES {
                for rs2 in SOURCES {
                    for rd in DESTINATIONS {
                        let inst = RvMulDivRem::new(op, ThreeRegisters::new(rs1, rs2, rd));
                        assert_no_wsp_reads(
                            &format!("{inst:?}"),
                            &Arm64InstructionTranspiler::transpile_mul_div_rem(
                                &inst,
                                RiscvAddress::new(8),
                            ),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn atomics_never_read_wsp() {
        for op in AtomicOp::iter() {
            for rs1 in SOURCES {
                for rs2 in SOURCES {
                    for rd in SOURCES {
                        for trusted in [false, true] {
                            let inst = RvAtomic::new(op, ThreeRegisters::new(rs1, rs2, rd));
                            assert_no_wsp_reads(
                                &format!("{inst:?} (trusted: {trusted})"),
                                &Arm64InstructionTranspiler::transpile_atomic(
                                    &inst,
                                    data_range(),
                                    data_range(),
                                    trusted,
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn loads_stores_and_jumps_never_read_wsp() {
        for rs1 in SOURCES {
            for rs2 in SOURCES {
                for trusted in [false, true] {
                    for imm in [0, 4, -4] {
                        let load = RvLoad::new(
                            LoadSize::OneWord,
                            TwoRegistersAndImmediate::new(rs1, rs2, RvImmI12(imm)),
                        );
                        assert_no_wsp_reads(
                            &format!("{load:?}"),
                            &Arm64InstructionTranspiler::transpile_load(
                                &load,
                                data_range(),
                                trusted,
                            ),
                        );
                        let store = RvStore::new(
                            StoreSize::OneWord,
                            TwoRegistersAndImmediate::new(rs1, rs2, RvImmS12(i32::from(imm))),
                        );
                        assert_no_wsp_reads(
                            &format!("{store:?}"),
                            &Arm64InstructionTranspiler::transpile_store(
                                &store,
                                data_range(),
                                trusted,
                            ),
                        );
                        let jalr =
                            RvJalr::new(TwoRegistersAndImmediate::new(rs1, rs2, RvImmI12(imm)));
                        assert_no_wsp_reads(
                            &format!("{jalr:?}"),
                            &Arm64InstructionTranspiler::transpile_jalr(
                                &jalr,
                                RiscvAddress::new(8),
                                RiscvAddress::new(0x100),
                                trusted,
                            ),
                        );
                    }
                }
                for condition in BranchFunct3::iter() {
                    let branch = RvBranch::new(
                        condition,
                        TwoRegistersAndImmediate::new(rs1, rs2, RvImmB13(8)),
                    );
                    assert_no_wsp_reads(
                        &format!("{branch:?}"),
                        &Arm64InstructionTranspiler::transpile_branch(
                            &branch,
                            RiscvAddress::new(8).into(),
                        ),
                    );
                }
            }
        }
    }
}
