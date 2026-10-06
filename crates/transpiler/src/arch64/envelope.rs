//! The aarch64 envelope: the code that enters and leaves the transpiled program, and the
//! passes that resolve symbolic branches.
//!
//! The envelope follows the program's code, in this order: the exits (`ExitIllegalOpcode`,
//! which the last instruction falls through to, then `ExitPanic` and `ExitNormal`),
//! `SaveLocalState`, the return to the host, the entry label `StartTranspiledCode`, the guest
//! state load, the dispatch on a7, and a table of `B` instructions to the init functions and
//! entry points. Each exit puts its exit value in w7 (guest a7); exits and ecalls then save
//! the guest state in `.register_store` and return to the host.
//!
//! While guest code runs, the host stack holds the callee-saved host registers (96 bytes) and,
//! below them, 32 reserved bytes: `t3` to `t6` at `[sp, #0]` to `[sp, #12]` and the AMO
//! scratch slot at `[sp, #16]`.
use object::elf;
use riscv32::gas_cost::RvGasCost;
use riscv32::immediate::{ImmU5_0, ImmU12_0};
use riscv32::registers::{RiscvRegister, RvPreservedRegister};
use riscv32_elf::{
    data_layout::{NO_ECALL_RETURN_ADDRESS, StoredRegister},
    types::RiscvAddress,
};
use strum::IntoEnumIterator as _;
use user_code_runner::runner::{
    ILLEGAL_INSTRUCTION_EXIT_VALUE, NORMAL_EXIT_VALUE, PANIC_EXIT_VALUE,
};

use crate::arch64::{
    ops::{
        Arm64NativeOp, Arm64Op, Arm64SymbolicOp, ConditionCode, ImmI9_0, ImmI9_2, ImmI12_0,
        ImmI13_0, ImmU5_4,
    },
    registers::{
        ARM_GLOBAL_PRESERVED_REGISTER_PAIRS, ArmMappedRegister, ArmRegister, REG_GAS, REG_TMP,
        REG_TMP_2, RISC_MEMORY_MAPPED_REGISTER_PAIRS,
    },
    transpile_inst::{
        RESERVED_STACK_BYTES_AMO_SCRATCH, RESERVED_STACK_BYTES_MEMORY_MAPPED_REGISTERS,
        jump_via_jump_table_stub,
    },
};
use crate::{
    TranspilerError,
    layout::{GlobalCodeLabel, GlobalDataLabel, LocalCodeLabel, SymbolicBranchTarget},
    traits::{PcRelativeOffset, TranspiledEnvelope, UnresOp},
    transpiled_address::{AddressResolveOutcome, TranspiledAddress, TranspiledAddressResolver},
};

/// Stack bytes reserved while guest code runs: the stack-held guest registers and the AMO
/// scratch slot (32 bytes, keeping `sp` 16-byte aligned).
const RESERVED_STACK_SIZE_IN_BYTES: i16 =
    RESERVED_STACK_BYTES_MEMORY_MAPPED_REGISTERS + RESERVED_STACK_BYTES_AMO_SCRATCH;

/// `MOVZ` + `MOVK`: loads a 32-bit constant into `reg`.
fn mov_w_u32(reg: ArmRegister, value: u32) -> [UnresOp<Arm64Op>; 2] {
    [
        Arm64NativeOp::Movz_W_Imm_Shift(reg, ((value >> 16) & 0xFFFF) as u16, ImmU5_4(16)).into(),
        Arm64NativeOp::Movk_W_Imm_Shift(reg, (value & 0xFFFF) as u16, ImmU5_4(0)).into(),
    ]
}

/// Emits the aarch64 envelope and resolves aarch64 branches.
pub(crate) struct Arm64Envelope;

impl TranspiledEnvelope<Arm64Op> for Arm64Envelope {
    fn branch_to_illegal_code() -> Vec<UnresOp<Arm64Op>> {
        // `B`, not `BL`: x30 holds the guest's `ra`, which the exit saves in the register store.
        vec![Arm64SymbolicOp::B(GlobalCodeLabel::ExitIllegalOpcode.into()).into()]
    }

    fn deduct_gas_and_panic_trampoline(cost: RvGasCost) -> Vec<UnresOp<Arm64Op>> {
        vec![
            Arm64NativeOp::Subs_xRegGas_xRegGas_Imm(ImmU12_0(cost.value())).into(),
            Arm64SymbolicOp::Bcc(LocalCodeLabel::SmallSkip.into(), ConditionCode::Pl).into(),
            // Failed checks in this block branch here (`PanicIndirection`), because `B.cond` and
            // `CBZ` reach only ±1 MiB while `B` reaches ±128 MiB. `B`, not `BL`: x30 holds the
            // guest's `ra`, which the exit saves in the register store.
            UnresOp::Label(LocalCodeLabel::PanicIndirection.into()),
            Arm64SymbolicOp::B(GlobalCodeLabel::ExitPanic.into()).into(),
            UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
        ]
    }

    fn exit_user_code() -> Vec<UnresOp<Arm64Op>> {
        // `ExitIllegalOpcode` comes first, so running off the end of the program lands on it.
        // `ExitNormal`, the target of the jump table's exit sentinel, falls through into
        // `SaveLocalState`. None of these exits is an ecall, so each sets the return address
        // (x28) to `NO_ECALL_RETURN_ADDRESS`.
        let exit = |label: GlobalCodeLabel, exit_value: i32| {
            let mut ops = vec![
                UnresOp::Label(label.into()),
                Arm64NativeOp::Mov_W_Imm(ArmRegister::X7, ImmI12_0(exit_value as i16)).into(),
            ];
            ops.extend(mov_w_u32(REG_TMP_2, NO_ECALL_RETURN_ADDRESS));
            ops
        };
        let to_save = || Arm64SymbolicOp::B(GlobalCodeLabel::SaveLocalState.into()).into();
        let mut ops = exit(
            GlobalCodeLabel::ExitIllegalOpcode,
            ILLEGAL_INSTRUCTION_EXIT_VALUE,
        );
        ops.push(to_save());
        ops.extend(exit(GlobalCodeLabel::ExitPanic, PANIC_EXIT_VALUE));
        ops.push(to_save());
        ops.extend(exit(GlobalCodeLabel::ExitNormal, NORMAL_EXIT_VALUE));
        ops
    }

    fn save_local_state() -> Vec<UnresOp<Arm64Op>> {
        let mut ret = vec![
            UnresOp::Label(GlobalCodeLabel::SaveLocalState.into()),
            Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::RegisterStore).into(),
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::RegisterStore).into(),
        ];
        // First, store the guest registers held in host registers.
        for pr in RvPreservedRegister::iter() {
            match ArmMappedRegister::from(RiscvRegister::from(pr)) {
                ArmMappedRegister::Register(arm_reg) => {
                    ret.push(
                        Arm64NativeOp::Stur_W_xRegTmp_Imm(
                            arm_reg,
                            ImmI9_0(StoredRegister::Riscv32Reg(pr).get_offset() as i16),
                        )
                        .into(),
                    );
                }
                ArmMappedRegister::Memory(_) => {}
            }
        }
        // Then copy the stack-held guest registers to the store, a pair at a time, and zero
        // their stack slots so no guest value stays on the host stack. x30 and x29 serve as
        // scratch: their guest values (`ra`, `s0`) are already stored, and the host's values
        // are restored from the stack before returning.
        for (i, (pr1, pr2)) in RISC_MEMORY_MAPPED_REGISTER_PAIRS.iter().enumerate() {
            ret.extend(vec![
                Arm64NativeOp::Ldp_W_W_xSp_Imm(
                    ArmRegister::Lr,
                    ArmRegister::Fp,
                    ImmI9_2(i as i16 * 8),
                )
                .into(),
                Arm64NativeOp::Stp_W_W_xSp_Imm(
                    ArmRegister::Zr,
                    ArmRegister::Zr,
                    ImmI9_2(i as i16 * 8),
                )
                .into(),
                Arm64NativeOp::Stur_W_xRegTmp_Imm(
                    ArmRegister::Lr,
                    ImmI9_0(StoredRegister::Riscv32Reg(*pr1).get_offset() as i16),
                )
                .into(),
                Arm64NativeOp::Stur_W_xRegTmp_Imm(
                    ArmRegister::Fp,
                    ImmI9_0(StoredRegister::Riscv32Reg(*pr2).get_offset() as i16),
                )
                .into(),
            ]);
        }
        // Release the reserved stack bytes, so `sp` points at the saved host registers again.
        ret.extend(vec![
            Arm64NativeOp::Add_xSp_Imm(ImmI13_0::new(RESERVED_STACK_SIZE_IN_BYTES)).into(),
            // After an ecall, REG_TMP_2 holds the RISC-V address to resume at.
            Arm64NativeOp::Stur_W_xRegTmp_Imm(
                REG_TMP_2,
                ImmI9_0(StoredRegister::ReturnAddress.get_offset() as i16),
            )
            .into(),
            Arm64NativeOp::Stur_X_xRegTmp_Imm(
                REG_GAS,
                ImmI9_0(StoredRegister::Gas.get_offset() as i16),
            )
            .into(),
        ]);
        ret
    }

    fn load_global_state_and_ret() -> Vec<UnresOp<Arm64Op>> {
        let mut ret = ARM_GLOBAL_PRESERVED_REGISTER_PAIRS
            .iter()
            .rev()
            .map(|(r1, r2)| Arm64NativeOp::Ldp_X_X_xSp_adv16(*r1, *r2).into())
            .collect::<Vec<_>>();
        // Return to the host.
        ret.push(Arm64NativeOp::Ret_Lr.into());
        ret
    }

    fn ecall_exit() -> Vec<UnresOp<Arm64Op>> {
        // a7 + 2 <= 2, unsigned, exactly when a7 is -2, -1 or 0: an exit code, not an ecall.
        vec![
            UnresOp::Label(GlobalCodeLabel::EcallExit.into()),
            Arm64NativeOp::Adds_W_W_Imm(REG_TMP, ArmRegister::X7, ImmU12_0(2)).into(),
            Arm64NativeOp::Cmp_W_Imm(REG_TMP, ImmI12_0(2)).into(),
            Arm64SymbolicOp::Bcc(GlobalCodeLabel::ExitIllegalOpcode.into(), ConditionCode::Ls)
                .into(),
            // Save the ecall number, which a resume restores to a7.
            Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::RegisterStore).into(),
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::RegisterStore).into(),
            Arm64NativeOp::Stur_W_xRegTmp_Imm(
                ArmRegister::X7,
                ImmI9_0(StoredRegister::EcallNumber.get_offset() as i16),
            )
            .into(),
            Arm64SymbolicOp::B(GlobalCodeLabel::SaveLocalState.into()).into(),
        ]
    }

    fn save_global_state() -> Vec<UnresOp<Arm64Op>> {
        // The host calls this label.
        let mut ret = vec![UnresOp::Label(GlobalCodeLabel::StartTranspiledCode.into())];
        // Push the callee-saved host registers in pairs, keeping `sp` 16-byte aligned.
        ret.extend(
            ARM_GLOBAL_PRESERVED_REGISTER_PAIRS
                .iter()
                .flat_map(|(r1, r2)| vec![Arm64NativeOp::Stp_X_X_xSp_minus16(*r1, *r2).into()]),
        );
        ret
    }

    fn load_local_state() -> Vec<UnresOp<Arm64Op>> {
        let mut ret: Vec<UnresOp<Arm64Op>> = vec![
            Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::RegisterStore).into(),
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::RegisterStore).into(),
            // Reserve stack for the stack-held guest registers and the AMO scratch slot.
            Arm64NativeOp::Add_xSp_Imm(ImmI13_0::new(-RESERVED_STACK_SIZE_IN_BYTES)).into(),
        ];
        // First, copy the stack-held guest registers from the store to the stack, a pair at a
        // time. x30 and x29 serve as scratch: the host's values are already pushed, and the
        // guest's values (`ra`, `s0`) are loaded below.
        for (i, (pr1, pr2)) in RISC_MEMORY_MAPPED_REGISTER_PAIRS.iter().enumerate() {
            ret.extend(vec![
                Arm64NativeOp::Ldur_W_xRegTmp_Imm(
                    ArmRegister::Lr,
                    ImmI9_0(StoredRegister::Riscv32Reg(*pr1).get_offset() as i16),
                )
                .into(),
                Arm64NativeOp::Ldur_W_xRegTmp_Imm(
                    ArmRegister::Fp,
                    ImmI9_0(StoredRegister::Riscv32Reg(*pr2).get_offset() as i16),
                )
                .into(),
                Arm64NativeOp::Stp_W_W_xSp_Imm(
                    ArmRegister::Lr,
                    ArmRegister::Fp,
                    ImmI9_2(i as i16 * 8),
                )
                .into(),
            ]);
        }
        // Then load the guest registers held in host registers.
        for pr in RvPreservedRegister::iter() {
            match ArmMappedRegister::from(RiscvRegister::from(pr)) {
                ArmMappedRegister::Register(arm_reg) => {
                    ret.push(
                        Arm64NativeOp::Ldur_W_xRegTmp_Imm(
                            arm_reg,
                            ImmI9_0(StoredRegister::Riscv32Reg(pr).get_offset() as i16),
                        )
                        .into(),
                    );
                }
                ArmMappedRegister::Memory(_) => {}
            }
        }
        // Load the RISC-V address to resume at after an ecall into REG_TMP_2.
        ret.extend(vec![
            Arm64NativeOp::Ldur_W_xRegTmp_Imm(
                REG_TMP_2,
                ImmI9_0(StoredRegister::ReturnAddress.get_offset() as i16),
            )
            .into(),
            Arm64NativeOp::Ldur_X_xRegTmp_Imm(
                REG_GAS,
                ImmI9_0(StoredRegister::Gas.get_offset() as i16),
            )
            .into(),
        ]);
        ret
    }

    fn enter_user_code(
        end_text_section: RiscvAddress,
        n_hooks: u32,
        trusted: bool,
    ) -> Vec<UnresOp<Arm64Op>> {
        let panic: SymbolicBranchTarget = GlobalCodeLabel::ExitPanic.into();
        // X28 (REG_TMP_2) holds the RISC-V address to resume at. The jump-table entry's byte
        // offset is (address / 4) * stride: address * 2 for the untrusted 8-byte stride (two
        // adds of X28), and the address itself for the trusted 4-byte stride (one add).
        let mut ecall_return_stub = vec![
            Arm64SymbolicOp::Cbnz_W(ArmRegister::X7, GlobalCodeLabel::NotEcallReturn.into()).into(),
            // Give the guest back its a7, the ecall number. xRegTmp still points at the
            // register store, from `load_local_state`.
            Arm64NativeOp::Ldur_W_xRegTmp_Imm(
                ArmRegister::X7,
                ImmI9_0(StoredRegister::EcallNumber.get_offset() as i16),
            )
            .into(),
            // w7 == 0: return from an ecall and resume through the jump table. The address
            // comes from the host-writable register store, so check that it is 4-aligned and
            // at most `end_text_section` (the exit sentinel) before using it as an index.
            Arm64NativeOp::Bic_W_W_3(REG_TMP, ArmRegister::X28).into(),
            Arm64NativeOp::Cmp_W_W(REG_TMP, ArmRegister::X28).into(),
            Arm64SymbolicOp::Bcc(panic, ConditionCode::Ne).into(),
        ];
        ecall_return_stub.extend(mov_w_u32(REG_TMP, end_text_section.as_u32()));
        ecall_return_stub.extend([
            Arm64NativeOp::Cmp_W_W(ArmRegister::X28, REG_TMP).into(),
            Arm64SymbolicOp::Bcc(panic, ConditionCode::Hi).into(),
            // Resuming at the end of the code is like falling off the last instruction.
            Arm64SymbolicOp::Bcc(GlobalCodeLabel::ExitIllegalOpcode.into(), ConditionCode::Eq)
                .into(),
            Arm64SymbolicOp::Adrp_xRegTmp_Label(GlobalDataLabel::JumpTable).into(),
            Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(GlobalDataLabel::JumpTable).into(),
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(ArmRegister::X28).into(),
        ]);
        if !trusted {
            ecall_return_stub.push(Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(ArmRegister::X28).into());
        }

        // Otherwise call an init function or entry point. Its `ra` is `end_text_section`, so
        // its final return goes through the exit sentinel to `ExitNormal`.
        let mut not_ecall_return_stub =
            vec![UnresOp::Label(GlobalCodeLabel::NotEcallReturn.into())];
        not_ecall_return_stub.extend(mov_w_u32(ArmRegister::Lr, end_text_section.as_u32()));
        // w7 holds the 1-based selector; compute the 0-based index in x28, which is free on a
        // fresh call, so the guest sees a7 = selector. The host supplies the selector, so reject
        // an index of `n_hooks` or more; the unsigned compare also rejects a negative selector.
        // Each entry in the table is one 4-byte `B`.
        not_ecall_return_stub.push(
            Arm64NativeOp::Subs_W_W_Imm(ArmRegister::X28, ArmRegister::X7, ImmU12_0(1)).into(),
        );
        not_ecall_return_stub.extend(mov_w_u32(REG_TMP, n_hooks));
        not_ecall_return_stub.extend([
            Arm64NativeOp::Cmp_W_W(ArmRegister::X28, REG_TMP).into(),
            Arm64SymbolicOp::Bcc(panic, ConditionCode::Hs).into(),
            Arm64NativeOp::Lsl_W_W_Imm(ArmRegister::X28, ArmRegister::X28, ImmU5_0(2)).into(),
            Arm64SymbolicOp::Adr_xRegTmp(GlobalDataLabel::EntryPointsHooksBegin).into(),
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(ArmRegister::X28).into(),
            Arm64NativeOp::Br_xRegTmp.into(),
        ]);

        [
            ecall_return_stub.as_slice(),
            jump_via_jump_table_stub(trusted).as_slice(),
            not_ecall_return_stub.as_slice(),
        ]
        .concat()
    }

    fn elf_arch_indicator() -> u16 {
        elf::EM_AARCH64
    }

    /// Forward pass: resolves branches to labels and RISC-V addresses, and the label operands
    /// of `ADRP`, `ADR` and `ADD :lo12:`. `PanicIndirection` resolves to the nearest preceding
    /// `B ExitPanic`, normally the one in the current block's gas trampoline.
    fn forward_pass_convert_branches(
        ops: &mut [UnresOp<Arm64Op>],
        address_resolver: &TranspiledAddressResolver<'_>,
    ) -> Result<TranspiledAddress, TranspilerError> {
        let mut last_panic_branch_addr: Option<TranspiledAddress> = None;
        let mut cur_pos = TranspiledAddress::new(0);
        let panic_addr = address_resolver.resolve_label(&GlobalCodeLabel::ExitPanic.into())?;

        for op in ops.iter_mut() {
            if let Some(pc_rel) = match op {
                UnresOp::Symbolic(Arm64SymbolicOp::B(SymbolicBranchTarget::Label(
                    GlobalCodeLabel::ExitPanic,
                ))) => {
                    last_panic_branch_addr = Some(cur_pos);
                    Some(PcRelativeOffset::from_to(cur_pos, panic_addr))
                }
                // `SmallSkip` targets are left for the backward pass.
                UnresOp::Symbolic(Arm64SymbolicOp::B(target))
                | UnresOp::Symbolic(Arm64SymbolicOp::Bcc(target, _))
                | UnresOp::Symbolic(Arm64SymbolicOp::Cbz_W(_, target))
                | UnresOp::Symbolic(Arm64SymbolicOp::Cbnz_W(_, target)) => {
                    match address_resolver.resolve(
                        target,
                        LocalCodeLabel::PanicIndirection,
                        &last_panic_branch_addr,
                    )? {
                        AddressResolveOutcome::Resolved(target_addr) => {
                            Some(PcRelativeOffset::from_to(cur_pos, target_addr))
                        }
                        AddressResolveOutcome::NotApplicable => None,
                    }
                }
                // Convert ADRP label to computed page offset (immediate is target_page - cur_page)
                UnresOp::Symbolic(Arm64SymbolicOp::Adrp_xRegTmp_Label(label)) => {
                    let target_addr = address_resolver.resolve_label(&(*label).into())?;
                    // The whole image is at most 8 MiB, so the page offset fits in an `i32`.
                    let cur_page = cur_pos.as_u32() & !0xFFF;
                    let target_page = target_addr.as_u32() & !0xFFF;
                    let offset = TranspiledAddress::new(target_page)
                        .checked_sub(cur_page)?
                        .as_i32();
                    Some(PcRelativeOffset::new(offset))
                }
                // Convert ADD :lo12: label to computed lo12 immediate (the absolute lo12 of target)
                UnresOp::Symbolic(Arm64SymbolicOp::Add_xRegTmp_xRegTmp_Lo12Label(label)) => {
                    let target_addr = address_resolver.resolve_label(&(*label).into())?;
                    Some(PcRelativeOffset::new((target_addr.as_u32() & 0xFFF) as i32))
                }
                // Convert ADR label to computed PC-relative offset
                UnresOp::Symbolic(Arm64SymbolicOp::Adr_xRegTmp(label)) => {
                    let target_addr = address_resolver.resolve_label(&(*label).into())?;
                    Some(PcRelativeOffset::from_to(cur_pos, target_addr))
                }
                _ => None,
            } {
                op.sym_to_native(pc_rel)?;
            }

            cur_pos = cur_pos.checked_add(op.size_in_bytes())?;
        }

        Ok(cur_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{arch64::ops::ImmI21_2, layout::GlobalLabelAddrMap};
    use googletest::assert_that;
    use googletest::prelude::{anything, contains_substring, eq, err, ok, pat};
    use riscv32::immediate::Immediate as _;

    /// A gas-deduction trampoline, `n_filler_ops` NOPs, then a bounds-check style `B.cond`
    /// back to the trampoline's `PanicIndirection` label, followed by the `ExitPanic` label the
    /// trampoline's `B` needs. The `B.cond` sits `4 + 4 * n_filler_ops` bytes past its target.
    fn block_with_panic_branch(n_filler_ops: usize) -> Vec<UnresOp<Arm64Op>> {
        let mut ops = Arm64Envelope::deduct_gas_and_panic_trampoline(RvGasCost::zero());
        ops.extend(std::iter::repeat_n(
            UnresOp::Native(Arm64NativeOp::Nop),
            n_filler_ops,
        ));
        ops.push(
            Arm64SymbolicOp::Bcc(LocalCodeLabel::PanicIndirection.into(), ConditionCode::Pl).into(),
        );
        ops.push(GlobalCodeLabel::ExitPanic.into());
        ops
    }

    fn resolve(ops: &mut [UnresOp<Arm64Op>]) -> Result<TranspiledAddress, TranspilerError> {
        // `ExitPanic` is the last item, so its address is the byte size of everything before it.
        let exit_panic_addr = TranspiledAddress::new(ops.iter().map(UnresOp::size_in_bytes).sum());
        let mut global_addresses = GlobalLabelAddrMap::new();
        global_addresses.insert(GlobalCodeLabel::ExitPanic.into(), exit_panic_addr);
        let resolver = TranspiledAddressResolver::new(&global_addresses, &[]);
        Arm64Envelope::resolve_in_place(ops, &resolver)
    }

    /// The largest filler that keeps the `B.cond` within its ±1MB reach: the offset lands
    /// exactly on `ImmI21_2::MIN_VAL`.
    const MAX_FILLER_OPS_IN_REACH: usize = (-(ImmI21_2::MIN_VAL as isize) as usize / 4) - 1;

    #[test]
    fn conditional_branch_at_maximum_reach_resolves() {
        let mut ops = block_with_panic_branch(MAX_FILLER_OPS_IN_REACH);

        assert_that!(resolve(&mut ops), ok(anything()));
        let UnresOp::Native(resolved_branch) = &ops[ops.len() - 2] else {
            panic!("the B.cond must be resolved to a native op");
        };
        assert_that!(
            resolved_branch,
            pat!(Arm64NativeOp::Bcc(
                pat!(ImmI21_2(eq(&ImmI21_2::MIN_VAL))),
                anything()
            ))
        );
    }

    #[test]
    fn conditional_branch_beyond_reach_is_rejected_instead_of_truncated() {
        let mut ops = block_with_panic_branch(MAX_FILLER_OPS_IN_REACH + 1);

        let result = resolve(&mut ops);
        assert_that!(
            result,
            err(pat!(TranspilerError::PcRelativeOffsetOutOfRange {
                op: eq(&"B.cond"),
                offset: eq(&(ImmI21_2::MIN_VAL - 4)),
                ..
            }))
        );
        let message = result.map(|_| ()).unwrap_err().to_string();
        assert_that!(message, contains_substring("B.cond"));
        assert_that!(message, contains_substring("does not fit"));
    }
}
