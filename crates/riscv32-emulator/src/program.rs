//! The decoded program: one entry per instruction word, with its gas.
//!
//! Decoding happens once, at load time, so data can never run as code: a jump can only reach
//! an instruction of `.text`. Words the transpiler would turn into an illegal-instruction exit
//! become [`Instruction::Illegal`] here, for the same reasons.
use std::collections::BTreeSet;

use riscv32::N_BYTES_IN_WORD;
use riscv32::blocks::{block_boundaries, split_into_blocks, tail_costs};
use riscv32::gas_cost::{RvGasCost, RvGasCostScheme};
use riscv32::ops::{LoadSize, RiscvOp, StoreSize};

use crate::error::EmulatorError;
use crate::memory::GuestMemoryRanges;

/// One instruction word of the program.
#[derive(Debug, Clone)]
pub(crate) enum Instruction {
    /// A supported instruction.
    Op(RiscvOp),
    /// A word that takes the illegal-instruction exit: undecodable, or a memory access with no
    /// valid address at all.
    Illegal,
}

/// One instruction of the program, with the gas charged on entering it.
#[derive(Debug)]
pub(crate) struct ProgramEntry {
    pub(crate) instruction: Instruction,
    /// The gas charged when execution reaches this instruction by falling through or by a
    /// direct jump: the whole block's cost at a block start, else nothing.
    pub(crate) block_cost: Option<i64>,
    /// The gas charged when an indirect jump lands here: 0 at a block start, whose own charge
    /// follows, else the cost from this instruction to the end of its block.
    pub(crate) indirect_cost: i64,
}

/// The decoded program and its gas blocks.
#[derive(Debug)]
pub(crate) struct Program {
    pub(crate) entries: Vec<ProgramEntry>,
}

impl Program {
    /// Decodes `words`, the code at address 0, and splits it into gas blocks. `hooks` are the
    /// addresses of the init functions and entry points, which start blocks. With no `scheme`,
    /// every instruction costs 0.
    pub(crate) fn new(
        words: &[i32],
        ranges: &GuestMemoryRanges,
        hooks: &[u32],
        scheme: Option<RvGasCostScheme>,
    ) -> Result<Self, EmulatorError> {
        let code_end = ranges.code_end;
        let instructions: Vec<Instruction> = words
            .iter()
            .map(|&word| match RiscvOp::decode(word) {
                Ok(op) if has_valid_address(&op, ranges) => Instruction::Op(op),
                Ok(_) | Err(_) => Instruction::Illegal,
            })
            .collect();
        let costs: Vec<RvGasCost> = instructions
            .iter()
            .map(|instruction| match (instruction, scheme) {
                (Instruction::Op(op), Some(costs_of)) => costs_of.cost(op),
                (Instruction::Op(_), None) | (Instruction::Illegal, _) => RvGasCost::zero(),
            })
            .collect();

        let word = N_BYTES_IN_WORD as u32;
        let n_instructions = u32::try_from(instructions.len()).unwrap_or(u32::MAX);
        let mut boundaries: BTreeSet<u32> = BTreeSet::from([0, n_instructions]);
        boundaries.extend(hooks.iter().map(|address| address / word));
        for (address, instruction) in (0..).step_by(N_BYTES_IN_WORD).zip(&instructions) {
            if let Instruction::Op(op) = instruction {
                boundaries.extend(
                    block_boundaries(op, address, code_end)
                        .into_iter()
                        .map(|boundary| boundary / word),
                );
            }
        }
        let blocks = split_into_blocks(&costs, &boundaries)?;
        let tails = tail_costs(&costs, &blocks)?;

        let block_starts: BTreeSet<u32> = blocks.iter().map(|block| block.start).collect();
        let entries = (0_u32..)
            .zip(instructions)
            .zip(tails)
            .map(|((index, instruction), tail)| {
                let cost = i64::from(tail.value());
                let block_start = block_starts.contains(&index);
                ProgramEntry {
                    instruction,
                    block_cost: block_start.then_some(cost),
                    indirect_cost: if block_start { 0 } else { cost },
                }
            })
            .collect();
        Ok(Self { entries })
    }
}

/// Whether a memory access has any valid address. The transpiler turns one that has none into
/// an illegal-instruction exit (fail closed), so the emulator does too.
fn has_valid_address(op: &RiscvOp, ranges: &GuestMemoryRanges) -> bool {
    match op {
        RiscvOp::Load(load) => ranges.last_readable(load_size(load.size())).is_some(),
        RiscvOp::Store(store) => ranges.last_writable(store_size(store.size())).is_some(),
        RiscvOp::Atomic(_) => {
            ranges.last_readable(N_BYTES_IN_WORD as u32).is_some()
                && ranges.last_writable(N_BYTES_IN_WORD as u32).is_some()
        }
        _ => true,
    }
}

/// The number of bytes a load reads.
pub(crate) fn load_size(size: LoadSize) -> u32 {
    match size {
        LoadSize::OneByte(_) => 1,
        LoadSize::HalfWord(_) => 2,
        LoadSize::OneWord => 4,
    }
}

/// The number of bytes a store writes.
pub(crate) fn store_size(size: StoreSize) -> u32 {
    match size {
        StoreSize::OneByte => 1,
        StoreSize::HalfWord => 2,
        StoreSize::OneWord => 4,
    }
}
