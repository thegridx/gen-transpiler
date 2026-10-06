//! Lowers each RISC-V instruction to x86_64 ops. x86_64 has only 16 registers, two-operand
//! arithmetic and fixed registers for shifts (cl), mul and div (edx:eax), so most guest registers
//! live in `.register_store` and are used in place through RIP-relative operands, and each
//! lowering works on a host register: the guest register's own, or a temporary (eax or ebx),
//! which every lowering may clobber. Fixed registers that hold guest values are saved on the
//! host stack around the instructions that need them.
use crate::layout::{
    GlobalCodeLabel, GlobalDataLabel, JUMP_TABLE_OFFSET_POSITION, LocalCodeLabel,
    SymbolicBranchTarget,
};
use crate::traits::{InstructionTranspiler, UnresOp};
use crate::x86_64::ops::{
    X86ConditionCode, X86Imm8Bit, X86Immediate, X86NativeOp, X86Op, X86ShiftAmount, X86SymbolicOp,
};
use crate::x86_64::registers::{
    REG_EAX, REG_ECX, REG_EDX, REG_RCX, REG_RDX, X86_REG_TMP_EAX, X86_REG_TMP_EBX,
    X86MappedRegister, X86Reg32Bit, X86TempReg, is_x86_reg,
};
use riscv32::N_BYTES_IN_WORD;
use riscv32::common::SignType;
use riscv32::immediate::{ImmU5_0, Immediate as _};
use riscv32::ops::{
    ArithmeticOp, AtomicOp, BasicOp, BranchCondition, DivRemOp, LoadSize, MulDivRemOp, MulHighOp,
    RegArithmeticOp, RvAtomic, RvAuipc, RvBranch, RvImmI12, RvJal, RvJalr, RvLoad, RvLui,
    RvMulDivRem, RvOpImm, RvOpReg, RvStore, ShiftOp, StoreSize,
};
use riscv32::registers::RiscvRegister;
use riscv32_elf::data_layout::StoredRegister;
use riscv32_elf::types::{RiscvAddress, VirtualDataRange};
use std::vec;

pub(crate) const N_BYTES_IN_WORD_MINUS_1: i32 = (N_BYTES_IN_WORD as i32) - 1; // 3

// Collects the ops of one lowering, with helpers that move values between RISC-V registers
// (in a host register, in the register store, or x0) and host registers, so the lowerings need
// not care where each guest register lives. A helper that takes a temporary may clobber it.
#[derive(Default)]
struct X86TranspiledCode {
    code: Vec<UnresOp<X86Op>>,
}

impl X86TranspiledCode {
    fn push(&mut self, op: X86NativeOp) {
        self.code.push(op.into());
    }
    fn push_sym(&mut self, op: X86SymbolicOp) {
        self.code.push(op.into());
    }
    fn extend(&mut self, ops: impl IntoIterator<Item = UnresOp<X86Op>>) {
        self.code.extend(ops);
    }
    fn into_vec(self) -> Vec<UnresOp<X86Op>> {
        self.code
    }

    // dst = imm. Writes to x0 are dropped.
    fn mov_imm_to_risc_reg(&mut self, imm: i32, dst: RiscvRegister) {
        match X86MappedRegister::from(dst) {
            X86MappedRegister::Register(reg) => {
                self.push(if imm == 0 {
                    // Special case for zeroing a register
                    X86NativeOp::Xor_Self(reg.into())
                } else {
                    X86NativeOp::Mov_Reg_RoI(reg, X86Immediate::from(imm).into())
                });
            }
            X86MappedRegister::MemoryMapped(reg_idx) => {
                self.push_sym(X86SymbolicOp::Mov_Mem_Imm(
                    StoredRegister::Riscv32Reg(reg_idx).into(),
                    X86Immediate::from(imm),
                ));
            }
            X86MappedRegister::Zero => {} // Do nothing
        }
    }

    // Returns the host register of `risc_reg`, or `tmp_reg` if it has none. Emits nothing, so the
    // returned register does not hold the guest value; use it as a destination.
    fn get_risc_reg_as_x86_reg(&self, risc_reg: RiscvRegister, tmp_reg: X86TempReg) -> X86Reg32Bit {
        match X86MappedRegister::from(risc_reg) {
            X86MappedRegister::Register(r) => r,
            X86MappedRegister::MemoryMapped(_) | X86MappedRegister::Zero => tmp_reg.d(),
        }
    }

    // dst_x86_reg = src_risc_reg. Emits nothing if they are already the same register.
    fn mov_risc_reg_to_x86_reg(&mut self, src_risc_reg: RiscvRegister, dst_x86_reg: X86Reg32Bit) {
        match X86MappedRegister::from(src_risc_reg) {
            X86MappedRegister::Register(src_x86_reg) => {
                if src_x86_reg != dst_x86_reg {
                    self.push(X86NativeOp::Mov_Reg_RoI(dst_x86_reg, src_x86_reg.into()));
                }
            }
            X86MappedRegister::MemoryMapped(reg_idx) => {
                self.push_sym(X86SymbolicOp::Mov_Reg_Mem(
                    dst_x86_reg.into(),
                    StoredRegister::Riscv32Reg(reg_idx).into(),
                ));
            }
            X86MappedRegister::Zero => {
                self.push(X86NativeOp::Xor_Self(dst_x86_reg.into()));
            }
        }
    }

    // dst_risc_reg = src_x86_reg, typically to write a result back. Emits nothing if they are
    // already the same register; writes to x0 are dropped.
    fn mov_x86_reg_to_risc_reg(&mut self, src_x86_reg: X86Reg32Bit, dst_risc_reg: RiscvRegister) {
        match X86MappedRegister::from(dst_risc_reg) {
            X86MappedRegister::Register(dst_x86_reg) => {
                if src_x86_reg != dst_x86_reg {
                    self.push(X86NativeOp::Mov_Reg_RoI(dst_x86_reg, src_x86_reg.into()));
                }
            }
            X86MappedRegister::MemoryMapped(reg_idx) => {
                self.push_sym(X86SymbolicOp::Mov_Mem_Reg(
                    StoredRegister::Riscv32Reg(reg_idx).into(),
                    src_x86_reg.into(),
                ));
            }
            X86MappedRegister::Zero => {} // Do nothing
        }
    }

    // Loads `src` into the host register for `dst` (its own register, or `tmp_reg`) and returns
    // that register. The caller then operates on it and writes it back to `dst`.
    fn mov_risc_src_to_x86_dst(
        &mut self,
        src: RiscvRegister,
        dst: RiscvRegister,
        tmp_reg: X86TempReg,
    ) -> X86Reg32Bit {
        let dst_x86_reg = self.get_risc_reg_as_x86_reg(dst, tmp_reg);
        self.mov_risc_reg_to_x86_reg(src, dst_x86_reg);
        dst_x86_reg
    }

    // Returns a host register that holds the value of `risc_reg`: its own register, or `tmp_reg`
    // loaded from the register store (or zeroed for x0).
    fn load_risc_reg_as_x86_reg(
        &mut self,
        risc_reg: RiscvRegister,
        tmp_reg: X86TempReg,
    ) -> X86Reg32Bit {
        self.mov_risc_src_to_x86_dst(risc_reg, risc_reg, tmp_reg)
    }

    // dst_risc_reg = src_risc_reg, through `tmp_reg` if needed.
    fn mov_risc_src_to_dst(
        &mut self,
        src_risc_reg: RiscvRegister,
        dst_risc_reg: RiscvRegister,
        tmp_reg: X86TempReg,
    ) {
        if src_risc_reg != dst_risc_reg {
            let src_x86_reg = self.load_risc_reg_as_x86_reg(src_risc_reg, tmp_reg);
            self.mov_x86_reg_to_risc_reg(src_x86_reg, dst_risc_reg);
        }
    }

    // dst_risc_reg = -src_risc_reg, computed in `tmp_reg`.
    fn mov_neg_risc_src_to_dst(
        &mut self,
        src_risc_reg: RiscvRegister,
        dst_risc_reg: RiscvRegister,
        tmp_reg: X86TempReg,
    ) {
        self.mov_risc_reg_to_x86_reg(src_risc_reg, tmp_reg.d());
        self.push(X86NativeOp::Neg_Reg(tmp_reg.d()));
        self.mov_x86_reg_to_risc_reg(tmp_reg.d(), dst_risc_reg);
    }

    // Prepares a two-operand op `dst = src1 op src2`: returns `(src2_reg, dst_reg)`, where
    // `dst_reg` holds src1 and is the host register for dst (or ebx), and `src2_reg` holds src2.
    // May clobber both temporaries.
    fn load_risc_src1_src2_dst_as_x86(
        &mut self,
        src1: RiscvRegister,
        src2: RiscvRegister,
        dst: RiscvRegister,
    ) -> (X86Reg32Bit, X86Reg32Bit) {
        // Writing src1 into dst would overwrite src2, so copy src2 to eax first.
        if src2 == dst {
            self.mov_risc_reg_to_x86_reg(src2, X86_REG_TMP_EAX.d());
            let dst_x86_reg = self.mov_risc_src_to_x86_dst(src1, dst, X86_REG_TMP_EBX);
            (X86_REG_TMP_EAX.d(), dst_x86_reg)
        // Writing src1 into dst leaves src2 intact.
        } else {
            let dst_x86_reg = self.mov_risc_src_to_x86_dst(src1, dst, X86_REG_TMP_EBX);
            let src2_x86_reg = self.load_risc_reg_as_x86_reg(src2, X86_REG_TMP_EAX);
            (src2_x86_reg, dst_x86_reg)
        }
    }
}

pub(crate) struct X86InstructionTranspiler;

impl InstructionTranspiler<X86Op> for X86InstructionTranspiler {
    fn transpile_op_imm(inst: &RvOpImm) -> Vec<UnresOp<X86Op>> {
        match inst.op() {
            ArithmeticOp::Basic(basic) => {
                basic_op_imm_asm(basic, inst.rs1(), inst.imm(), inst.rd())
            }
            ArithmeticOp::Shift(shift) => shift_op_imm_asm(
                shift,
                inst.rs1(),
                ImmU5_0(inst.imm().value() as u8),
                inst.rd(),
            ),
            ArithmeticOp::SetLessThan(slt) => {
                set_less_than_op_imm_asm(slt, inst.rs1(), inst.imm(), inst.rd())
            }
        }
    }

    fn transpile_op_reg(inst: &RvOpReg) -> Vec<UnresOp<X86Op>> {
        match inst.op() {
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(basic)) => {
                basic_op_reg_asm(basic, inst.rs1(), inst.rs2(), inst.rd())
            }
            RegArithmeticOp::Sub => sub_reg_asm(inst.rs1(), inst.rs2(), inst.rd()),
            RegArithmeticOp::NonSub(ArithmeticOp::Shift(shift)) => {
                shift_op_reg_asm(shift, inst.rs1(), inst.rs2(), inst.rd())
            }
            RegArithmeticOp::NonSub(ArithmeticOp::SetLessThan(slt)) => {
                set_less_than_op_reg_asm(slt, inst.rs1(), inst.rs2(), inst.rd())
            }
        }
    }

    fn transpile_lui(inst: &RvLui) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        code.mov_imm_to_risc_reg(inst.imm().value(), inst.rd());
        code.into_vec()
    }

    fn transpile_auipc(inst: &RvAuipc, target: RiscvAddress) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        code.mov_imm_to_risc_reg(target.as_i32(), inst.rd());
        code.into_vec()
    }

    fn transpile_mul_div_rem(inst: &RvMulDivRem, next_place: RiscvAddress) -> Vec<UnresOp<X86Op>> {
        match inst.op() {
            MulDivRemOp::MulLow => mul_low_op_asm(inst.rs1(), inst.rs2(), inst.rd()),
            MulDivRemOp::MulHigh(mul_high_op) => {
                mul_high_op_asm(mul_high_op, inst.rs1(), inst.rs2(), inst.rd())
            }
            MulDivRemOp::DivRem(div_rem_op) => {
                div_rem_op_asm(div_rem_op, inst.rs1(), inst.rs2(), inst.rd(), next_place)
            }
        }
    }

    fn transpile_jal(
        inst: &RvJal,
        target: RiscvAddress,
        next_place: RiscvAddress,
    ) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        code.mov_imm_to_risc_reg(next_place.as_i32(), inst.rd());
        code.push_sym(X86SymbolicOp::Jmp(target.into()));
        code.into_vec()
    }

    fn transpile_jalr(
        inst: &RvJalr,
        next_place: RiscvAddress,
        risc_code_end: RiscvAddress,
        trusted: bool,
    ) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        // Compute the JALR target (rs1 + imm) & !1. With IALIGN=32, RISC-V raises
        // instruction-address-misaligned when bit 1 is set, so that takes the panic exit. The
        // check is kept in trusted mode too: the trusted jump table (4-byte entries) is indexed
        // as `JumpTable + target`, so a misaligned target would read the middle of an entry.
        code.mov_risc_reg_to_x86_reg(inst.rs1(), X86_REG_TMP_EAX.d());
        code.extend([
            X86NativeOp::Add_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(inst.imm().value() as i32).into(),
            )
            .into(),
            X86NativeOp::And_Reg_RoI(X86_REG_TMP_EAX.d(), X86Immediate::from(!1_i32).into()).into(),
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
        ]);
        // Trusted mode skips the RISC-V target range check.
        if !trusted {
            // The target must be at most the end of `.text` (the exit sentinel).
            code.extend(bound_check_zero_imm_ops(
                0,
                risc_code_end.as_u32(),
                X86_REG_TMP_EAX,
            ));
        }
        // rd = return address. rs1 was already read, so rd == rs1 is safe.
        code.mov_imm_to_risc_reg(next_place.as_i32(), inst.rd());
        // Jump through the jump table to the RISC-V address in eax.
        code.extend(x86_jump_via_jump_table_ops(trusted));
        code.into_vec()
    }

    fn transpile_branch(inst: &RvBranch, target: SymbolicBranchTarget) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        // When both operands are the same register, the outcome is statically known: emit either
        // an unconditional jump or nothing (fall through to the next instruction).
        if inst.rs1() == inst.rs2() {
            let always_taken = matches!(
                inst.condition(),
                BranchCondition::Beq | BranchCondition::Bge | BranchCondition::Bgeu
            );
            if always_taken {
                code.push_sym(X86SymbolicOp::Jmp(target));
            }
            return code.into_vec();
        }
        let src1_x86_reg = code.load_risc_reg_as_x86_reg(inst.rs1(), X86_REG_TMP_EAX);
        let src2_x86_reg = code.load_risc_reg_as_x86_reg(inst.rs2(), X86_REG_TMP_EBX);
        code.push(X86NativeOp::Cmp_Reg_RoI(src1_x86_reg, src2_x86_reg.into()));
        code.push_sym(match inst.condition() {
            BranchCondition::Beq => X86SymbolicOp::Jcc(X86ConditionCode::Equal, target),
            BranchCondition::Bne => X86SymbolicOp::Jcc(X86ConditionCode::NotEqual, target),
            BranchCondition::Blt => X86SymbolicOp::Jcc(X86ConditionCode::Less, target),
            BranchCondition::Bge => X86SymbolicOp::Jcc(X86ConditionCode::GreaterEqual, target),
            BranchCondition::Bltu => X86SymbolicOp::Jcc(X86ConditionCode::Below, target),
            BranchCondition::Bgeu => X86SymbolicOp::Jcc(X86ConditionCode::AboveEqual, target),
        });
        code.into_vec()
    }

    fn transpile_load(
        inst: &RvLoad,
        readable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        code.mov_risc_reg_to_x86_reg(inst.rs1(), X86_REG_TMP_EAX.d());
        // Loads may read `[0, _data_end_value)`; `readable_range.end()` is inclusive (the last
        // valid start address). Trusted mode skips the check but still adds the immediate.
        if !trusted {
            code.extend(bound_check_ops(
                readable_range.start().as_u32(),
                readable_range.end().as_u32(),
                inst.imm().value() as i32,
                X86_REG_TMP_EAX,
            ));
        } else if inst.imm().value() != 0 {
            code.push(X86NativeOp::Add_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(inst.imm().value() as i32).into(),
            ));
        }
        code.extend([
            // Host address = start of `.original_risc_prog` (guest address 0) + guest address.
            X86SymbolicOp::Lea_Tmp_Mem(X86_REG_TMP_EBX, GlobalDataLabel::RoDataStart.into()).into(),
            X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EBX, X86_REG_TMP_EAX.q()).into(),
        ]);
        // Load with the access size and sign or zero extension.
        let dst_x86_reg = code.get_risc_reg_as_x86_reg(inst.rd(), X86_REG_TMP_EAX);
        code.push(X86NativeOp::Load_Reg_MemTmp(
            inst.size(),
            dst_x86_reg,
            X86_REG_TMP_EBX,
        ));
        code.mov_x86_reg_to_risc_reg(dst_x86_reg, inst.rd());
        code.into_vec()
    }

    fn transpile_store(
        inst: &RvStore,
        writable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        code.mov_risc_reg_to_x86_reg(inst.rs1(), X86_REG_TMP_EAX.d());
        // Stores may write `[_data_start_value, _data_end_value)`, from the start of `.stack` to
        // the end of `.ecall_data`; `writable_range.end()` is inclusive (the last valid start
        // address). Trusted mode skips the check but still adds the immediate.
        if !trusted {
            code.extend(bound_check_ops(
                writable_range.start().as_u32(),
                writable_range.end().as_u32(),
                inst.imm().value(),
                X86_REG_TMP_EAX,
            ));
        } else if inst.imm().value() != 0 {
            code.push(X86NativeOp::Add_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(inst.imm().value()).into(),
            ));
        }
        code.extend([
            // Host address = start of `.original_risc_prog` (guest address 0) + guest address.
            X86SymbolicOp::Lea_Tmp_Mem(X86_REG_TMP_EBX, GlobalDataLabel::RoDataStart.into()).into(),
            X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EBX, X86_REG_TMP_EAX.q()).into(),
        ]);
        // rs2 can go in eax: the address is now in ebx.
        let src_x86_reg2 = code.load_risc_reg_as_x86_reg(inst.rs2(), X86_REG_TMP_EAX);
        code.push(X86NativeOp::Store_MemTmp_Reg(
            inst.size(),
            X86_REG_TMP_EBX,
            src_x86_reg2,
        ));
        code.into_vec()
    }

    fn transpile_system_call(next_place: RiscvAddress) -> Vec<UnresOp<X86Op>> {
        let mut code = X86TranspiledCode::default();
        code.extend([
            // Save the resume address in eax and exit through `EcallExit`, which checks the
            // ecall number; the envelope then stores the address.
            X86NativeOp::Mov_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(next_place.as_i32()).into(),
            )
            .into(),
            X86SymbolicOp::Jmp(GlobalCodeLabel::EcallExit.into()).into(),
        ]);
        code.into_vec()
    }

    fn transpile_atomic(
        inst: &RvAtomic,
        readable_range: VirtualDataRange,
        writable_range: VirtualDataRange,
        trusted: bool,
    ) -> Vec<UnresOp<X86Op>> {
        // Transpiled code is single-threaded, so SC always succeeds: store rs2 to M[rs1] and
        // set rd to 0.
        if matches!(inst.op(), AtomicOp::Sc) {
            let mut ops = atomic_alignment_check_ops(inst.rs1(), trusted);
            ops.extend(Self::transpile_store(
                &RvStore::from(inst),
                writable_range,
                trusted,
            ));
            let mut zero_code = X86TranspiledCode::default();
            zero_code.mov_imm_to_risc_reg(0, inst.rd());
            ops.extend(zero_code.into_vec());
            return ops;
        }
        // LR: a plain word load from M[rs1].
        if matches!(inst.op(), AtomicOp::Lr) {
            let mut ops = atomic_alignment_check_ops(inst.rs1(), trusted);
            ops.extend(Self::transpile_load(
                &RvLoad::from(inst),
                readable_range,
                trusted,
            ));
            return ops;
        }
        // AMOs: rd = M[rs1]; M[rs1] = op(M[rs1], rs2). The address must be in the writable range.
        let mut code = X86TranspiledCode::default();
        code.mov_risc_reg_to_x86_reg(inst.rs1(), X86_REG_TMP_EAX.d());
        // Trusted mode skips the alignment and bounds checks. Neither check changes eax.
        if !trusted {
            // The address must be 4-byte aligned.
            code.extend([
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
            ]);
            code.extend(bound_check_zero_imm_ops(
                writable_range.start().as_u32(),
                writable_range.end().as_u32(),
                X86_REG_TMP_EAX,
            ));
        }
        // Host address = start of `.original_risc_prog` (guest address 0) + guest address.
        code.extend([
            X86SymbolicOp::Lea_Tmp_Mem(X86_REG_TMP_EBX, GlobalDataLabel::RoDataStart.into()).into(),
            X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EBX, X86_REG_TMP_EAX.q()).into(),
            // Both temporaries are needed below, so keep the host address on the host stack.
            X86NativeOp::Push(X86_REG_TMP_EBX.q()).into(),
            // ebx = old value.
            X86NativeOp::Load_Reg_MemTmp(LoadSize::OneWord, X86_REG_TMP_EBX.d(), X86_REG_TMP_EBX)
                .into(),
        ]);
        // Read rs2 into eax before writing rd, in case rd == rs2.
        code.mov_risc_reg_to_x86_reg(inst.rs2(), X86_REG_TMP_EAX.d());
        code.mov_x86_reg_to_risc_reg(X86_REG_TMP_EBX.d(), inst.rd());
        // ebx = op(ebx, eax)
        match inst.op() {
            AtomicOp::AmoSwap => code.push(X86NativeOp::Mov_Reg_RoI(
                X86_REG_TMP_EBX.d(),
                X86_REG_TMP_EAX.d().into(),
            )),
            AtomicOp::AmoAdd => code.push(X86NativeOp::Add_Reg_RoI(
                X86_REG_TMP_EBX.d(),
                X86_REG_TMP_EAX.d().into(),
            )),
            AtomicOp::AmoXor => code.push(X86NativeOp::Xor_Reg_RoI(
                X86_REG_TMP_EBX.d(),
                X86_REG_TMP_EAX.d().into(),
            )),
            AtomicOp::AmoOr => code.push(X86NativeOp::Or_Reg_RoI(
                X86_REG_TMP_EBX.d(),
                X86_REG_TMP_EAX.d().into(),
            )),
            AtomicOp::AmoAnd => code.push(X86NativeOp::And_Reg_RoI(
                X86_REG_TMP_EBX.d(),
                X86_REG_TMP_EAX.d().into(),
            )),
            _ => {
                // Min/max: keep ebx if it already wins the comparison, else take eax.
                let condition = match inst.op() {
                    AtomicOp::AmoMin => X86ConditionCode::Less, // X86_REG_TMP_EBX < X86_REG_TMP_EAX: already min
                    AtomicOp::AmoMax => X86ConditionCode::Greater, // X86_REG_TMP_EBX > X86_REG_TMP_EAX: already max
                    AtomicOp::AmoMinU => X86ConditionCode::Below,  // unsigned below: already min
                    AtomicOp::AmoMaxU => X86ConditionCode::Above,  // unsigned above: already max
                    _ => unreachable!(),
                };
                code.extend([
                    X86NativeOp::Cmp_Reg_RoI(X86_REG_TMP_EBX.d(), X86_REG_TMP_EAX.d().into())
                        .into(),
                    X86SymbolicOp::Jcc(condition, LocalCodeLabel::SmallSkip.into()).into(),
                    X86NativeOp::Mov_Reg_RoI(X86_REG_TMP_EBX.d(), X86_REG_TMP_EAX.d().into())
                        .into(),
                    UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
                ]);
            }
        }
        // Pop the host address into eax and store the new value.
        code.extend([
            X86NativeOp::Pop(X86_REG_TMP_EAX.q()).into(),
            X86NativeOp::Store_MemTmp_Reg(StoreSize::OneWord, X86_REG_TMP_EAX, X86_REG_TMP_EBX.d())
                .into(),
        ]);
        code.into_vec()
    }
}

// ADDI, ANDI, ORI and XORI.
fn basic_op_imm_asm(
    op: BasicOp,
    src: RiscvRegister,
    imm: RvImmI12,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // x0 op imm is imm, except for AND, where it is 0.
    if src == RiscvRegister::Zero {
        match op {
            BasicOp::Add | BasicOp::Or | BasicOp::Xor => {
                code.mov_imm_to_risc_reg(imm.value() as i32, dst);
            }
            BasicOp::And => {
                code.mov_imm_to_risc_reg(0, dst);
            }
        }
        return code.into_vec();
    }
    // src op 0 is src, except for AND, where it is 0.
    if imm.value() == 0 {
        match op {
            BasicOp::Add | BasicOp::Or | BasicOp::Xor => {
                code.mov_risc_src_to_dst(src, dst, X86_REG_TMP_EAX);
            }
            BasicOp::And => {
                code.mov_imm_to_risc_reg(0, dst);
            }
        }
        return code.into_vec();
    }
    // Operate on dst's host register (or eax), loaded with src.
    let dst_x86_reg = code.mov_risc_src_to_x86_dst(src, dst, X86_REG_TMP_EAX);
    code.push(match op {
        // A negative immediate becomes SUB of its negation; a 12-bit value cannot overflow.
        BasicOp::Add => {
            if imm.value() >= 0 {
                X86NativeOp::Add_Reg_RoI(dst_x86_reg, X86Immediate::from(imm.value() as i32).into())
            } else {
                X86NativeOp::Sub_Reg_RoI(
                    dst_x86_reg,
                    X86Immediate::from(-imm.value() as i32).into(),
                )
            }
        }
        BasicOp::And => {
            X86NativeOp::And_Reg_RoI(dst_x86_reg, X86Immediate::from(imm.value() as i32).into())
        }
        BasicOp::Or => {
            X86NativeOp::Or_Reg_RoI(dst_x86_reg, X86Immediate::from(imm.value() as i32).into())
        }
        BasicOp::Xor => {
            X86NativeOp::Xor_Reg_RoI(dst_x86_reg, X86Immediate::from(imm.value() as i32).into())
        }
    });
    // Write the result back to dst if it lives in the register store.
    code.mov_x86_reg_to_risc_reg(dst_x86_reg, dst);
    code.into_vec()
}

// ADD, AND, OR and XOR.
fn basic_op_reg_asm(
    op: BasicOp,
    src1: RiscvRegister,
    src2: RiscvRegister,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // x0 op src2 is src2, except for AND, where it is 0.
    if src1 == RiscvRegister::Zero {
        match op {
            BasicOp::Add | BasicOp::Or | BasicOp::Xor => {
                code.mov_risc_src_to_dst(src2, dst, X86_REG_TMP_EAX);
            }
            BasicOp::And => {
                code.mov_imm_to_risc_reg(0, dst);
            }
        }
        return code.into_vec();
    }
    // src1 op x0 is src1, except for AND, where it is 0.
    if src2 == RiscvRegister::Zero {
        match op {
            BasicOp::Add | BasicOp::Or | BasicOp::Xor => {
                code.mov_risc_src_to_dst(src1, dst, X86_REG_TMP_EAX);
            }
            BasicOp::And => {
                code.mov_imm_to_risc_reg(0, dst);
            }
        }
        return code.into_vec();
    }
    let (src2_x86_reg, dst_x86_reg) = code.load_risc_src1_src2_dst_as_x86(src1, src2, dst);
    code.push(match op {
        BasicOp::Add => X86NativeOp::Add_Reg_RoI(dst_x86_reg, src2_x86_reg.into()),
        BasicOp::And => X86NativeOp::And_Reg_RoI(dst_x86_reg, src2_x86_reg.into()),
        BasicOp::Or => X86NativeOp::Or_Reg_RoI(dst_x86_reg, src2_x86_reg.into()),
        BasicOp::Xor => X86NativeOp::Xor_Reg_RoI(dst_x86_reg, src2_x86_reg.into()),
    });
    // Write the result back to dst if it lives in the register store.
    code.mov_x86_reg_to_risc_reg(dst_x86_reg, dst);
    code.into_vec()
}

// SUB (RISC-V has no SUBI).
fn sub_reg_asm(
    src1: RiscvRegister,
    src2: RiscvRegister,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // src - src is 0.
    if src1 == src2 {
        code.mov_imm_to_risc_reg(0, dst);
        return code.into_vec();
    }
    // x0 - src2 is -src2.
    if src1 == RiscvRegister::Zero {
        code.mov_neg_risc_src_to_dst(src2, dst, X86_REG_TMP_EAX);
        return code.into_vec();
    }
    // src1 - x0 is src1.
    if src2 == RiscvRegister::Zero {
        code.mov_risc_src_to_dst(src1, dst, X86_REG_TMP_EAX);
        return code.into_vec();
    }
    let (src2_x86_reg, dst_x86_reg) = code.load_risc_src1_src2_dst_as_x86(src1, src2, dst);
    code.push(X86NativeOp::Sub_Reg_RoI(dst_x86_reg, src2_x86_reg.into()));
    // Write the result back to dst if it lives in the register store.
    code.mov_x86_reg_to_risc_reg(dst_x86_reg, dst);
    code.into_vec()
}

// SLLI, SRLI and SRAI. The shift amount is a 5-bit immediate, as on x86 for 32-bit operands.
fn shift_op_imm_asm(
    op: ShiftOp,
    src: RiscvRegister,
    imm: ImmU5_0,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // Shifting x0 yields 0.
    if src == RiscvRegister::Zero {
        code.mov_imm_to_risc_reg(0, dst);
        return code.into_vec();
    }
    // Shifting by 0 yields src.
    if imm.value() == 0 {
        code.mov_risc_src_to_dst(src, dst, X86_REG_TMP_EAX);
        return code.into_vec();
    }
    // Operate on dst's host register (or eax), loaded with src.
    let dst_x86_reg = code.mov_risc_src_to_x86_dst(src, dst, X86_REG_TMP_EAX);
    code.push(match op {
        ShiftOp::Sll => {
            X86NativeOp::Shl_Reg_Sft(dst_x86_reg, X86Imm8Bit::new(imm.value() as i8).into())
        }
        ShiftOp::Srl => {
            X86NativeOp::Shr_Reg_Sft(dst_x86_reg, X86Imm8Bit::new(imm.value() as i8).into())
        }
        ShiftOp::Sra => {
            X86NativeOp::Sar_Reg_Sft(dst_x86_reg, X86Imm8Bit::new(imm.value() as i8).into())
        }
    });
    code.mov_x86_reg_to_risc_reg(dst_x86_reg, dst);
    code.into_vec()
}

// SLL, SRL and SRA. x86 takes a register shift amount only in cl, and masks it to 5 bits for
// 32-bit operands, as RISC-V does.
fn shift_op_reg_asm(
    shift_op: ShiftOp,
    src1: RiscvRegister,
    src2: RiscvRegister,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // Shifting x0 yields 0.
    if src1 == RiscvRegister::Zero {
        code.mov_imm_to_risc_reg(0, dst);
        return code.into_vec();
    }
    // Shifting by x0 (0) yields src1.
    if src2 == RiscvRegister::Zero {
        code.mov_risc_src_to_dst(src1, dst, X86_REG_TMP_EAX);
        return code.into_vec();
    }
    let is_shift_amt_ecx = is_x86_reg(src2, REG_ECX);
    let is_dst_ecx = is_x86_reg(dst, REG_ECX);
    // Shift in eax.
    code.mov_risc_reg_to_x86_reg(src1, X86_REG_TMP_EAX.d());
    // ecx holds a0: save it unless it is the destination or already holds the shift amount.
    if !is_dst_ecx && !is_shift_amt_ecx {
        code.push(X86NativeOp::Push(REG_RCX));
    }
    code.mov_risc_reg_to_x86_reg(src2, REG_ECX);
    code.push(match shift_op {
        ShiftOp::Sll => X86NativeOp::Shl_Reg_Sft(X86_REG_TMP_EAX.d(), X86ShiftAmount::Cl),
        ShiftOp::Srl => X86NativeOp::Shr_Reg_Sft(X86_REG_TMP_EAX.d(), X86ShiftAmount::Cl),
        ShiftOp::Sra => X86NativeOp::Sar_Reg_Sft(X86_REG_TMP_EAX.d(), X86ShiftAmount::Cl),
    });
    code.mov_x86_reg_to_risc_reg(X86_REG_TMP_EAX.d(), dst);
    if !is_dst_ecx && !is_shift_amt_ecx {
        code.push(X86NativeOp::Pop(REG_RCX));
    }
    code.into_vec()
}

// SLTI and SLTIU.
fn set_less_than_op_imm_asm(
    sign: SignType,
    src: RiscvRegister,
    imm: RvImmI12,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // With src = x0 the result is known at transpile time.
    if src == RiscvRegister::Zero {
        let result = match sign {
            SignType::Signed => i32::from(imm.value() > 0),
            SignType::Unsigned => i32::from(imm.value() != 0),
        };
        code.mov_imm_to_risc_reg(result, dst);
        return code.into_vec();
    }
    // No value is unsigned-less-than 0.
    if matches!(sign, SignType::Unsigned) && imm.value() == 0 {
        code.mov_imm_to_risc_reg(0, dst);
        return code.into_vec();
    }
    let src_x86_reg = code.load_risc_reg_as_x86_reg(src, X86_REG_TMP_EAX);
    code.push(X86NativeOp::Cmp_Reg_RoI(
        src_x86_reg,
        X86Immediate::from(imm.value() as i32).into(),
    ));
    let dst_x86_reg = code.get_risc_reg_as_x86_reg(dst, X86_REG_TMP_EBX);
    code.push(X86NativeOp::SetLow8(sign, dst_x86_reg.b()));
    // SETcc writes only the low byte; zero-extend it to 32 bits.
    code.push(X86NativeOp::Movzx_Self(dst_x86_reg));
    code.mov_x86_reg_to_risc_reg(dst_x86_reg, dst);
    code.into_vec()
}

// SLT and SLTU.
fn set_less_than_op_reg_asm(
    sign: SignType,
    src1: RiscvRegister,
    src2: RiscvRegister,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // Nothing is less than itself, and no value is unsigned-less-than 0.
    if src1 == src2 || (matches!(sign, SignType::Unsigned) && src2 == RiscvRegister::Zero) {
        code.mov_imm_to_risc_reg(0, dst);
        return code.into_vec();
    }
    let src1_x86_reg = code.load_risc_reg_as_x86_reg(src1, X86_REG_TMP_EAX);
    let src2_x86_reg = code.load_risc_reg_as_x86_reg(src2, X86_REG_TMP_EBX);
    code.push(X86NativeOp::Cmp_Reg_RoI(src1_x86_reg, src2_x86_reg.into()));
    let dst_x86_reg = code.get_risc_reg_as_x86_reg(dst, X86_REG_TMP_EBX);
    code.push(X86NativeOp::SetLow8(sign, dst_x86_reg.b()));
    // SETcc writes only the low byte; zero-extend it to 32 bits.
    code.push(X86NativeOp::Movzx_Self(dst_x86_reg));
    code.mov_x86_reg_to_risc_reg(dst_x86_reg, dst);
    code.into_vec()
}

// MUL: the low 32 bits of the product, which are the same for signed and unsigned operands.
fn mul_low_op_asm(
    src1: RiscvRegister,
    src2: RiscvRegister,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // Multiplying by x0 yields 0.
    if src1 == RiscvRegister::Zero || src2 == RiscvRegister::Zero {
        code.mov_imm_to_risc_reg(0, dst);
        return code.into_vec();
    }
    let (src2_x86_reg, dst_x86_reg) = code.load_risc_src1_src2_dst_as_x86(src1, src2, dst);
    // Two-operand IMUL, which does not use edx:eax.
    code.push(X86NativeOp::Mul_Reg_RoI(dst_x86_reg, src2_x86_reg.into()));
    code.mov_x86_reg_to_risc_reg(dst_x86_reg, dst);
    code.into_vec()
}

// MULH, MULHU and MULHSU: the high 32 bits of the 64-bit product, which one-operand MUL or IMUL
// leaves in edx.
fn mul_high_op_asm(
    op: MulHighOp,
    src1: RiscvRegister,
    src2: RiscvRegister,
    dst: RiscvRegister,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    // Multiplying by x0 yields 0.
    if src1 == RiscvRegister::Zero || src2 == RiscvRegister::Zero {
        code.mov_imm_to_risc_reg(0, dst);
        return code.into_vec();
    }
    // edx holds a1: save it unless it is the destination. eax is a temporary.
    let is_dst_edx = is_x86_reg(dst, REG_EDX);
    if !is_dst_edx {
        code.push(X86NativeOp::Push(REG_RDX));
    }
    code.mov_risc_reg_to_x86_reg(src2, X86_REG_TMP_EBX.d());
    // One-operand MUL takes src1 in eax.
    code.mov_risc_reg_to_x86_reg(src1, REG_EAX);
    match op {
        // high32( (int32)rs1 * (int32)rs2 )
        MulHighOp::Mulh => code.push(X86NativeOp::MulClobber(
            SignType::Signed,
            X86_REG_TMP_EBX.d(),
        )),
        // high32( (uint32)rs1 * (uint32)rs2 )
        MulHighOp::Mulhu => code.push(X86NativeOp::MulClobber(
            SignType::Unsigned,
            X86_REG_TMP_EBX.d(),
        )),
        // high32( (int32)rs1 * (uint32)rs2 )
        MulHighOp::Mulhsu => {
            // x86 has no signed-by-unsigned multiply, so multiply as signed and correct the
            // high half when src2 has its top bit set. The multiply overwrites eax (src1), so
            // save src1 on the host stack across it.
            code.extend([
                X86NativeOp::Push(X86_REG_TMP_EAX.q()).into(),
                X86NativeOp::MulClobber(SignType::Signed, X86_REG_TMP_EBX.d()).into(),
                X86NativeOp::Pop(X86_REG_TMP_EAX.q()).into(),
                // The multiply leaves src2 in ebx.
                X86NativeOp::Test_Reg_RoI(X86_REG_TMP_EBX.d(), X86_REG_TMP_EBX.d().into()).into(),
                X86SymbolicOp::Jcc(X86ConditionCode::NotSign, LocalCodeLabel::SmallSkip.into())
                    .into(),
                // With the top bit of src2 set, (u32)src2 = (i32)src2 + 2^32, so
                //   (i32)src1 * (u32)src2 = (i32)src1 * (i32)src2 + src1 * 2^32,
                // and the high half needs src1 added.
                X86NativeOp::Add_Reg_RoI(REG_EDX, REG_EAX.into()).into(),
                UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
            ]);
        }
    }
    code.mov_x86_reg_to_risc_reg(REG_EDX, dst);
    if !is_dst_edx {
        code.push(X86NativeOp::Pop(REG_RDX));
    }
    code.into_vec()
}

// DIV, DIVU, REM and REMU. x86 DIV and IDIV divide edx:eax, leaving the quotient in eax and
// the remainder in edx, and fault on division by zero or signed overflow, so both cases are
// handled first with the results the RISC-V spec defines.
fn div_rem_op_asm(
    op: DivRemOp,
    src1_dividend: RiscvRegister,
    src2_divisor: RiscvRegister,
    dst: RiscvRegister,
    next_place: RiscvAddress,
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    let (sign, is_div, eax_or_edx) = match op {
        DivRemOp::Div(sign) => (sign, true, REG_EAX),
        DivRemOp::Rem(sign) => (sign, false, REG_EDX),
    };
    // Divisor x0: division by zero is known at transpile time.
    if src2_divisor == RiscvRegister::Zero {
        if is_div {
            code.mov_imm_to_risc_reg(-1, dst);
        } else {
            code.mov_risc_src_to_dst(src1_dividend, dst, X86_REG_TMP_EAX);
        }
        return code.into_vec();
    }
    code.mov_risc_reg_to_x86_reg(src2_divisor, X86_REG_TMP_EBX.d());
    code.extend([
        X86NativeOp::Test_Reg_RoI(X86_REG_TMP_EBX.d(), X86_REG_TMP_EBX.d().into()).into(),
        X86SymbolicOp::Jcc(X86ConditionCode::NotEqual, LocalCodeLabel::SmallSkip.into()).into(), // jump to 'divisor is not zero'
    ]);
    // Division by zero: the quotient is -1 and the remainder is the dividend.
    if is_div {
        code.mov_imm_to_risc_reg(-1, dst);
    } else {
        code.mov_risc_src_to_dst(src1_dividend, dst, X86_REG_TMP_EAX);
    }
    code.extend([
        X86SymbolicOp::Jmp(next_place.into()).into(), // jump to the next instruction
        UnresOp::Label(LocalCodeLabel::SmallSkip.into()),
    ]); // 'divisor is not zero'
    let dividend = code.load_risc_reg_as_x86_reg(src1_dividend, X86_REG_TMP_EAX);
    // Signed overflow (i32::MIN / -1): the quotient is i32::MIN and the remainder is 0.
    if matches!(sign, SignType::Signed) {
        code.extend([
            X86NativeOp::Cmp_Reg_RoI(dividend, X86Immediate::from(i32::MIN).into()).into(),
            X86SymbolicOp::Jcc(X86ConditionCode::NotEqual, LocalCodeLabel::SmallSkip.into()).into(), // jump to 'normal division'
            X86NativeOp::Cmp_Reg_RoI(X86_REG_TMP_EBX.d(), X86Immediate::from(-1).into()).into(),
            X86SymbolicOp::Jcc(X86ConditionCode::NotEqual, LocalCodeLabel::SmallSkip.into()).into(), // jump to 'normal division'
        ]);
        if is_div {
            code.mov_imm_to_risc_reg(i32::MIN, dst);
        } else {
            code.mov_imm_to_risc_reg(0, dst);
        }
        code.extend([
            X86SymbolicOp::Jmp(next_place.into()).into(), // jump to the next instruction
            UnresOp::Label(LocalCodeLabel::SmallSkip.into()), // 'normal division'
        ]);
    }
    // edx holds a1: save it unless it is the destination.
    let is_dst_edx = is_x86_reg(dst, REG_EDX);
    if !is_dst_edx {
        code.push(X86NativeOp::Push(REG_RDX));
    }
    // eax is a temporary. Put the dividend in eax, then sign- or zero-extend it into edx.
    if dividend != REG_EAX {
        code.push(X86NativeOp::Mov_Reg_RoI(REG_EAX, dividend.into()));
    }
    code.push(if matches!(sign, SignType::Signed) {
        X86NativeOp::Cdq
    } else {
        X86NativeOp::Xor_Self(REG_EDX.into())
    });
    code.push(X86NativeOp::DivClobber(sign, X86_REG_TMP_EBX.d()));
    code.mov_x86_reg_to_risc_reg(eax_or_edx, dst);
    if !is_dst_edx {
        code.push(X86NativeOp::Pop(REG_RDX));
    }
    code.into_vec()
}

// Jumps to the native code of the RISC-V address in eax, which must be 4-byte aligned and
// within the table. Clobbers eax and ebx.
//
// RISC-V instructions are 4 bytes, so the entry for address `a` is at `JumpTable + a` for the
// trusted 4-byte entries and `JumpTable + 2 * a` for the untrusted 8-byte ones. An untrusted
// entry is `(gas_cost, offset)`: the stub deducts the gas (panicking on underflow), then follows
// the offset at +4. A trusted entry holds only the offset, and no gas is charged. The offset is
// relative to the offset field itself.
pub(crate) fn x86_jump_via_jump_table_ops(trusted: bool) -> Vec<UnresOp<X86Op>> {
    let mut ops = vec![
        X86SymbolicOp::Lea_Tmp_Mem(X86_REG_TMP_EBX, GlobalDataLabel::JumpTable.into()).into(),
        X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EBX, X86_REG_TMP_EAX.q()).into(),
    ];
    if !trusted {
        ops.extend([
            // Add the address again: untrusted entries are 8 bytes.
            X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EBX, X86_REG_TMP_EAX.q()).into(),
            X86NativeOp::Mov_Reg_MemTmp(X86_REG_TMP_EAX.d(), X86_REG_TMP_EBX).into(),
            X86NativeOp::Sub_Gas_Tmp(X86_REG_TMP_EAX).into(),
            // Gas below zero means out of gas.
            X86SymbolicOp::Jcc(X86ConditionCode::Sign, GlobalCodeLabel::ExitPanic.into()).into(),
            // Point ebx at the offset field.
            X86NativeOp::Mov_Reg_RoI(
                X86_REG_TMP_EAX.d(),
                X86Immediate::from(JUMP_TABLE_OFFSET_POSITION as i32).into(),
            )
            .into(),
            X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EBX, X86_REG_TMP_EAX.q()).into(),
        ]);
    }
    ops.extend([
        X86NativeOp::Mov_Reg_MemTmp(X86_REG_TMP_EAX.d(), X86_REG_TMP_EBX).into(),
        // The offset is signed: the code lies below the jump table.
        X86NativeOp::Movsxd_Tmp(X86_REG_TMP_EAX).into(),
        // Target = address of the offset field + offset.
        X86NativeOp::Add_Tmp64_Reg64(X86_REG_TMP_EAX, X86_REG_TMP_EBX.q()).into(),
        X86NativeOp::Jmp_Tmp(X86_REG_TMP_EAX).into(),
    ]);
    ops
}

// Adds `imm` to the guest address in `tmp_reg` (wrapping, as RISC-V does) and takes the panic exit
// unless the result is in `[lower_bound, upper_bound]`; `upper_bound` is inclusive. Both compares
// are unsigned, so addresses with the top bit set fail the upper-bound check.
fn bound_check_ops(
    lower_bound: u32,
    upper_bound: u32,
    imm: i32,
    tmp_reg: X86TempReg, // Contains the base value to add imm and check bounds
) -> Vec<UnresOp<X86Op>> {
    let mut code = X86TranspiledCode::default();
    if imm != 0 {
        code.push(X86NativeOp::Add_Reg_RoI(
            tmp_reg.d(),
            X86Immediate::from(imm).into(),
        ));
    }
    // Every address is at least 0, so a zero lower bound needs no check.
    if lower_bound != 0 {
        code.extend([
            X86NativeOp::Cmp_Reg_RoI(tmp_reg.d(), X86Immediate::from(lower_bound as i32).into())
                .into(),
            X86SymbolicOp::Jcc(X86ConditionCode::Below, GlobalCodeLabel::ExitPanic.into()).into(),
        ]);
    }
    code.extend([
        X86NativeOp::Cmp_Reg_RoI(tmp_reg.d(), X86Immediate::from(upper_bound as i32).into()).into(),
        X86SymbolicOp::Jcc(X86ConditionCode::Above, GlobalCodeLabel::ExitPanic.into()).into(),
    ]);
    code.into_vec()
}

// `bound_check_ops` with no immediate: `tmp_reg` is left unchanged, so the caller can reuse it.
fn bound_check_zero_imm_ops(
    lower_bound: u32,
    upper_bound: u32,
    tmp_reg: X86TempReg, // Contains the value to check bounds
) -> Vec<UnresOp<X86Op>> {
    bound_check_ops(lower_bound, upper_bound, 0, tmp_reg)
}

// Takes the panic exit unless the address in `addr_reg` is 4-byte aligned, as atomics require.
// Clobbers eax.
fn atomic_alignment_check_ops(addr_reg: RiscvRegister, trusted: bool) -> Vec<UnresOp<X86Op>> {
    if trusted {
        vec![]
    } else {
        let mut code = X86TranspiledCode::default();
        code.mov_risc_reg_to_x86_reg(addr_reg, X86_REG_TMP_EAX.d());
        code.push(X86NativeOp::Test_Reg_RoI(
            X86_REG_TMP_EAX.d(),
            X86Immediate::from(N_BYTES_IN_WORD_MINUS_1).into(),
        ));
        code.push_sym(X86SymbolicOp::Jcc(
            X86ConditionCode::NotEqual,
            GlobalCodeLabel::ExitPanic.into(),
        ));
        code.into_vec()
    }
}
