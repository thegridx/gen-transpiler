//! Transpiles one RISC-V instruction: decodes it, computes its gas cost, records branch targets
//! as block boundaries, and calls the target's `InstructionTranspiler`.
use core::fmt;
use riscv32::N_BYTES_IN_WORD;
use riscv32::blocks::{block_boundaries, direct_target};
use riscv32::gas_cost::RvGasCost;
use riscv32::immediate::Immediate as _;
use riscv32::ops::RvDecodeError;
use riscv32::ops::{LoadSize, RiscvOp, StoreSize};
use riscv32::registers::RiscvRegister;
use riscv32_elf::types::{RiscvAddress, RvSymbolValues, VirtualDataPointer, VirtualDataRange};
use std::collections::HashSet;

use crate::TranspilationMode;
use crate::layout::{GlobalCodeLabel, LocalCodeLabel};
use crate::traits::{InstructionTranspiler as _, TranspiledOp};
use crate::traits::{TranspilerImplementation, UnresOp};

/// A raw RISC-V instruction word and its guest address.
#[derive(Debug, Copy, Clone)]
pub(crate) struct RiscvInstruction {
    pub loc: RiscvAddress,
    pub inst: i32,
}
impl fmt::Display for RiscvInstruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RiscvInstruction {{ loc: {}, inst: 0x{:08x}}}",
            self.loc.as_u32(),
            self.inst
        )
    }
}

/// Why an instruction could not be transpiled. The program transpiler replaces such an
/// instruction with a jump to the illegal-instruction exit.
#[derive(Debug)]
#[expect(
    dead_code,
    reason = "the fields are read through `Debug`, when the failed instruction is logged"
)]
pub(crate) enum InstructionTranspilationError {
    /// The word is not a supported RV32IMA instruction.
    DecodeError {
        risc_inst: RiscvInstruction,
        decode_error: RvDecodeError,
    },
    /// A computed target or next address is not 4-byte aligned.
    IllegalRiscAddress {
        risc_inst: RiscvInstruction,
        risc_addr: RiscvAddress,
    },
    /// A computed target or next address is past the end of the code.
    TargetRiscLocBeyondCodeEnd {
        risc_inst: RiscvInstruction,
        risc_code_end_addr: RiscvAddress,
    },
    /// No address is valid for this memory access: the guest's memory is smaller than it.
    NoAccessibleMemory { risc_inst: RiscvInstruction },
}

#[derive(Clone)]
pub(crate) struct TranspiledCodeBlock<TO: TranspiledOp> {
    pub(crate) ops: Vec<UnresOp<TO>>,
    pub(crate) gas_cost: RvGasCost,
}

fn get_risc_target_address(
    risc_inst: RiscvInstruction,
    imm: i32,
    text_end: RiscvAddress,
) -> Result<RiscvAddress, InstructionTranspilationError> {
    // The end of the code itself is a valid target: it reaches the illegal-instruction exit.
    // `direct_target` is the same rule the block boundaries use.
    if let Some(target) = direct_target(risc_inst.loc.as_u32(), imm, text_end.as_u32()) {
        return Ok(RiscvAddress::new(target));
    }
    let risc_addr = risc_inst.loc.wrapping_add(imm as u32);
    if (risc_addr.as_u32() & 3) != 0 {
        Err(InstructionTranspilationError::IllegalRiscAddress {
            risc_inst,
            risc_addr,
        })
    } else {
        Err(InstructionTranspilationError::TargetRiscLocBeyondCodeEnd {
            risc_inst,
            risc_code_end_addr: text_end,
        })
    }
}

// Returns the address of the next instruction, which may be the end of the code but not past it.
fn get_risc_next_place(
    risc_inst: RiscvInstruction,
    text_end: RiscvAddress,
) -> Result<RiscvAddress, InstructionTranspilationError> {
    get_risc_target_address(risc_inst, N_BYTES_IN_WORD as i32, text_end)
}

// Loads may read `[0, _data_end_value)`. The range's `end` is inclusive: the last address a
// load of `load_size` may start at. `None` if no load of that size fits.
fn readable_range(symbol_values: &RvSymbolValues, load_size: LoadSize) -> Option<VirtualDataRange> {
    let last_start = symbol_values
        .data_end
        .as_u32()
        .checked_sub(load_size.remainder_size_in_bytes() as u32 + 1)?;
    Some(VirtualDataRange::new(
        VirtualDataPointer::new(0),
        VirtualDataPointer::new(last_start),
    ))
}

// Stores may write `[_data_start_value, _data_end_value)`. The range's `end` is inclusive: the
// last address a store of `store_size` may start at. `None` if no store of that size fits.
fn writable_range(
    symbol_values: &RvSymbolValues,
    store_size: StoreSize,
) -> Option<VirtualDataRange> {
    let start = symbol_values.data_start.as_u32();
    let last_start = symbol_values
        .data_end
        .as_u32()
        .checked_sub(store_size as u32)?;
    (last_start >= start).then(|| {
        VirtualDataRange::new(
            VirtualDataPointer::new(start),
            VirtualDataPointer::new(last_start),
        )
    })
}

fn transpile_if_rd_not_zero<TO: TranspiledOp>(
    rd: RiscvRegister,
    transpile: impl FnOnce() -> Result<Vec<UnresOp<TO>>, InstructionTranspilationError>,
) -> Result<Vec<UnresOp<TO>>, InstructionTranspilationError> {
    if rd == RiscvRegister::Zero {
        Ok(Vec::new())
    } else {
        transpile()
    }
}

/// Decodes and transpiles one RV32IMA instruction, and adds the addresses that start a new block
/// (branch targets and the instructions after jumps, branches and ecalls) to
/// `risc_block_boundaries`.
pub(crate) fn transpile_rv32ima_instruction<TI: TranspilerImplementation>(
    risc_inst: RiscvInstruction,
    symbol_values: &RvSymbolValues,
    risc_block_boundaries: &mut HashSet<RiscvAddress>,
    mode: &TranspilationMode,
) -> Result<TranspiledCodeBlock<TI::OpType>, InstructionTranspilationError> {
    let decoded = RiscvOp::decode(risc_inst.inst).map_err(|e| {
        InstructionTranspilationError::DecodeError {
            risc_inst,
            decode_error: e,
        }
    })?;

    // The block boundaries this instruction makes, shared with the emulator.
    risc_block_boundaries.extend(
        block_boundaries(
            &decoded,
            risc_inst.loc.as_u32(),
            symbol_values.text_end.as_u32(),
        )
        .into_iter()
        .map(RiscvAddress::new),
    );

    // With no valid address, a memory access becomes an illegal-instruction exit (fail closed).
    let no_memory = || InstructionTranspilationError::NoAccessibleMemory { risc_inst };
    let transpiled_code = match &decoded {
        RiscvOp::OpImm(op) => transpile_if_rd_not_zero(op.rd(), || {
            Ok(TI::InstructionTranspiler::transpile_op_imm(op))
        })?,
        RiscvOp::Lui(op) => {
            transpile_if_rd_not_zero(op.rd(), || Ok(TI::InstructionTranspiler::transpile_lui(op)))?
        }
        RiscvOp::Auipc(op) => transpile_if_rd_not_zero(op.rd(), || {
            let target = risc_inst.loc.wrapping_add(op.imm().value() as u32);
            Ok(TI::InstructionTranspiler::transpile_auipc(op, target))
        })?,
        RiscvOp::OpReg(op) => transpile_if_rd_not_zero(op.rd(), || {
            Ok(TI::InstructionTranspiler::transpile_op_reg(op))
        })?,
        RiscvOp::MulDivRem(op) => transpile_if_rd_not_zero(op.rd(), || {
            let next_place = get_risc_next_place(risc_inst, symbol_values.text_end)?;
            Ok(TI::InstructionTranspiler::transpile_mul_div_rem(
                op, next_place,
            ))
        })?,
        RiscvOp::Jal(op) => {
            let next_place = get_risc_next_place(risc_inst, symbol_values.text_end)?;
            match get_risc_target_address(risc_inst, op.imm().value(), symbol_values.text_end) {
                Ok(target) => TI::InstructionTranspiler::transpile_jal(op, target, next_place),
                // A jump to a misaligned target faults (instruction-address-misaligned), and one
                // past the code leaves the sandbox; `jal` is always taken, so it always panics,
                // as `jalr` does at run time. `rd` is not written.
                Err(_) => vec![UnresOp::new_branch(GlobalCodeLabel::ExitPanic.into())],
            }
        }
        RiscvOp::Jalr(op) => {
            let next_place = get_risc_next_place(risc_inst, symbol_values.text_end)?;
            TI::InstructionTranspiler::transpile_jalr(
                op,
                next_place,
                symbol_values.text_end,
                mode.is_trusted(),
            )
        }
        RiscvOp::Branch(op) => {
            match get_risc_target_address(risc_inst, op.imm().value(), symbol_values.text_end) {
                Ok(target) => TI::InstructionTranspiler::transpile_branch(op, target.into()),
                // An invalid target faults only if the branch is taken: when the inverted
                // branch is not taken, take the panic exit; otherwise fall through.
                Err(_) => {
                    let mut ops = TI::InstructionTranspiler::transpile_branch(
                        &op.inverted(),
                        LocalCodeLabel::SmallSkip.into(),
                    );
                    ops.push(UnresOp::new_branch(GlobalCodeLabel::ExitPanic.into()));
                    ops.push(UnresOp::Label(LocalCodeLabel::SmallSkip.into()));
                    ops
                }
            }
        }
        RiscvOp::Load(op) => TI::InstructionTranspiler::transpile_load(
            op,
            readable_range(symbol_values, op.size()).ok_or_else(no_memory)?,
            mode.is_trusted(),
        ),
        RiscvOp::Store(op) => TI::InstructionTranspiler::transpile_store(
            op,
            writable_range(symbol_values, op.size()).ok_or_else(no_memory)?,
            mode.is_trusted(),
        ),
        RiscvOp::Ecall => {
            let next_place = get_risc_next_place(risc_inst, symbol_values.text_end)?;
            TI::InstructionTranspiler::transpile_system_call(next_place)
        }
        // Transpiled code is single-threaded, so a fence orders nothing.
        RiscvOp::Fence(_) => Vec::new(),
        RiscvOp::Atomic(op) => TI::InstructionTranspiler::transpile_atomic(
            op,
            readable_range(symbol_values, LoadSize::OneWord).ok_or_else(no_memory)?,
            writable_range(symbol_values, StoreSize::OneWord).ok_or_else(no_memory)?,
            mode.is_trusted(),
        ),
    };
    let gas_cost = match mode {
        TranspilationMode::Trusted => RvGasCost::zero(),
        TranspilationMode::Untrusted(scheme) => scheme.cost(&decoded),
    };
    Ok(TranspiledCodeBlock {
        ops: transpiled_code,
        gas_cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_next_place_from_first_instruction_returns_second() {
        let result = get_risc_next_place(
            RiscvInstruction {
                inst: 0,
                loc: RiscvAddress::new(0),
            },
            RiscvAddress::new(8),
        )
        .unwrap();
        assert_eq!(result.as_u32(), 4);
    }

    #[test]
    fn get_next_place_at_last_instruction_returns_end_sentinel() {
        // risc_loc = end - 4: next_place == end, which is not > end → Ok
        let result = get_risc_next_place(
            RiscvInstruction {
                inst: 0,
                loc: RiscvAddress::new(4),
            },
            RiscvAddress::new(8),
        )
        .unwrap();
        assert_eq!(result.as_u32(), 8);
    }

    #[test]
    fn get_next_place_at_end_sentinel_exceeds_section_and_errors() {
        // risc_loc == end: next_place = end + 4 > end → Err
        let err = get_risc_next_place(
            RiscvInstruction {
                inst: 0,
                loc: RiscvAddress::new(8),
            },
            RiscvAddress::new(8),
        )
        .unwrap_err();
        assert!(
            matches!(err, InstructionTranspilationError::TargetRiscLocBeyondCodeEnd { risc_inst, risc_code_end_addr } if risc_inst.loc.as_u32() == 8 && risc_code_end_addr.as_u32() == 8),
            "expected TargetRiscLocBeyondCodeEnd with risc_loc=12 and end_addr=8, got {err:?}"
        );
    }

    #[test]
    fn get_next_place_past_end_errors() {
        assert!(
            get_risc_next_place(
                RiscvInstruction {
                    inst: 0,
                    loc: RiscvAddress::new(12),
                },
                RiscvAddress::new(8)
            )
            .is_err()
        );
    }

    #[test]
    fn get_next_place_single_instruction_section() {
        // end_text_section == 4: one instruction at 0, next_place = 4 == end → Ok
        assert_eq!(
            get_risc_next_place(
                RiscvInstruction {
                    inst: 0,
                    loc: RiscvAddress::new(0),
                },
                RiscvAddress::new(4)
            )
            .unwrap()
            .as_u32(),
            4
        );
        // Getting past the sentinel errors
        assert!(
            get_risc_next_place(
                RiscvInstruction {
                    inst: 0,
                    loc: RiscvAddress::new(4),
                },
                RiscvAddress::new(4)
            )
            .is_err()
        );
    }

    fn layout(data_start: u32, data_end: u32) -> RvSymbolValues {
        RvSymbolValues {
            text_end: RiscvAddress::new(8),
            data_start: RiscvAddress::new(data_start),
            stack_top: RiscvAddress::new(data_start),
            data_end: RiscvAddress::new(data_end),
        }
    }

    #[test]
    fn memory_ranges_end_at_the_last_valid_start_address() {
        let symbols = layout(0x100, 0x200);
        let read = readable_range(&symbols, LoadSize::OneWord).unwrap();
        assert_eq!((read.start().as_u32(), read.end().as_u32()), (0, 0x1FC));
        let write = writable_range(&symbols, StoreSize::HalfWord).unwrap();
        assert_eq!(
            (write.start().as_u32(), write.end().as_u32()),
            (0x100, 0x1FE)
        );
    }

    #[test]
    fn memory_ranges_too_small_for_the_access_are_empty() {
        assert!(readable_range(&layout(0, 2), LoadSize::OneWord).is_none());
        // Only 2 writable bytes: no word store fits.
        assert!(writable_range(&layout(0x100, 0x102), StoreSize::OneWord).is_none());
    }
}

#[cfg(test)]
mod native_size_tests {
    use std::collections::HashSet;

    use rand::Rng as _;
    use riscv32::blocks::MAX_BLOCK_INSTRUCTIONS;
    use riscv32_elf::types::{RiscvAddress, RvSymbolValues};

    use super::{RiscvInstruction, transpile_rv32ima_instruction};
    use crate::TranspilationMode;
    use crate::arch64::Arm64Transpiler;
    use crate::test_support::seeded_rng;
    use crate::traits::{TranspilerImplementation, UnresOp};
    use crate::x86_64::X86Transpiler;

    /// The most native code one RISC-V instruction may expand to. A block holds at most
    /// `MAX_BLOCK_INSTRUCTIONS` instructions, and on aarch64 a bounds check at the end of a block
    /// branches back to its start with a `B.cond`, which reaches ±1 MiB.
    const MAX_INSTRUCTION_NATIVE_BYTES: u32 = (1 << 20) / MAX_BLOCK_INSTRUCTIONS - 16;

    /// The largest native expansion of many random instructions, of every major opcode.
    fn largest_expansion<TI: TranspilerImplementation>(mode: &TranspilationMode) -> u32 {
        const OPCODES: [u32; 12] = [
            0x13, 0x33, 0x37, 0x17, 0x03, 0x23, 0x63, 0x6F, 0x67, 0x73, 0x2F, 0x0F,
        ];
        let symbol_values = RvSymbolValues {
            text_end: RiscvAddress::new(0x1000),
            data_start: RiscvAddress::new(0x8000),
            stack_top: RiscvAddress::new(0x1_8000),
            data_end: RiscvAddress::new(0x3_8000),
        };
        let mut rng = seeded_rng("largest_expansion");
        let mut largest = 0;
        for _ in 0..200_000 {
            let opcode = OPCODES[rng.random_range(0..OPCODES.len())];
            let word = (rng.random::<u32>() & !0x7F) | opcode;
            let instruction = RiscvInstruction {
                loc: RiscvAddress::new(4 * rng.random_range(0..0x400)),
                inst: word as i32,
            };
            if let Ok(block) = transpile_rv32ima_instruction::<TI>(
                instruction,
                &symbol_values,
                &mut HashSet::new(),
                mode,
            ) {
                let size: u32 = block.ops.iter().map(UnresOp::size_in_bytes).sum();
                largest = largest.max(size);
            }
        }
        largest
    }

    #[test]
    fn no_instruction_expands_past_the_branch_reach_budget() {
        for mode in [TranspilationMode::default(), TranspilationMode::Trusted] {
            for (target, largest) in [
                ("aarch64", largest_expansion::<Arm64Transpiler>(&mode)),
                ("x86_64", largest_expansion::<X86Transpiler>(&mode)),
            ] {
                println!("{target} {mode:?}: largest expansion {largest} bytes");
                assert!(
                    largest <= MAX_INSTRUCTION_NATIVE_BYTES,
                    "{target} {mode:?}: an instruction expands to {largest} bytes, more than \
                     {MAX_INSTRUCTION_NATIVE_BYTES}"
                );
            }
        }
    }
}
