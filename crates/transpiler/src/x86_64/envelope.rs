//! The x86_64 envelope: the code around the transpiled program that saves and restores host
//! and guest state, dispatches to entry points and ecall returns, and provides the exits.
use object::elf;
use riscv32::gas_cost::RvGasCost;
use riscv32::registers::{RiscvRegister, RvPreservedRegister};
use riscv32_elf::data_layout::{NO_ECALL_RETURN_ADDRESS, StoredRegister};
use riscv32_elf::types::RiscvAddress;
use strum::IntoEnumIterator as _;
use user_code_runner::runner::{
    ILLEGAL_INSTRUCTION_EXIT_VALUE, NORMAL_EXIT_VALUE, PANIC_EXIT_VALUE,
};

use crate::TranspilerError;
use crate::layout::{GlobalCodeLabel, GlobalDataLabel, LocalCodeLabel};
use crate::traits::{PcRelativeOffset, TranspiledEnvelope, TranspiledOp, UnresOp};
use crate::transpiled_address::{
    AddressResolveOutcome, TranspiledAddress, TranspiledAddressResolver,
};
use crate::x86_64::ops::{X86ConditionCode, X86Immediate, X86NativeOp, X86Op, X86SymbolicOp};
use crate::x86_64::registers::{
    REG_EAX, REG_EDI, REG_ESI, REG_R11D, REG_R12D, REG_R13D, X86_CALLEE_PRESERVED_REGISTERS,
    X86_REG_GAS, X86_REG_TMP_EAX, X86_REG_TMP_EBX, X86_RISC_A7_REG, X86_RISC_RA_REG,
    X86MappedRegister, X86Reg32Bit,
};
use crate::x86_64::transpile_inst::{N_BYTES_IN_WORD_MINUS_1, x86_jump_via_jump_table_ops};

// The host passes guest a0-a7 in rcx, rdx, r8, r9, r10, rax, rsi and rdi (see `code_wrapper.rs`).
// Inside the program a0-a4 use the same registers, but a5-a7 live in r11-r13, because rax is a
// temporary, rsi holds gas and rdi holds `ra`. Each pair is (host register, guest register); the
// envelope moves a5-a7 between them on entry and exit.
const IN_OUT_ARGS_MAPPING: [(X86Reg32Bit, X86Reg32Bit); 3] = [
    (REG_EAX, REG_R11D),
    (REG_ESI, REG_R12D),
    (REG_EDI, REG_R13D),
];

// Loads the guest registers that live in host registers from the register store.
fn reg_store_to_mapped_registers_ops() -> Vec<UnresOp<X86Op>> {
    RvPreservedRegister::iter()
        .filter_map(|risc_reg| {
            // Registers that live in the register store are used in place.
            if let X86MappedRegister::Register(r) =
                X86MappedRegister::from(RiscvRegister::from(risc_reg))
            {
                Some(
                    X86SymbolicOp::Mov_Reg_Mem(
                        r.into(),
                        StoredRegister::Riscv32Reg(risc_reg).into(),
                    )
                    .into(),
                )
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
}

pub(crate) struct X86Envelope;

impl TranspiledEnvelope<X86Op> for X86Envelope {
    fn branch_to_illegal_code() -> Vec<UnresOp<X86Op>> {
        vec![X86SymbolicOp::Jmp(GlobalCodeLabel::ExitIllegalOpcode.into()).into()]
    }

    fn deduct_gas_and_panic_trampoline(cost: RvGasCost) -> Vec<UnresOp<X86Op>> {
        let mut code = vec![
            X86NativeOp::Sub_Gas_Imm(X86Immediate::from(cost.value() as i32)).into(),
            // Panic if out of gas (underflow)
            X86SymbolicOp::Jcc(X86ConditionCode::NotSign, LocalCodeLabel::SmallSkip.into()).into(),
        ];
        code.push(X86SymbolicOp::Jmp(GlobalCodeLabel::ExitPanic.into()).into());
        code.push(UnresOp::Label(LocalCodeLabel::SmallSkip.into()));
        code
    }

    fn exit_user_code() -> Vec<UnresOp<X86Op>> {
        // `ExitIllegalOpcode` comes first, so falling off the end of the program lands on it.
        // Each exit sets a7 to its exit code and saves the guest state. None of them is an
        // ecall, so each sets the return address (eax) to `NO_ECALL_RETURN_ADDRESS`.
        let exit = |label: GlobalCodeLabel, exit_value: i32| -> [UnresOp<X86Op>; 3] {
            [
                UnresOp::Label(label.into()),
                X86NativeOp::Mov_Reg_RoI(X86_RISC_A7_REG, X86Immediate::from(exit_value).into())
                    .into(),
                X86NativeOp::Mov_Reg_RoI(
                    X86_REG_TMP_EAX.d(),
                    X86Immediate::from(NO_ECALL_RETURN_ADDRESS as i32).into(),
                )
                .into(),
            ]
        };
        let to_save = || X86SymbolicOp::Jmp(GlobalCodeLabel::SaveLocalState.into()).into();
        let mut code = exit(
            GlobalCodeLabel::ExitIllegalOpcode,
            ILLEGAL_INSTRUCTION_EXIT_VALUE,
        )
        .to_vec();
        code.push(to_save());
        code.extend(exit(GlobalCodeLabel::ExitPanic, PANIC_EXIT_VALUE));
        code.push(to_save());
        code.extend(exit(GlobalCodeLabel::ExitNormal, NORMAL_EXIT_VALUE));
        code
    }

    fn save_local_state() -> Vec<UnresOp<X86Op>> {
        let mut code = vec![UnresOp::Label(GlobalCodeLabel::SaveLocalState.into())];
        for risc_reg in RvPreservedRegister::iter() {
            // Registers that live in the register store are already up to date.
            if let X86MappedRegister::Register(r) =
                X86MappedRegister::from(RiscvRegister::from(risc_reg))
            {
                code.push(
                    X86SymbolicOp::Mov_Mem_Reg(
                        StoredRegister::Riscv32Reg(risc_reg).into(),
                        r.into(),
                    )
                    .into(),
                );
            }
        }
        // Store eax: after an ecall it holds the RISC-V address to resume at.
        code.push(
            X86SymbolicOp::Mov_Mem_Reg(
                StoredRegister::ReturnAddress.into(),
                X86_REG_TMP_EAX.d().into(),
            )
            .into(),
        );
        // Store the 64-bit gas register.
        code.push(
            X86SymbolicOp::Mov_Mem_Reg(StoredRegister::Gas.into(), X86_REG_GAS.into()).into(),
        );
        code
    }

    fn load_global_state_and_ret() -> Vec<UnresOp<X86Op>> {
        let mut code = vec![];
        // Move a5-a7 back to the registers the host expects.
        code.extend(
            IN_OUT_ARGS_MAPPING
                .iter()
                .map(|(external_reg, internal_reg)| {
                    X86NativeOp::Mov_Reg_RoI(*external_reg, (*internal_reg).into()).into()
                }),
        );
        // Restore the host's callee-saved registers.
        code.extend(
            X86_CALLEE_PRESERVED_REGISTERS
                .iter()
                .rev()
                .map(|real_reg| X86NativeOp::Pop(*real_reg).into()),
        );
        code.push(X86NativeOp::Ret().into());
        code
    }

    fn ecall_exit() -> Vec<UnresOp<X86Op>> {
        // a7 + 2 < 3, unsigned, exactly when a7 is -2, -1 or 0: an exit code, not an ecall.
        // eax holds the return address, so use ebx.
        vec![
            UnresOp::Label(GlobalCodeLabel::EcallExit.into()),
            X86NativeOp::Mov_Reg_RoI(X86_REG_TMP_EBX.d(), X86_RISC_A7_REG.into()).into(),
            X86NativeOp::Add_Reg_RoI(X86_REG_TMP_EBX.d(), X86Immediate::from(2).into()).into(),
            X86NativeOp::Cmp_Reg_RoI(X86_REG_TMP_EBX.d(), X86Immediate::from(3).into()).into(),
            X86SymbolicOp::Jcc(
                X86ConditionCode::Below,
                GlobalCodeLabel::ExitIllegalOpcode.into(),
            )
            .into(),
            // Save the ecall number, which a resume restores to a7.
            X86SymbolicOp::Mov_Mem_Reg(StoredRegister::EcallNumber.into(), X86_RISC_A7_REG.into())
                .into(),
            X86SymbolicOp::Jmp(GlobalCodeLabel::SaveLocalState.into()).into(),
        ]
    }

    fn save_global_state() -> Vec<UnresOp<X86Op>> {
        let mut code = vec![UnresOp::Label(GlobalCodeLabel::StartTranspiledCode.into())];
        // Save the host's callee-saved registers.
        code.extend(
            X86_CALLEE_PRESERVED_REGISTERS
                .iter()
                .map(|real_reg| X86NativeOp::Push(*real_reg).into()),
        );
        // Move a5-a7 from the host's argument registers to the guest's.
        code.extend(
            IN_OUT_ARGS_MAPPING
                .iter()
                .rev()
                .map(|(external_reg, internal_reg)| {
                    X86NativeOp::Mov_Reg_RoI(*internal_reg, (*external_reg).into()).into()
                }),
        );
        code
    }

    fn load_local_state() -> Vec<UnresOp<X86Op>> {
        let mut code = reg_store_to_mapped_registers_ops();
        // Load eax with the ecall return address; it is used only when a7 is 0.
        code.push(
            X86SymbolicOp::Mov_Reg_Mem(
                X86_REG_TMP_EAX.d().into(),
                StoredRegister::ReturnAddress.into(),
            )
            .into(),
        );
        // Load the 64-bit gas register.
        code.push(
            X86SymbolicOp::Mov_Reg_Mem(X86_REG_GAS.into(), StoredRegister::Gas.into()).into(),
        );
        code
    }

    fn enter_user_code(
        end_text_section: RiscvAddress,
        n_hooks: u32,
        trusted: bool,
    ) -> Vec<UnresOp<X86Op>> {
        // a7 == 0: return from an ecall to the RISC-V address in eax.
        let mut code = vec![
            X86NativeOp::Test_Reg_RoI(X86_RISC_A7_REG, X86_RISC_A7_REG.into()).into(),
            X86SymbolicOp::Jcc(
                X86ConditionCode::NotEqual,
                GlobalCodeLabel::NotEcallReturn.into(),
            )
            .into(),
            // The return address is read from the host-writable register store, so check that
            // it is 4-aligned and at most `end_text_section` (the exit sentinel) before using it
            // as a jump-table index. The unsigned compare also rejects negative values.
            X86NativeOp::Test_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(N_BYTES_IN_WORD_MINUS_1).into(),
            )
            .into(),
            X86SymbolicOp::Jcc(
                X86ConditionCode::NotEqual,
                GlobalCodeLabel::ExitPanic.into(),
            )
            .into(),
            X86NativeOp::Cmp_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(end_text_section.as_i32()).into(),
            )
            .into(),
            X86SymbolicOp::Jcc(X86ConditionCode::Above, GlobalCodeLabel::ExitPanic.into()).into(),
            // Resuming at the end of the code is like falling off the last instruction.
            X86SymbolicOp::Jcc(
                X86ConditionCode::Equal,
                GlobalCodeLabel::ExitIllegalOpcode.into(),
            )
            .into(),
            // Give the guest back its a7, the ecall number.
            X86SymbolicOp::Mov_Reg_Mem(X86_RISC_A7_REG.into(), StoredRegister::EcallNumber.into())
                .into(),
        ];
        // Jump through the jump table to the RISC-V address in eax.
        code.extend(x86_jump_via_jump_table_ops(trusted));
        // a7 != 0: run init function or entry point number a7 (1-based).
        code.extend([
            UnresOp::Label(GlobalCodeLabel::NotEcallReturn.into()),
            // Set ra to the end of `.text`, so returning from the entry point takes the exit
            // sentinel.
            X86NativeOp::Mov_Reg_RoI(
                X86_RISC_RA_REG,
                X86Immediate::from(end_text_section.as_i32()).into(),
            )
            .into(),
            // Make the selector 0-based, in eax so the guest sees a7 = selector. Each hook is a
            // 5-byte `jmp`, so the hook address is `EntryPointsHooksBegin + 5 * index`.
            X86NativeOp::Mov_Reg_RoI(X86_REG_TMP_EAX.d(), X86_RISC_A7_REG.into()).into(),
            X86NativeOp::Sub_Reg_RoI(X86_REG_TMP_EAX.d(), X86Immediate::from(1).into()).into(),
            // The host supplies the selector, so reject a 0-based index of `n_hooks` or more; the
            // unsigned compare also rejects a negative selector.
            X86NativeOp::Cmp_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(n_hooks as i32).into(),
            )
            .into(),
            X86SymbolicOp::Jcc(
                X86ConditionCode::AboveEqual,
                GlobalCodeLabel::ExitPanic.into(),
            )
            .into(),
            X86NativeOp::Mul_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(X86Op::branch_size_in_bytes() as i32).into(),
            )
            .into(),
            X86SymbolicOp::Lea_Tmp_Mem(
                X86_REG_TMP_EBX,
                GlobalDataLabel::EntryPointsHooksBegin.into(),
            )
            .into(),
            X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EBX, X86_REG_TMP_EAX.q()).into(),
            X86NativeOp::Jmp_Tmp(X86_REG_TMP_EBX).into(),
        ]);
        code
    }

    fn elf_arch_indicator() -> u16 {
        elf::EM_X86_64
    }

    fn forward_pass_convert_branches(
        ops: &mut [UnresOp<X86Op>],
        address_resolver: &TranspiledAddressResolver<'_>,
    ) -> Result<TranspiledAddress, TranspilerError> {
        let mut cur_pos = TranspiledAddress::new(0);

        for op in ops.iter_mut() {
            // x86 PC-relative offsets count from the end of the instruction, so advance first.
            let op_size = op.size_in_bytes();
            cur_pos = cur_pos.checked_add(op_size)?;

            if let Some(target) = match op {
                // `SmallSkip` targets are left for the backward pass. x86 has no
                // `PanicIndirection`: its branches reach `ExitPanic` directly.
                UnresOp::Symbolic(X86SymbolicOp::Jmp(target))
                | UnresOp::Symbolic(X86SymbolicOp::Jcc(_, target)) => {
                    match address_resolver.resolve(
                        target,
                        LocalCodeLabel::PanicIndirection,
                        &None,
                    )? {
                        AddressResolveOutcome::Resolved(target_addr) => Some(target_addr),
                        AddressResolveOutcome::NotApplicable => None,
                    }
                }
                // `lea` and `mov` to or from a data label become RIP-relative.
                UnresOp::Symbolic(X86SymbolicOp::Lea_Tmp_Mem(_, mem_loc))
                | UnresOp::Symbolic(X86SymbolicOp::Mov_Reg_Mem(_, mem_loc))
                | UnresOp::Symbolic(X86SymbolicOp::Mov_Mem_Reg(mem_loc, _))
                | UnresOp::Symbolic(X86SymbolicOp::Mov_Mem_Imm(mem_loc, _)) => {
                    let target_addr = address_resolver.resolve_label(&(mem_loc.label).into())?;
                    Some(target_addr.wrapping_add(mem_loc.offset))
                }
                _ => None,
            } {
                op.sym_to_native(PcRelativeOffset::from_to(cur_pos, target))?;
            }
        }
        Ok(cur_pos)
    }
}
