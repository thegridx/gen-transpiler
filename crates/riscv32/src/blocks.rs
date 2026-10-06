//! Pseudo basic blocks: how a program's instructions are grouped for gas metering.
//!
//! Gas is charged per block, on entry, for the whole block. The transpiler and the emulator both
//! use this module, so they split a program into the same blocks and charge the same gas at the
//! same points.
//!
//! A block starts at address 0, at each init function and entry point, at the instruction after
//! every jump, branch and ecall, and at every valid direct branch or jump target. A block also
//! ends early when its cost would exceed [`ImmU12_0::MAX_VAL`], the largest cost one charge can
//! hold, or when it would hold more than [`MAX_BLOCK_INSTRUCTIONS`] instructions.
use std::collections::BTreeSet;
use std::ops::Range;

use thiserror::Error;

use crate::N_BYTES_IN_WORD;
use crate::common::ValueError;
use crate::gas_cost::RvGasCost;
use crate::immediate::{ImmU12_0, Immediate as _};
use crate::ops::RiscvOp;

/// The most instructions a block may hold. Every instruction costs at least 1 gas except
/// undecodable ones, so this only splits long runs of those. It bounds a block's native code,
/// which on aarch64 must stay within a conditional branch's ±1 MiB reach.
pub const MAX_BLOCK_INSTRUCTIONS: u32 = 4096;

/// Why a program could not be split into blocks.
#[derive(Debug, Error)]
pub enum BlockError {
    /// A block boundary lies past the end of the code.
    #[error(
        "block boundary at instruction {boundary} is beyond the code ({n_instructions} instructions)"
    )]
    BoundaryBeyondCode { boundary: u32, n_instructions: u32 },
    /// One instruction costs more than a block may.
    #[error("an instruction's gas cost {cost} exceeds the per-block maximum {max}")]
    GasCostTooLarge { cost: u16, max: u16 },
    /// Adding up costs overflowed.
    #[error("value error: {0}")]
    Value(#[from] ValueError),
}

/// The target of a direct branch or jump at `address` with offset `imm`, if it is valid: 4-byte
/// aligned and at most `code_end`. The end of the code itself is a valid target; reaching it
/// takes the illegal-instruction exit.
pub fn direct_target(address: u32, imm: i32, code_end: u32) -> Option<u32> {
    let target = address.wrapping_add(imm as u32);
    (target.is_multiple_of(N_BYTES_IN_WORD as u32) && target <= code_end).then_some(target)
}

/// The addresses where `op`, at `address`, makes a new block start: the next instruction after a
/// jump, branch or ecall, and the target of a direct branch or jump when it is valid.
pub fn block_boundaries(op: &RiscvOp, address: u32, code_end: u32) -> Vec<u32> {
    let next = address.wrapping_add(N_BYTES_IN_WORD as u32);
    match op {
        RiscvOp::Jal(jal) => [
            Some(next),
            direct_target(address, jal.imm().value(), code_end),
        ]
        .into_iter()
        .flatten()
        .collect(),
        RiscvOp::Branch(branch) => [
            Some(next),
            direct_target(address, branch.imm().value(), code_end),
        ]
        .into_iter()
        .flatten()
        .collect(),
        RiscvOp::Jalr(_) | RiscvOp::Ecall => vec![next],
        _ => Vec::new(),
    }
}

/// Splits instructions with gas `costs` into blocks: ranges of instruction indices. A block
/// starts at each index in `boundaries`, and ends early when its cost would exceed
/// [`ImmU12_0::MAX_VAL`] or it would hold more than [`MAX_BLOCK_INSTRUCTIONS`] instructions.
///
/// # Errors
///
/// Returns an error if a boundary lies past the end of the code or one instruction costs more
/// than a block may.
pub fn split_into_blocks(
    costs: &[RvGasCost],
    boundaries: &BTreeSet<u32>,
) -> Result<Vec<Range<u32>>, BlockError> {
    let n_instructions = u32::try_from(costs.len()).map_err(|_| ValueError::Overflow {
        base: costs.len() as u64,
        add: 0,
        context: "n_instructions".to_string(),
    })?;
    if let Some(&boundary) = boundaries.last()
        && boundary > n_instructions
    {
        return Err(BlockError::BoundaryBeyondCode {
            boundary,
            n_instructions,
        });
    }
    let mut blocks = Vec::new();
    let mut block_start = 0_u32;
    let mut block_cost = RvGasCost::zero();
    for (index, cost) in (0_u32..).zip(costs) {
        if cost.value() > ImmU12_0::MAX_VAL {
            return Err(BlockError::GasCostTooLarge {
                cost: cost.value(),
                max: ImmU12_0::MAX_VAL,
            });
        }
        let at_boundary = boundaries.contains(&index);
        let too_long = index.saturating_sub(block_start) >= MAX_BLOCK_INSTRUCTIONS;
        let too_costly = block_cost.checked_add(cost, "block_cost").is_err();
        if index > block_start && (at_boundary || too_long || too_costly) {
            blocks.push(block_start..index);
            block_start = index;
            block_cost = RvGasCost::zero();
        }
        block_cost = block_cost.checked_add(cost, "block_cost")?;
    }
    if block_start < n_instructions {
        blocks.push(block_start..n_instructions);
    }
    Ok(blocks)
}

/// For each instruction, the gas from it to the end of its block: what entering the block at
/// that instruction costs. A block's first instruction holds the whole block's cost.
///
/// # Errors
///
/// Returns an error if a block's cost overflows, which [`split_into_blocks`] prevents.
pub fn tail_costs(
    costs: &[RvGasCost],
    blocks: &[Range<u32>],
) -> Result<Vec<RvGasCost>, BlockError> {
    let mut tails = vec![RvGasCost::zero(); costs.len()];
    for block in blocks {
        let mut tail = RvGasCost::zero();
        for index in block.clone().rev() {
            let position = index as usize;
            let (Some(cost), Some(slot)) = (costs.get(position), tails.get_mut(position)) else {
                continue;
            };
            tail = tail.checked_add(cost, "tail_cost")?;
            *slot = tail;
        }
    }
    Ok(tails)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(value: u16) -> RvGasCost {
        RvGasCost::new(ImmU12_0(value))
    }

    #[test]
    fn blocks_start_at_boundaries() {
        let costs = vec![cost(1); 6];
        let boundaries = BTreeSet::from([0, 2, 5, 6]);
        let blocks = split_into_blocks(&costs, &boundaries).expect("valid");
        assert_eq!(blocks, [0..2, 2..5, 5..6]);
        let tails = tail_costs(&costs, &blocks).expect("valid");
        let tail_values: Vec<u16> = tails.iter().map(RvGasCost::value).collect();
        assert_eq!(tail_values, [2, 1, 3, 2, 1, 1]);
    }

    #[test]
    fn a_block_closes_before_its_cost_overflows() {
        let costs = vec![cost(4000), cost(95), cost(1), cost(7)];
        let blocks = split_into_blocks(&costs, &BTreeSet::from([0, 4])).expect("valid");
        assert_eq!(blocks, [0..2, 2..4]);
    }

    #[test]
    fn a_block_closes_at_the_instruction_cap() {
        let n = MAX_BLOCK_INSTRUCTIONS + 3;
        let costs = vec![RvGasCost::zero(); n as usize];
        let blocks = split_into_blocks(&costs, &BTreeSet::from([0, n])).expect("valid");
        assert_eq!(
            blocks,
            [0..MAX_BLOCK_INSTRUCTIONS, MAX_BLOCK_INSTRUCTIONS..n]
        );
    }

    #[test]
    fn a_boundary_past_the_code_is_rejected() {
        let costs = vec![cost(1); 2];
        assert!(matches!(
            split_into_blocks(&costs, &BTreeSet::from([0, 3])),
            Err(BlockError::BoundaryBeyondCode {
                boundary: 3,
                n_instructions: 2
            })
        ));
    }

    #[test]
    fn direct_targets_must_be_aligned_and_within_the_code() {
        assert_eq!(direct_target(8, -8, 16), Some(0));
        assert_eq!(direct_target(8, 8, 16), Some(16));
        assert_eq!(direct_target(8, 12, 16), None);
        assert_eq!(direct_target(8, 2, 16), None);
        assert_eq!(direct_target(0, -4, 16), None);
    }
}
