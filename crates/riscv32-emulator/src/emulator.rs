//! The emulator: guest state, the interpreter loop and the `user_code_runner` traits.
use std::collections::HashSet;
use std::sync::Arc;

use log::debug;
use riscv32::N_BYTES_IN_WORD;
use riscv32::blocks::direct_target;
use riscv32::common::SignType;
use riscv32::gas_cost::{DEFAULT_GAS_COST_SCHEME, RvGasCostScheme};
use riscv32::immediate::Immediate as _;
use riscv32::ops::{
    ArithmeticOp, AtomicOp, BasicOp, BranchCondition, DivRemOp, LoadSize, MulDivRemOp, MulHighOp,
    RegArithmeticOp, RiscvOp, ShiftOp,
};
use riscv32::registers::{RiscvRegister, RvPreservedRegister};
use riscv32_elf::data_layout::NO_ECALL_RETURN_ADDRESS;
use riscv32_elf::reader::read_riscv_elf;
use riscv32_elf::types::{EntryPointMapping, RiscvAddress, VirtualDataPointer, VirtualDataRange};
use user_code_runner::gas::UserCodeGasAmount;
use user_code_runner::runner::{
    CodeRunner, ILLEGAL_INSTRUCTION_EXIT_VALUE, MemoryAccessor, NORMAL_EXIT_VALUE,
    PANIC_EXIT_VALUE, RegistersInOut, UserCodeRunnerError,
};

use crate::error::EmulatorError;
use crate::memory::{GuestMemory, GuestMemoryRanges};
use crate::program::{Instruction, Program, load_size, store_size};

/// Whether the emulator meters gas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GasMetering {
    /// Charge gas like untrusted transpiled code, with the given cost scheme.
    Metered(RvGasCostScheme),
    /// Charge no gas, like trusted transpiled code. Memory accesses are still checked.
    Unmetered,
}

impl Default for GasMetering {
    fn default() -> Self {
        Self::Metered(DEFAULT_GAS_COST_SCHEME)
    }
}

/// Index of a0 in the register file; a0 to a7 are x10 to x17.
const A0: usize = 10;
/// Index of a7, which selects what to run on entry and holds the exit value on exit.
const A7: usize = 17;

/// A RISC-V program loaded into the emulator, with its memory, registers and gas.
///
/// It behaves like the same program transpiled in untrusted mode: the same memory ranges, gas
/// blocks, ecall ABI and exit values. Run it with `user_code_runner::runner::call_entry_point`,
/// exactly as a host runs a `TranspiledCodeRunner`.
#[derive(Debug)]
pub struct Emulator {
    /// Shared, so a run can read it while changing the rest of the state.
    program: Arc<Program>,
    ranges: GuestMemoryRanges,
    memory: GuestMemory,
    stack: VirtualDataRange,
    /// x0 to x31; x0 is never written.
    registers: [u32; 32],
    gas: i64,
    metered: bool,
    /// Where a resume (a7 = 0) continues: the instruction after the last ecall, or
    /// [`NO_ECALL_RETURN_ADDRESS`].
    return_address: u32,
    /// The number of the last ecall, which a resume puts back in a7.
    ecall_number: u32,
    /// The addresses of the init functions, then the entry points: selector `i` runs `hooks[i - 1]`.
    hooks: Vec<u32>,
    n_init_array_functions: usize,
    entry_point_mapping: EntryPointMapping,
}

/// What stopped a run.
enum Stop {
    /// An exit, with its exit value.
    Exit(i32),
    /// An ecall, with its number.
    Ecall(u32),
}

/// The result of one instruction.
enum Flow {
    /// Continue at this instruction index.
    Next(usize),
    Stop(Stop),
}

impl Emulator {
    /// Loads a linked RISC-V ELF with the named entry points, as the transpiler reads it.
    ///
    /// # Errors
    ///
    /// Returns an error if the ELF is invalid, an entry point is missing, a section lies outside
    /// guest memory, or `metering` has a zero gas cost.
    pub fn load(
        riscv_elf_bytes: &[u8],
        entry_points: &HashSet<String>,
        metering: GasMetering,
    ) -> Result<Self, EmulatorError> {
        let scheme = match metering {
            GasMetering::Metered(RvGasCostScheme::Fixed(cost)) if cost.value() == 0 => {
                return Err(EmulatorError::ZeroGasCost);
            }
            GasMetering::Metered(scheme) => Some(scheme),
            GasMetering::Unmetered => None,
        };
        let riscv_elf = read_riscv_elf(riscv_elf_bytes, entry_points)?;
        let symbols = riscv_elf.symbol_values();
        let ranges = GuestMemoryRanges {
            code_end: symbols.text_end.as_u32(),
            data_start: symbols.data_start.as_u32(),
            data_end: symbols.data_end.as_u32(),
        };
        let stack_top = symbols.stack_top.as_u32();
        let stack = VirtualDataRange::new(
            VirtualDataPointer::new(stack_top.saturating_sub(riscv_elf.stack_size_in_bytes())),
            VirtualDataPointer::new(stack_top),
        );
        let entries = riscv_elf.entry_points();
        let hooks: Vec<u32> = entries
            .init_array_addrs()
            .iter()
            .chain(entries.entry_point_addrs())
            .map(RiscvAddress::as_u32)
            .collect();
        let program = Program::new(
            &riscv_elf.code_section_as_i32_vec()?,
            &ranges,
            &hooks,
            scheme,
        )?;
        let memory = GuestMemory::new(&riscv_elf, ranges.data_end)?;
        debug!(
            "emulator: {} instructions, code end {:#x}, data [{:#x}, {:#x})",
            program.entries.len(),
            ranges.code_end,
            ranges.data_start,
            ranges.data_end
        );
        Ok(Self {
            program: Arc::new(program),
            ranges,
            memory,
            stack,
            registers: [0; 32],
            gas: 0,
            metered: scheme.is_some(),
            return_address: NO_ECALL_RETURN_ADDRESS,
            ecall_number: 0,
            n_init_array_functions: entries.init_array_addrs().len(),
            hooks,
            entry_point_mapping: entries.name_to_index().clone(),
        })
    }

    /// Maps each entry-point name to the index to pass in a7. It is the mapping the transpiler
    /// returns for the same program.
    pub fn entry_point_mapping(&self) -> &EntryPointMapping {
        &self.entry_point_mapping
    }

    /// Sets the address a return from an ecall (a7 = 0) resumes at. A resume takes the panic
    /// exit if it is not a 4-aligned address within the code.
    pub fn set_ecall_return_address(&mut self, address: RiscvAddress) {
        self.return_address = address.as_u32();
    }

    fn reg(&self, register: RiscvRegister) -> u32 {
        self.registers
            .get(register as usize)
            .copied()
            .unwrap_or_default()
    }

    fn set_reg(&mut self, register: RiscvRegister, value: u32) {
        if register != RiscvRegister::Zero
            && let Some(slot) = self.registers.get_mut(register as usize)
        {
            *slot = value;
        }
    }

    /// Subtracts `cost` from the gas and reports whether the gas ran out. Unmetered runs never
    /// do.
    fn charge(&mut self, cost: i64) -> bool {
        if !self.metered {
            return false;
        }
        self.gas = self.gas.saturating_sub(cost);
        self.gas < 0
    }

    /// Jumps through the jump table to `target`, which is 4-aligned and before the end of the
    /// code. Entering mid-block charges the rest of the block; entering at a block start
    /// charges 0 here, and the block's own charge follows.
    fn jump_indirect(&mut self, target: u32) -> Flow {
        let index = (target / N_BYTES_IN_WORD as u32) as usize;
        let cost = self
            .program
            .entries
            .get(index)
            .map_or(0, |entry| entry.indirect_cost);
        if self.charge(cost) {
            return Flow::Stop(Stop::Exit(PANIC_EXIT_VALUE));
        }
        Flow::Next(index)
    }

    /// Enters the program as the envelope does: resumes after an ecall if a7 is 0, else calls
    /// the init function or entry point that a7 selects.
    fn enter(&mut self) -> Stop {
        let selector = self.registers[A7];
        let flow = if selector == 0 {
            let target = self.return_address;
            if !target.is_multiple_of(N_BYTES_IN_WORD as u32) || target > self.ranges.code_end {
                return Stop::Exit(PANIC_EXIT_VALUE);
            }
            // Resuming at the end of the code is like falling off the last instruction.
            if target == self.ranges.code_end {
                return Stop::Exit(ILLEGAL_INSTRUCTION_EXIT_VALUE);
            }
            self.registers[A7] = self.ecall_number;
            self.jump_indirect(target)
        } else {
            // The function returns to the end of the code, which exits normally.
            self.set_reg(RiscvRegister::Ra, self.ranges.code_end);
            match self.hooks.get(selector.wrapping_sub(1) as usize) {
                Some(&address) => Flow::Next((address / N_BYTES_IN_WORD as u32) as usize),
                None => return Stop::Exit(PANIC_EXIT_VALUE),
            }
        };
        match flow {
            Flow::Next(index) => self.execute_from(index),
            Flow::Stop(stop) => stop,
        }
    }

    /// Runs from instruction `index` until the program stops.
    fn execute_from(&mut self, mut index: usize) -> Stop {
        let program = Arc::clone(&self.program);
        loop {
            // Falling off the end of the code, or branching to it, is an illegal instruction.
            let Some(entry) = program.entries.get(index) else {
                return Stop::Exit(ILLEGAL_INSTRUCTION_EXIT_VALUE);
            };
            if let Some(cost) = entry.block_cost
                && self.charge(cost)
            {
                return Stop::Exit(PANIC_EXIT_VALUE);
            }
            let flow = match &entry.instruction {
                Instruction::Op(op) => self.step(op, index),
                Instruction::Illegal => return Stop::Exit(ILLEGAL_INSTRUCTION_EXIT_VALUE),
            };
            match flow {
                Flow::Next(next) => index = next,
                Flow::Stop(stop) => return stop,
            }
        }
    }

    /// Runs one instruction at `index`.
    fn step(&mut self, instruction: &RiscvOp, index: usize) -> Flow {
        let word = N_BYTES_IN_WORD as u32;
        let address = (index as u32).wrapping_mul(word);
        let next = index.wrapping_add(1);
        let panic = Flow::Stop(Stop::Exit(PANIC_EXIT_VALUE));
        match instruction {
            RiscvOp::OpImm(op) => {
                let imm = i32::from(op.imm().value()) as u32;
                let value = arithmetic(op.op(), self.reg(op.rs1()), imm);
                self.set_reg(op.rd(), value);
            }
            RiscvOp::OpReg(op) => {
                let (a, b) = (self.reg(op.rs1()), self.reg(op.rs2()));
                let value = match op.op() {
                    RegArithmeticOp::Sub => a.wrapping_sub(b),
                    RegArithmeticOp::NonSub(arith) => arithmetic(arith, a, b),
                };
                self.set_reg(op.rd(), value);
            }
            RiscvOp::Lui(op) => self.set_reg(op.rd(), op.imm().value() as u32),
            RiscvOp::Auipc(op) => {
                self.set_reg(op.rd(), address.wrapping_add(op.imm().value() as u32));
            }
            RiscvOp::MulDivRem(op) => {
                let value = mul_div_rem(op.op(), self.reg(op.rs1()), self.reg(op.rs2()));
                self.set_reg(op.rd(), value);
            }
            RiscvOp::Jal(op) => {
                // A jump to an invalid target faults; `rd` is not written.
                let Some(target) = direct_target(address, op.imm().value(), self.ranges.code_end)
                else {
                    return panic;
                };
                self.set_reg(op.rd(), address.wrapping_add(word));
                return Flow::Next((target / word) as usize);
            }
            RiscvOp::Jalr(op) => {
                let imm = i32::from(op.imm().value()) as u32;
                let target = self.reg(op.rs1()).wrapping_add(imm) & !1;
                // A misaligned target faults, and one past the code leaves the sandbox; `rd` is
                // not written.
                if !target.is_multiple_of(word) || target > self.ranges.code_end {
                    return panic;
                }
                self.set_reg(op.rd(), address.wrapping_add(word));
                // The end of the code is where entry points return to.
                if target == self.ranges.code_end {
                    return Flow::Stop(Stop::Exit(NORMAL_EXIT_VALUE));
                }
                return self.jump_indirect(target);
            }
            RiscvOp::Branch(op) => {
                let (a, b) = (self.reg(op.rs1()), self.reg(op.rs2()));
                if branch_taken(op.condition(), a, b) {
                    // An invalid target faults only when the branch is taken.
                    let Some(target) =
                        direct_target(address, op.imm().value(), self.ranges.code_end)
                    else {
                        return panic;
                    };
                    return Flow::Next((target / word) as usize);
                }
            }
            RiscvOp::Load(op) => {
                let size = load_size(op.size());
                let imm = i32::from(op.imm().value()) as u32;
                let target = self.reg(op.rs1()).wrapping_add(imm);
                if !self.ranges.can_load(target, size) {
                    return panic;
                }
                let Some(raw) = self.memory.read(target, size) else {
                    return panic;
                };
                let value = match op.size() {
                    LoadSize::OneByte(SignType::Signed) => i32::from(raw as u8 as i8) as u32,
                    LoadSize::HalfWord(SignType::Signed) => i32::from(raw as u16 as i16) as u32,
                    LoadSize::OneByte(SignType::Unsigned)
                    | LoadSize::HalfWord(SignType::Unsigned)
                    | LoadSize::OneWord => raw,
                };
                self.set_reg(op.rd(), value);
            }
            RiscvOp::Store(op) => {
                let size = store_size(op.size());
                let target = self.reg(op.rs1()).wrapping_add(op.imm().value() as u32);
                if !self.ranges.can_store(target, size) {
                    return panic;
                }
                let value = self.reg(op.rs2());
                if self.memory.write(target, size, value).is_none() {
                    return panic;
                }
            }
            RiscvOp::Ecall => {
                let number = self.registers[A7];
                // The exit values cannot be ecall numbers, so a guest cannot pose as an exit.
                if matches!(number as i32, NORMAL_EXIT_VALUE | PANIC_EXIT_VALUE)
                    || number as i32 == ILLEGAL_INSTRUCTION_EXIT_VALUE
                {
                    return Flow::Stop(Stop::Exit(ILLEGAL_INSTRUCTION_EXIT_VALUE));
                }
                self.return_address = address.wrapping_add(word);
                self.ecall_number = number;
                return Flow::Stop(Stop::Ecall(number));
            }
            // The guest runs on one thread, so a fence orders nothing.
            RiscvOp::Fence(_) => {}
            RiscvOp::Atomic(op) => {
                let target = self.reg(op.rs1());
                if !target.is_multiple_of(word) {
                    return panic;
                }
                let operand = self.reg(op.rs2());
                match op.op() {
                    AtomicOp::Lr => {
                        if !self.ranges.can_load(target, word) {
                            return panic;
                        }
                        let Some(value) = self.memory.read(target, word) else {
                            return panic;
                        };
                        self.set_reg(op.rd(), value);
                    }
                    // With one thread nothing can break the reservation, so SC always succeeds.
                    AtomicOp::Sc => {
                        if !self.ranges.can_store(target, word)
                            || self.memory.write(target, word, operand).is_none()
                        {
                            return panic;
                        }
                        self.set_reg(op.rd(), 0);
                    }
                    amo => {
                        if !self.ranges.can_store(target, word) {
                            return panic;
                        }
                        let Some(old) = self.memory.read(target, word) else {
                            return panic;
                        };
                        let new = amo_result(amo, old, operand);
                        if self.memory.write(target, word, new).is_none() {
                            return panic;
                        }
                        self.set_reg(op.rd(), old);
                    }
                }
            }
        }
        Flow::Next(next)
    }
}

/// An OP or OP-IMM operation on `a` and `b`.
fn arithmetic(op: ArithmeticOp, a: u32, b: u32) -> u32 {
    match op {
        ArithmeticOp::Basic(BasicOp::Add) => a.wrapping_add(b),
        ArithmeticOp::Basic(BasicOp::And) => a & b,
        ArithmeticOp::Basic(BasicOp::Or) => a | b,
        ArithmeticOp::Basic(BasicOp::Xor) => a ^ b,
        ArithmeticOp::Shift(ShiftOp::Sll) => a << (b & 31),
        ArithmeticOp::Shift(ShiftOp::Srl) => a >> (b & 31),
        ArithmeticOp::Shift(ShiftOp::Sra) => ((a as i32) >> (b & 31)) as u32,
        ArithmeticOp::SetLessThan(SignType::Signed) => u32::from((a as i32) < (b as i32)),
        ArithmeticOp::SetLessThan(SignType::Unsigned) => u32::from(a < b),
    }
}

/// An RV32M operation, with the RISC-V results for division by zero and overflow.
fn mul_div_rem(op: MulDivRemOp, a: u32, b: u32) -> u32 {
    let (sa, sb) = (a as i32, b as i32);
    match op {
        MulDivRemOp::MulLow => a.wrapping_mul(b),
        MulDivRemOp::MulHigh(MulHighOp::Mulh) => ((i64::from(sa) * i64::from(sb)) >> 32) as u32,
        MulDivRemOp::MulHigh(MulHighOp::Mulhu) => ((u64::from(a) * u64::from(b)) >> 32) as u32,
        MulDivRemOp::MulHigh(MulHighOp::Mulhsu) => ((i128::from(sa) * i128::from(b)) >> 32) as u32,
        MulDivRemOp::DivRem(DivRemOp::Div(SignType::Signed)) => match sb {
            0 => u32::MAX,
            -1 => sa.wrapping_neg() as u32,
            _ => (sa / sb) as u32,
        },
        MulDivRemOp::DivRem(DivRemOp::Div(SignType::Unsigned)) => {
            a.checked_div(b).unwrap_or(u32::MAX)
        }
        MulDivRemOp::DivRem(DivRemOp::Rem(SignType::Signed)) => match sb {
            0 => a,
            -1 => 0,
            _ => (sa % sb) as u32,
        },
        MulDivRemOp::DivRem(DivRemOp::Rem(SignType::Unsigned)) => a.checked_rem(b).unwrap_or(a),
    }
}

/// Whether a branch with `condition` on `a` and `b` is taken.
fn branch_taken(condition: BranchCondition, a: u32, b: u32) -> bool {
    match condition {
        BranchCondition::Beq => a == b,
        BranchCondition::Bne => a != b,
        BranchCondition::Blt => (a as i32) < (b as i32),
        BranchCondition::Bge => (a as i32) >= (b as i32),
        BranchCondition::Bltu => a < b,
        BranchCondition::Bgeu => a >= b,
    }
}

/// The value an AMO writes back, from the old memory value and rs2.
fn amo_result(op: AtomicOp, old: u32, operand: u32) -> u32 {
    match op {
        AtomicOp::AmoAdd => old.wrapping_add(operand),
        AtomicOp::AmoXor => old ^ operand,
        AtomicOp::AmoOr => old | operand,
        AtomicOp::AmoAnd => old & operand,
        AtomicOp::AmoMin => (old as i32).min(operand as i32) as u32,
        AtomicOp::AmoMax => (old as i32).max(operand as i32) as u32,
        AtomicOp::AmoMinU => old.min(operand),
        AtomicOp::AmoMaxU => old.max(operand),
        AtomicOp::AmoSwap | AtomicOp::Lr | AtomicOp::Sc => operand,
    }
}

impl MemoryAccessor for Emulator {
    fn get_sp(&self) -> VirtualDataPointer {
        VirtualDataPointer::new(self.reg(RiscvRegister::Sp))
    }

    fn set_sp(&mut self, sp: VirtualDataPointer) {
        self.set_reg(RiscvRegister::Sp, sp.as_u32());
    }

    fn read_register(&self, register: RvPreservedRegister) -> i32 {
        self.reg(RiscvRegister::from(register)) as i32
    }

    fn write_register(&mut self, register: RvPreservedRegister, value: i32) {
        self.set_reg(RiscvRegister::from(register), value as u32);
    }

    fn get_gas(&self) -> UserCodeGasAmount {
        UserCodeGasAmount::new(self.gas)
    }

    fn set_gas(&mut self, gas: UserCodeGasAmount) {
        self.gas = gas.clamped_to_min_stored().as_i64();
    }

    fn read_memory(
        &self,
        addr: VirtualDataPointer,
        buf: &mut [u8],
    ) -> Result<(), UserCodeRunnerError> {
        let out_of_bounds = || UserCodeRunnerError::GuestMemoryOutOfBounds {
            addr: addr.as_u32(),
            len: u32::try_from(buf.len()).unwrap_or(u32::MAX),
        };
        let len = u32::try_from(buf.len()).map_err(|_| out_of_bounds())?;
        let end = addr.as_u32().checked_add(len).ok_or_else(out_of_bounds)?;
        if end > self.ranges.data_end {
            return Err(out_of_bounds());
        }
        let bytes = self
            .memory
            .get(addr.as_u32(), buf.len())
            .ok_or_else(out_of_bounds)?;
        buf.copy_from_slice(bytes);
        Ok(())
    }

    fn write_memory(
        &mut self,
        addr: VirtualDataPointer,
        data: &[u8],
    ) -> Result<(), UserCodeRunnerError> {
        let out_of_bounds = || UserCodeRunnerError::GuestMemoryOutOfBounds {
            addr: addr.as_u32(),
            len: u32::try_from(data.len()).unwrap_or(u32::MAX),
        };
        let len = u32::try_from(data.len()).map_err(|_| out_of_bounds())?;
        let end = addr.as_u32().checked_add(len).ok_or_else(out_of_bounds)?;
        if addr.as_u32() < self.ranges.data_start || end > self.ranges.data_end {
            return Err(out_of_bounds());
        }
        let bytes = self
            .memory
            .get_mut(addr.as_u32(), data.len())
            .ok_or_else(out_of_bounds)?;
        bytes.copy_from_slice(data);
        Ok(())
    }
}

impl CodeRunner for Emulator {
    fn run_till_ecall_or_return(
        &mut self,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        // The host passes a0 to a7 in; a7 selects what to run.
        for (slot, &value) in self
            .registers
            .iter_mut()
            .skip(A0)
            .zip(inout_registers.registers())
        {
            *slot = value as u32;
        }
        let stop = self.enter();
        let a7 = match stop {
            Stop::Exit(exit_value) => {
                self.return_address = NO_ECALL_RETURN_ADDRESS;
                exit_value
            }
            Stop::Ecall(number) => number as i32,
        };
        let mut out = [0_i32; 8];
        for (slot, &value) in out.iter_mut().zip(self.registers.iter().skip(A0)) {
            *slot = value as i32;
        }
        if let Some(slot) = out.last_mut() {
            *slot = a7;
        }
        inout_registers.set_registers(out);
        Ok(())
    }

    fn n_init_array_functions(&self) -> usize {
        self.n_init_array_functions
    }

    fn n_entry_point_functions(&self) -> usize {
        self.hooks.len().saturating_sub(self.n_init_array_functions)
    }

    fn virtual_stack_range(&self) -> VirtualDataRange {
        self.stack
    }
}
