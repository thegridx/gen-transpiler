//! Traits each target architecture implements: instruction encoders, the per-instruction
//! transpiler, the envelope around the program, and the host-side call into the output.
use riscv32::gas_cost::RvGasCost;
use riscv32::ops::{
    RvAtomic, RvAuipc, RvBranch, RvJal, RvJalr, RvLoad, RvLui, RvMulDivRem, RvOpImm, RvOpReg,
    RvStore,
};
use riscv32_elf::types::{RiscvAddress, VirtualDataRange};
use user_code_runner::runner::RegistersInOut;

use crate::TranspilerError;
use crate::execute::{LoaderError, SymbolHandle};
use crate::layout::{
    GlobalCodeLabel, GlobalDataLabel, LocalCodeLabel, SymbolicBranchTarget, SymbolicLabel,
};
use crate::transpiled_address::{TranspiledAddress, TranspiledAddressResolver};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct PcRelativeOffset(i32);
impl PcRelativeOffset {
    pub(crate) fn new(offset: i32) -> Self {
        Self(offset)
    }
    pub(crate) fn as_i32(&self) -> i32 {
        self.0
    }
    pub(crate) fn from_to(from: TranspiledAddress, to: TranspiledAddress) -> Self {
        Self(to.wrapping_sub(from.as_u32()).as_i32())
    }
}

/// An operation whose encoding depends on an address that is not known yet.
///
/// Symbolic operations are branches and label-relative memory accesses. The resolution passes
/// replace them with [`NativeOp`] values once all addresses are known.
pub(crate) trait SymbolicOp: Clone {
    /// Returns the size in bytes of the operation once resolved.
    ///
    /// The size must not depend on the resolved offset, because the resolution passes compute
    /// addresses from it before any offset is known.
    fn size_in_bytes(&self) -> u32;
    /// Whether this is a branch to [`LocalCodeLabel::SmallSkip`], which the backward pass
    /// resolves.
    fn is_small_skip_branch(&self) -> bool;
}

/// A fully resolved native instruction: every operand, including PC-relative offsets, is known.
pub(crate) trait NativeOp: Clone {
    /// Returns the encoded size of the operation in bytes.
    fn size_in_bytes(&self) -> u32;
    /// Appends the little-endian machine code of the operation to `out`.
    fn encode(&self, out: &mut Vec<u8>);
    /// Returns the operation rendered as a single line of GNU assembly.
    #[cfg(test)]
    fn as_assembly(&self) -> String;
}

/// The native and symbolic operation types of one target architecture.
pub(crate) trait TranspiledOp: Clone {
    /// Fully-resolved native instruction type for this architecture.
    type Native: NativeOp;
    /// Unresolved instruction type whose final encoding depends on layout.
    type Symbolic: SymbolicOp;

    /// Whether a PC-relative offset counts from the end of its instruction (x86_64) rather
    /// than its start (aarch64).
    const PC_RELATIVE_FROM_END: bool;

    /// Builds a symbolic branch to `target`.
    fn new_branch(target: SymbolicBranchTarget) -> Self::Symbolic;

    /// Returns the encoded size in bytes of a native branch instruction on this architecture.
    fn branch_size_in_bytes() -> u32;

    /// Resolves a symbolic op into a native op with the given PC-relative offset.
    ///
    /// Fails with [`TranspilerError::PcRelativeOffsetOutOfRange`] when the offset does not fit
    /// the instruction's immediate field, so an out-of-reach branch is never truncated.
    fn sym_to_native(
        sym_op: Self::Symbolic,
        pc_relative: PcRelativeOffset,
    ) -> Result<Self::Native, TranspilerError>;
}

/// An item in the instruction stream: a label (zero bytes), a native op or a symbolic op.
#[derive(Clone)]
pub(crate) enum UnresOp<TO: TranspiledOp> {
    Label(SymbolicLabel),
    Native(TO::Native),
    Symbolic(TO::Symbolic),
}
impl<TO: TranspiledOp> From<GlobalDataLabel> for UnresOp<TO> {
    fn from(label: GlobalDataLabel) -> Self {
        UnresOp::Label(SymbolicLabel::Global(label.into()))
    }
}
impl<TO: TranspiledOp> From<GlobalCodeLabel> for UnresOp<TO> {
    fn from(label: GlobalCodeLabel) -> Self {
        UnresOp::Label(SymbolicLabel::Global(label.into()))
    }
}
impl<TO: TranspiledOp> UnresOp<TO> {
    pub(crate) fn new_branch(target: SymbolicBranchTarget) -> Self {
        UnresOp::Symbolic(TO::new_branch(target))
    }
    pub(crate) fn sym_to_native(
        &mut self,
        pc_relative: PcRelativeOffset,
    ) -> Result<(), TranspilerError> {
        match self {
            UnresOp::Label(_) | UnresOp::Native(_) => {}
            UnresOp::Symbolic(sym_op) => {
                *self = UnresOp::Native(TO::sym_to_native(sym_op.clone(), pc_relative)?);
            }
        }
        Ok(())
    }
    pub(crate) fn size_in_bytes(&self) -> u32 {
        match self {
            UnresOp::Label(_) => 0,
            UnresOp::Native(op) => op.size_in_bytes(),
            UnresOp::Symbolic(sym_op) => sym_op.size_in_bytes(),
        }
    }
}

/// Translates one decoded RISC-V instruction into native operations for the target `TO`.
///
/// The methods take no `self`: the same instruction always produces the same code.
pub(crate) trait InstructionTranspiler<TO: TranspiledOp> {
    /// Transpile arithmetic/logical operations with an immediate value.
    /// The caller guarantees `inst.rd != RiscvRegister::Zero`.
    fn transpile_op_imm(inst: &RvOpImm) -> Vec<UnresOp<TO>>;
    /// Transpile arithmetic/logical operations with register values.
    /// The caller guarantees `inst.rd != RiscvRegister::Zero`.
    fn transpile_op_reg(inst: &RvOpReg) -> Vec<UnresOp<TO>>;
    /// Transpile `LUI` — load upper immediate.
    /// The caller guarantees `inst.rd != RiscvRegister::Zero`.
    fn transpile_lui(inst: &RvLui) -> Vec<UnresOp<TO>>;
    /// Transpile `AUIPC` — add upper immediate to the program counter.
    /// `target` is the address computed by adding the upper immediate to the program counter.
    fn transpile_auipc(inst: &RvAuipc, target: RiscvAddress) -> Vec<UnresOp<TO>>;
    /// Transpile multiplication/division/remainder operations.
    /// The caller guarantees `inst.rd != RiscvRegister::Zero`.
    /// `next_place` is the address of the next instruction; the code may branch to it to skip
    /// the division-by-zero path.
    fn transpile_mul_div_rem(inst: &RvMulDivRem, next_place: RiscvAddress) -> Vec<UnresOp<TO>>;
    /// Transpile `JAL` to a valid `target`. `next_place` is the return address written to `rd`.
    fn transpile_jal(
        inst: &RvJal,
        target: RiscvAddress,
        next_place: RiscvAddress,
    ) -> Vec<UnresOp<TO>>;
    /// Transpile `JALR`, which jumps through the jump table.
    /// Bit 0 of the target is cleared; a target with bit 1 set panics (RISC-V
    /// instruction-address-misaligned, IALIGN=32) in both modes. `risc_code_end` is the end of
    /// the RISC-V code; a target past it panics. In trusted mode this range check is omitted.
    fn transpile_jalr(
        inst: &RvJalr,
        next_place: RiscvAddress,
        risc_code_end: RiscvAddress,
        trusted: bool,
    ) -> Vec<UnresOp<TO>>;
    /// Transpile a conditional branch (`BEQ`, `BNE`, `BLT`, `BGE`, `BLTU`, `BGEU`).
    fn transpile_branch(inst: &RvBranch, target: SymbolicBranchTarget) -> Vec<UnresOp<TO>>;
    /// Transpile a load instruction (`LB`, `LH`, `LW`, `LBU`, `LHU`).
    /// `readable_range` holds the guest addresses a load of this size may start at: both `start`
    /// and `end` are valid, so the last byte read is at most `end` + size - 1.
    /// In trusted mode the bounds check is omitted.
    fn transpile_load(
        inst: &RvLoad,
        readable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<TO>>;
    /// Transpile a store instruction (`SB`, `SH`, `SW`).
    /// `writable_range` holds the guest addresses a store of this size may start at: both
    /// `start` and `end` are valid, so the last byte written is at most `end` + size - 1.
    /// In trusted mode the bounds check is omitted.
    fn transpile_store(
        inst: &RvStore,
        writable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<TO>>;
    /// Transpile an `ECALL` instruction.
    /// `next_place` is the address of the next instruction, where execution resumes when the
    /// host returns from the ecall.
    fn transpile_system_call(next_place: RiscvAddress) -> Vec<UnresOp<TO>>;
    /// Transpile an RV32A atomic memory operation.
    /// `readable_range` and `writable_range` hold the addresses a word access may start at, with
    /// both `start` and `end` valid; atomics are always word-sized.
    /// In trusted mode the bounds and alignment checks are omitted.
    fn transpile_atomic(
        inst: &RvAtomic,
        readable_range: VirtualDataRange,
        writable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<TO>>;
}

/// Generates the envelope: the code around the transpiled program that enters it from the
/// host, dispatches calls, and saves and restores state on exit.
pub(crate) trait TranspiledEnvelope<TO: TranspiledOp> {
    /// Emits a jump to the illegal-instruction exit, used in place of an instruction that
    /// cannot be transpiled: a single `b` on aarch64, a single `jmp` on x86_64.
    fn branch_to_illegal_code() -> Vec<UnresOp<TO>>;
    /// Emits code that subtracts `cost` from the gas register and jumps to
    /// [`GlobalCodeLabel::ExitPanic`] if the result is negative. On aarch64, later branches
    /// to [`LocalCodeLabel::PanicIndirection`] reach the panic exit through the most recent of
    /// these jumps.
    ///
    /// [`LocalCodeLabel::PanicIndirection`]: crate::layout::LocalCodeLabel::PanicIndirection
    fn deduct_gas_and_panic_trampoline(cost: RvGasCost) -> Vec<UnresOp<TO>>;
    /// Emits the three exits: illegal instruction (first, so that falling off the end of the
    /// program reaches it), panic and normal. Each sets a7 to its exit code and the return
    /// address to `NO_ECALL_RETURN_ADDRESS`, and all of them continue into
    /// [`TranspiledEnvelope::save_local_state`].
    fn exit_user_code() -> Vec<UnresOp<TO>>;
    /// Emits code that saves the guest state (the RISC-V preserved registers, the return
    /// address and gas) into `.register_store`, where the host can read or change it.
    fn save_local_state() -> Vec<UnresOp<TO>>;
    /// Emits code that restores the host's callee-saved registers from the stack and returns
    /// to the host.
    fn load_global_state_and_ret() -> Vec<UnresOp<TO>>;
    /// Emits `EcallExit`, where every `ECALL` goes with its return address set. An ecall whose
    /// number in a7 is an exit code (0, -1 or -2) takes the illegal-instruction exit, so a
    /// guest cannot pose as a normal, panic or illegal-instruction exit. Any other ecall
    /// continues into [`TranspiledEnvelope::save_local_state`].
    fn ecall_exit() -> Vec<UnresOp<TO>>;
    /// Emits `_start_transpiled_code`, the entry point, which saves the host's callee-saved
    /// registers on the stack.
    fn save_global_state() -> Vec<UnresOp<TO>>;
    /// Emits code that loads the guest state (the RISC-V preserved registers, the return
    /// address and gas) from `.register_store`. The entry stub from
    /// [`TranspiledEnvelope::enter_user_code`] follows it.
    fn load_local_state() -> Vec<UnresOp<TO>>;
    /// Emits the entry stub. When a7 is 0 it returns from an ecall by jumping through the jump
    /// table to the saved return address; a return address of exactly the end of the code takes
    /// the illegal-instruction exit, like falling off the end. Otherwise it calls init function
    /// or entry point a7 through the hook table. `end_text_section` is the end of the RISC-V code: the largest
    /// valid return address, and the return address a call starts with (the exit sentinel).
    /// `n_hooks` is the number of init functions plus entry points, which bounds the 1-based
    /// selector in a7. `trusted` selects the jump-table entry size (4 bytes in trusted mode,
    /// 8 otherwise).
    ///
    /// The selector and the saved return address come from host-writable state, so the stub
    /// checks both, in either mode, and takes the panic exit when one is out of range.
    fn enter_user_code(
        end_text_section: RiscvAddress,
        n_hooks: u32,
        trusted: bool,
    ) -> Vec<UnresOp<TO>>;
    /// Returns the ELF header architecture indicator for this target (e.g. `elf::EM_X86_64`
    /// or `elf::EM_AARCH64`).
    fn elf_arch_indicator() -> u16;

    /// First resolution pass: resolves every symbolic op except branches to
    /// [`LocalCodeLabel::SmallSkip`], and returns the end address of the code.
    ///
    /// [`LocalCodeLabel::SmallSkip`]: crate::layout::LocalCodeLabel::SmallSkip
    fn forward_pass_convert_branches(
        ops: &mut [UnresOp<TO>],
        address_resolver: &TranspiledAddressResolver<'_>,
    ) -> Result<TranspiledAddress, TranspilerError>;
    /// Resolves all symbolic ops to native ops in place, by running both passes. Afterwards
    /// `ops` holds only [`UnresOp::Native`] and [`UnresOp::Label`] entries. Returns the end
    /// address of the code, which is the exact `.text` length in bytes.
    fn resolve_in_place(
        ops: &mut [UnresOp<TO>],
        address_resolver: &TranspiledAddressResolver<'_>,
    ) -> Result<TranspiledAddress, TranspilerError> {
        let transpiled_code_end = Self::forward_pass_convert_branches(ops, address_resolver)?;
        backward_pass_resolve_small_skips(ops, transpiled_code_end)?;
        Ok(transpiled_code_end)
    }
}

/// Second resolution pass: walks the code backwards and resolves each branch to
/// [`LocalCodeLabel::SmallSkip`] to the next such label after it.
fn backward_pass_resolve_small_skips<TO: TranspiledOp>(
    ops: &mut [UnresOp<TO>],
    transpiled_code_end: TranspiledAddress,
) -> Result<(), TranspilerError> {
    let mut next_small_skip_addr: Option<TranspiledAddress> = None;
    let mut op_end = transpiled_code_end;
    for op in ops.iter_mut().rev() {
        let op_start = op_end.checked_sub(op.size_in_bytes())?;
        match op {
            UnresOp::Label(SymbolicLabel::Local(LocalCodeLabel::SmallSkip)) => {
                next_small_skip_addr = Some(op_start);
            }
            UnresOp::Symbolic(sym_op) if sym_op.is_small_skip_branch() => {
                let target = next_small_skip_addr.ok_or_else(|| {
                    TranspilerError::local_label_not_found(LocalCodeLabel::SmallSkip)
                })?;
                let pc = if TO::PC_RELATIVE_FROM_END {
                    op_end
                } else {
                    op_start
                };
                op.sym_to_native(PcRelativeOffset::from_to(pc, target))?;
            }
            UnresOp::Label(_) | UnresOp::Native(_) | UnresOp::Symbolic(_) => {}
        }
        op_end = op_start;
    }
    Ok(())
}

/// Encodes a resolved op stream into a new buffer. `code_len_in_bytes` (the code end returned
/// by [`TranspiledEnvelope::resolve_in_place`]) only sets the buffer's capacity.
///
/// `ops` must already be resolved (no [`UnresOp::Symbolic`]); a leftover symbolic op is a bug
/// and returns [`TranspilerError::UnexpectedSymbolicOperation`]. Labels emit no bytes.
pub(crate) fn resolved_ops_to_bytecode<TO: TranspiledOp>(
    ops: &[UnresOp<TO>],
    code_len_in_bytes: u32,
) -> Result<Vec<u8>, TranspilerError> {
    let mut out = Vec::with_capacity(code_len_in_bytes as usize);
    for op in ops {
        match op {
            UnresOp::Native(native_op) => native_op.encode(&mut out),
            UnresOp::Label(_) => {}
            UnresOp::Symbolic(_) => return Err(TranspilerError::UnexpectedSymbolicOperation),
        }
    }
    Ok(out)
}

/// Calls loaded transpiled code from the host.
///
/// The call enters `_start_transpiled_code` with guest a0 to a7 in the registers the target's
/// envelope expects. On a host the implementation cannot run on, it returns
/// [`LoaderError::CompiledWithWrongArchitecture`].
pub(crate) trait TranspiledCodeWrapper {
    /// Calls `_start_transpiled_code` with a0 to a7 from `in_out`, and writes the registers
    /// the code returns back into `in_out`.
    fn transpiled_code_wrapper(
        start_transpiled_code_symbol: SymbolHandle,
        in_out: &mut RegistersInOut,
    ) -> Result<(), LoaderError>;
}

/// Bundles the traits of one target architecture. There is one implementation per target
/// (`Arm64Transpiler` and `X86Transpiler`), and it is the type parameter of the pipeline.
pub(crate) trait TranspilerImplementation {
    /// Native and symbolic operation types.
    type OpType: TranspiledOp;
    /// Per-instruction transpiler.
    type InstructionTranspiler: InstructionTranspiler<Self::OpType>;
    /// Envelope generator.
    type EnvelopeType: TranspiledEnvelope<Self::OpType>;
    /// Host-side call into the transpiled code.
    type CodeWrapperType: TranspiledCodeWrapper;
}
