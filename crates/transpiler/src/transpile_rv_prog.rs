//! Transpiles a whole RISC-V program and writes the native ELF.
//!
//! The steps are:
//! 1. Transpile each instruction, and collect branch targets as block boundaries.
//! 2. Split the code into pseudo basic blocks, prepend one gas deduction to each block (untrusted
//!    mode only), and build the jump table.
//! 3. Append the envelope and the entry-point hook table: one branch per init function, then
//!    one per entry point. Every call from the host enters through `_start_transpiled_code`.
//! 4. Assign addresses to the global labels and resolve all branches to PC-relative offsets.
//! 5. Collect the sections and hand them to the ELF builder.
use log::debug;
use object::SectionKind;
use riscv32::N_BYTES_IN_WORD;
use riscv32::blocks::{split_into_blocks, tail_costs};
use riscv32::common::{
    SEGMENT_BYTE_ALIGN, checked_add_u32, checked_align_segment, checked_mul_u32,
    checked_usize_to_u32,
};
use riscv32::gas_cost::RvGasCost;
use riscv32_elf::consts::{ECALL_DATA_SECTION_NAME, HEAP_SECTION_NAME, STACK_SECTION_NAME};
use riscv32_elf::data_layout::{ECALL_DATA_SIZE_BYTES, REGISTER_STORE_SIZE_BYTES};
use riscv32_elf::reader::{RiscvElf, RvElfSection, RvEntryPoints};
use riscv32_elf::types::{RiscvAddress, RvSymbolValues};
use std::collections::{BTreeSet, HashSet};
use strum::IntoEnumIterator as _;

use crate::elf_builder::{
    AlignedBytes, NoBitsSection, OutputElfSections, RuntimeSections, TextSection,
    build_transpiled_elf, rodata_start_vaddr,
};
use crate::layout::{
    EF_TRUSTED_TRANSPILED, GlobalCodeLabel, GlobalDataLabel, GlobalLabel, GlobalLabelAddrMap,
    JUMP_TABLE_ENTRY_BYTE_SIZE, JUMP_TABLE_ENTRY_BYTE_SIZE_TRUSTED, JUMP_TABLE_OFFSET_POSITION,
    JumpTableEntry, MAX_CODE_SECTION_SIZE_IN_BYTES, MAX_TRANSPILED_ADDRESS, SymbolicBranchTarget,
    SymbolicLabel,
};
use crate::traits::{NativeOp as _, SymbolicOp as _, TranspiledOp, TranspilerImplementation};
use crate::traits::{TranspiledEnvelope as _, UnresOp, resolved_ops_to_bytecode};
use crate::transpile_rv32ima_inst::{
    RiscvInstruction, TranspiledCodeBlock, transpile_rv32ima_instruction,
};
use crate::transpiled_address::TranspiledAddress;
use crate::transpiled_address::TranspiledAddressResolver;
use crate::{TranspilationMode, TranspilerError};

// Convenience functions to work with multiple native ops
fn unresolved_ops_size_in_bytes<TO: TranspiledOp>(ops: &[UnresOp<TO>]) -> u32 {
    ops.iter().map(|op| op.size_in_bytes()).sum()
}

fn transpile_all_instructions<TI: TranspilerImplementation>(
    symbol_values: &RvSymbolValues,
    risc_prog: &[i32],
    risc_block_boundaries: &mut HashSet<RiscvAddress>,
    mode: &TranspilationMode,
) -> Vec<TranspiledCodeBlock<TI::OpType>> {
    let mut transpiled_code: Vec<TranspiledCodeBlock<TI::OpType>> = Vec::<_>::new();
    for (risc_index, risc_inst) in risc_prog.iter().enumerate() {
        let risc_loc = RiscvAddress::new(N_BYTES_IN_WORD as u32 * risc_index as u32);
        // Transpile the instruction and update block boundaries with any branch targets, if applicable
        let res = transpile_rv32ima_instruction::<TI>(
            RiscvInstruction {
                inst: *risc_inst,
                loc: risc_loc,
            },
            symbol_values,
            risc_block_boundaries,
            mode,
        );
        match res {
            Ok(cost_and_transpiled_inst) => {
                transpiled_code.push(cost_and_transpiled_inst);
            }
            // Errors are converted to jumps to illegal code
            Err(err) => {
                debug!(
                    "Error transpiling instruction at RISC-V address {:#x}: {err:?}",
                    risc_loc.as_u32()
                );
                transpiled_code.push(TranspiledCodeBlock {
                    gas_cost: RvGasCost::zero(),
                    ops: TI::EnvelopeType::branch_to_illegal_code(),
                });
            }
        }
    }
    transpiled_code
}

/// Splits the transpiled instructions into pseudo basic blocks (see `riscv32::blocks`) and sets
/// each instruction's `gas_cost` to the gas from it to the end of its block.
fn build_consecutive_chunks<TI: TranspilerImplementation>(
    transpiled_code: Vec<TranspiledCodeBlock<TI::OpType>>,
    risc_targets: HashSet<RiscvAddress>,
) -> Result<Vec<Vec<TranspiledCodeBlock<TI::OpType>>>, TranspilerError> {
    let n_instructions = checked_usize_to_u32(transpiled_code.len(), "n_instructions")?;
    // Block boundaries as instruction indices, always including the start and end of the code.
    let mut boundaries: BTreeSet<u32> = risc_targets.iter().map(|&addr| addr.index()).collect();
    boundaries.extend([0, n_instructions]);
    let costs: Vec<RvGasCost> = transpiled_code.iter().map(|inst| inst.gas_cost).collect();
    let blocks = split_into_blocks(&costs, &boundaries)?;
    let tails = tail_costs(&costs, &blocks)?;
    let mut instructions = transpiled_code.into_iter().zip(tails);
    Ok(blocks
        .iter()
        .map(|block| {
            instructions
                .by_ref()
                .take(block.len())
                .map(|(mut inst, tail)| {
                    inst.gas_cost = tail;
                    inst
                })
                .collect()
        })
        .collect())
}

struct TranspiledProgram<TO: TranspiledOp> {
    ops: Vec<UnresOp<TO>>,
    jump_table: Vec<JumpTableEntry>,
}

// Jump-table geometry and ELF header flags for each mode.
fn jump_table_entry_byte_size(mode: &TranspilationMode) -> u32 {
    match mode {
        TranspilationMode::Trusted => JUMP_TABLE_ENTRY_BYTE_SIZE_TRUSTED,
        TranspilationMode::Untrusted(_) => JUMP_TABLE_ENTRY_BYTE_SIZE,
    }
}
fn jump_table_offset_position(mode: &TranspilationMode) -> u32 {
    match mode {
        TranspilationMode::Trusted => 0,
        TranspilationMode::Untrusted(_) => JUMP_TABLE_OFFSET_POSITION,
    }
}
fn to_e_flags(mode: &TranspilationMode) -> u32 {
    match mode {
        TranspilationMode::Trusted => EF_TRUSTED_TRANSPILED,
        TranspilationMode::Untrusted(_) => 0,
    }
}

fn build_transpiled_prog<TI: TranspilerImplementation>(
    transpiled_code: Vec<TranspiledCodeBlock<TI::OpType>>,
    risc_targets: HashSet<RiscvAddress>,
    mode: &TranspilationMode,
) -> Result<TranspiledProgram<TI::OpType>, TranspilerError> {
    let consecutive_chunks = build_consecutive_chunks::<TI>(transpiled_code, risc_targets)?;
    // Emit the program and jump table from the cost-annotated pseudo basic blocks.
    let mut ops = Vec::new();
    let mut jump_table = Vec::new();
    let mut cur_transpiled_addr = TranspiledAddress::new(0);
    for mut pseudo_basic_block in consecutive_chunks.into_iter() {
        // In untrusted mode, the block's first instruction starts with a deduction of the
        // block's full cost, which panics on underflow. Trusted output has no gas metering.
        // A jump-table entry points at the start of its instruction's code, so a jump to a
        // block's first instruction runs that deduction; its entry therefore charges 0.
        let Some(first_inst) = pseudo_basic_block.first_mut() else {
            continue;
        };
        if !mode.is_trusted() {
            let gas_cost_ops =
                TI::EnvelopeType::deduct_gas_and_panic_trampoline(first_inst.gas_cost);
            first_inst.ops.splice(0..0, gas_cost_ops);
        }
        first_inst.gas_cost = RvGasCost::zero();
        for inst in pseudo_basic_block.into_iter() {
            let ops_byte_size = unresolved_ops_size_in_bytes(&inst.ops);
            ops.extend(inst.ops);
            jump_table.push(JumpTableEntry::new(inst.gas_cost, cur_transpiled_addr));
            cur_transpiled_addr = cur_transpiled_addr.checked_add(ops_byte_size)?;
        }
    }
    Ok(TranspiledProgram { ops, jump_table })
}

fn init_array_and_entry_points<TO: TranspiledOp>(entry_points: &RvEntryPoints) -> Vec<UnresOp<TO>> {
    // One branch per callable function: the init functions in `.init_array` order, then the
    // entry points in byte order of their names (the order `RvEntryPoints` keeps them in).
    let mut entry_point_jumps = Vec::<UnresOp<TO>>::new();
    entry_point_jumps.push(GlobalDataLabel::EntryPointsHooksBegin.into());
    entry_point_jumps.extend(
        entry_points
            .init_array_addrs()
            .iter()
            .map(|addr| UnresOp::new_branch(SymbolicBranchTarget::RiscLoc(*addr))),
    );
    entry_point_jumps.push(GlobalDataLabel::InitArrayEnd.into());
    entry_point_jumps.extend(
        entry_points
            .entry_point_addrs()
            .iter()
            .map(|addr| UnresOp::new_branch(SymbolicBranchTarget::RiscLoc(*addr))),
    );
    entry_point_jumps.push(GlobalDataLabel::EntryPointsHooksEnd.into());
    entry_point_jumps
}

fn update_global_addresses(
    label: GlobalLabel,
    addr: TranspiledAddress,
    global_addresses: &mut GlobalLabelAddrMap,
) -> Result<(), TranspilerError> {
    debug!("Resolved label {label:?} to address {:#x}", addr.as_u32());
    if global_addresses.insert(label, addr).is_some() {
        return Err(TranspilerError::duplicate_global_label(label));
    }
    Ok(())
}

fn resolve_and_remove_global_addresses<TO: TranspiledOp>(
    combined_code: &mut Vec<UnresOp<TO>>,
    n_jump_table_entries: u32,
    max_readable_addr: RiscvAddress,
    mode: &TranspilationMode,
) -> Result<GlobalLabelAddrMap, TranspilerError> {
    let mut cur_transpiled_addr = TranspiledAddress::new(0);
    let mut global_addresses = GlobalLabelAddrMap::new();
    for op in combined_code.iter() {
        match op {
            UnresOp::Label(SymbolicLabel::Global(label)) => {
                update_global_addresses(*label, cur_transpiled_addr, &mut global_addresses)?;
            }
            UnresOp::Label(_) => {}
            UnresOp::Symbolic(sym_op) => {
                cur_transpiled_addr = cur_transpiled_addr.checked_add(sym_op.size_in_bytes())?;
            }
            UnresOp::Native(nat_op) => {
                cur_transpiled_addr = cur_transpiled_addr.checked_add(nat_op.size_in_bytes())?;
            }
        }
    }
    combined_code.retain(|op| {
        if let UnresOp::Label(SymbolicLabel::Global(label)) = op {
            matches!(
                label,
                GlobalLabel::Code(GlobalCodeLabel::StartTranspiledCode)
                    | GlobalLabel::Data(GlobalDataLabel::EntryPointsHooksBegin)
                    | GlobalLabel::Data(GlobalDataLabel::InitArrayEnd)
                    | GlobalLabel::Data(GlobalDataLabel::EntryPointsHooksEnd)
            )
        } else {
            true
        }
    });
    let code_end_addr = checked_align_segment(cur_transpiled_addr.as_u32(), "code end")?;
    if code_end_addr > MAX_CODE_SECTION_SIZE_IN_BYTES {
        return Err(TranspilerError::CodeSizeTooLarge {
            code_size_in_bytes: code_end_addr,
            max_size_in_bytes: MAX_CODE_SECTION_SIZE_IN_BYTES,
        });
    }
    update_global_addresses(
        GlobalDataLabel::CodeSectionEnd.into(),
        TranspiledAddress::new(code_end_addr),
        &mut global_addresses,
    )?;
    // The jump table starts at the segment-aligned end of the code.
    let jump_table_addr = code_end_addr;
    update_global_addresses(
        GlobalDataLabel::JumpTable.into(),
        TranspiledAddress::new(jump_table_addr),
        &mut global_addresses,
    )?;
    // The jump table has one entry per RISC-V instruction plus the exit sentinel. Each entry is
    // 8 bytes in untrusted mode and 4 bytes (offset only) in trusted mode.
    let n_entries_plus_1 = checked_add_u32(n_jump_table_entries, 1, "n_jump_table_entries")?;
    let jump_table_byte_size = checked_mul_u32(
        n_entries_plus_1,
        jump_table_entry_byte_size(mode),
        "jump_table_byte_size",
    )?;
    // `.transpiled.symbols` follows the jump table; the original RISC-V program (guest address
    // 0) starts at the next segment boundary.
    let after_jump_table =
        checked_add_u32(jump_table_addr, jump_table_byte_size, "after_jump_table")?;
    let ro_data_addr = rodata_start_vaddr(after_jump_table)?;
    update_global_addresses(
        GlobalDataLabel::RoDataStart.into(),
        TranspiledAddress::new(ro_data_addr),
        &mut global_addresses,
    )?;
    // `.register_store` starts at guest address `_data_end_value`, right after `.ecall_data`.
    let register_store_addr = checked_add_u32(
        ro_data_addr,
        max_readable_addr.as_u32(),
        "register_store_addr",
    )?;
    if register_store_addr > MAX_TRANSPILED_ADDRESS {
        return Err(TranspilerError::TranspiledAddressTooBig(
            register_store_addr,
        ));
    }
    update_global_addresses(
        GlobalDataLabel::RegisterStore.into(),
        TranspiledAddress::new(register_store_addr),
        &mut global_addresses,
    )?;

    // Make sure all global labels are present in the map
    for label in GlobalCodeLabel::iter() {
        if !global_addresses.contains_key(&label.into()) {
            return Err(TranspilerError::missing_global_label(label));
        }
    }
    for label in GlobalDataLabel::iter() {
        if !global_addresses.contains_key(&label.into()) {
            return Err(TranspilerError::missing_global_label(label));
        }
    }

    Ok(global_addresses)
}

/// Appends one jump-table entry as little-endian `i32`s: the gas cost (untrusted mode only),
/// then the target's offset from the offset field itself. The bytes are the same for every
/// target architecture.
fn push_jump_table_entry(
    out: &mut Vec<u8>,
    jump_table_start_addr: TranspiledAddress,
    index: usize,
    gas_cost: RvGasCost,
    transpiled_addr: TranspiledAddress,
    mode: &TranspilationMode,
) {
    let diff = jump_table_start_addr
        .wrapping_add((index as u32).wrapping_mul(jump_table_entry_byte_size(mode)))
        .wrapping_add(jump_table_offset_position(mode));
    // Untrusted entries lead with the gas cost word; trusted entries are offset-only.
    if !mode.is_trusted() {
        out.extend_from_slice(&(gas_cost.value() as i32).to_le_bytes());
    }
    out.extend_from_slice(
        &transpiled_addr
            .wrapping_sub(diff.as_u32())
            .as_i32()
            .to_le_bytes(),
    );
}

/// Builds the `.jump_table` bytes: one entry per RISC-V instruction, plus a final entry that
/// points at the normal exit.
fn jump_table_to_bytes(
    jump_table: &[JumpTableEntry],
    normal_exit_addr: TranspiledAddress,
    code_section_end_addr: TranspiledAddress,
    mode: &TranspilationMode,
) -> Vec<u8> {
    let jump_table_start_addr = code_section_end_addr;
    let n_entries = jump_table.len().saturating_add(1);
    let mut out = Vec::with_capacity(n_entries * jump_table_entry_byte_size(mode) as usize);
    for (i, jte) in jump_table.iter().enumerate() {
        push_jump_table_entry(
            &mut out,
            jump_table_start_addr,
            i,
            jte.gas_cost(),
            jte.transpiled_addr(),
            mode,
        );
    }
    push_jump_table_entry(
        &mut out,
        jump_table_start_addr,
        jump_table.len(),
        RvGasCost::zero(),
        normal_exit_addr,
        mode,
    );
    out
}

/// Result of [`transpile_common`], before the section bytes are written.
///
/// `resolved_code` holds only [`UnresOp::Native`] and [`UnresOp::Label`] ops;
/// [`resolved_ops_to_bytecode`] encodes it. [`jump_table_to_bytes`] encodes `jump_table`.
struct TranspilationResult<TO: TranspiledOp> {
    resolved_code: Vec<UnresOp<TO>>,
    /// Exact byte length of the transpiled `.text` section, used to pre-size the output buffer.
    code_len_in_bytes: u32,
    n_init_array_functions: usize,
    jump_table: Vec<JumpTableEntry>,
    normal_exit_addr: TranspiledAddress,
    code_section_end_addr: TranspiledAddress,
    global_addresses: GlobalLabelAddrMap,
}

/// Encoded transpiler output, ready for [`build_elf_prog`].
struct ComputedSections {
    /// `.text` bytes.
    text_bytes: Vec<u8>,
    /// Address of every global label, relative to the start of `.text`.
    global_addresses: GlobalLabelAddrMap,
    /// `.jump_table` bytes.
    jump_table_bytes: Vec<u8>,
    /// Number of init functions from `.init_array`.
    n_init_array_functions: usize,
    /// The original RISC-V code, copied to `.original_risc_prog`.
    original_risc_prog: Vec<u8>,
}

// The start and end of the code, and every init function and entry point, start a block.
fn get_initial_risc_block_boundaries(riscv_elf: &RiscvElf) -> HashSet<RiscvAddress> {
    let mut init_risc_block_boundaries: HashSet<RiscvAddress> = HashSet::new();
    init_risc_block_boundaries.insert(RiscvAddress::new(0));
    init_risc_block_boundaries.insert(RiscvAddress::new(riscv_elf.code_section().size_in_bytes()));
    init_risc_block_boundaries.extend(riscv_elf.entry_points().init_array_addrs());
    init_risc_block_boundaries.extend(riscv_elf.entry_points().entry_point_addrs());
    init_risc_block_boundaries
}

/// Transpiles the program, adds the envelope and resolves all labels and branches.
fn transpile_common<TI: TranspilerImplementation>(
    riscv_elf: &RiscvElf,
    mode: &TranspilationMode,
) -> Result<TranspilationResult<TI::OpType>, TranspilerError> {
    let mut risc_block_boundaries = get_initial_risc_block_boundaries(riscv_elf);
    let transpiled_code = transpile_all_instructions::<TI>(
        riscv_elf.symbol_values(),
        &riscv_elf.code_section_as_i32_vec()?,
        &mut risc_block_boundaries,
        mode,
    );
    let transpiled_prog =
        build_transpiled_prog::<TI>(transpiled_code, risc_block_boundaries, mode)?;

    // The envelope and the entry-point hook table follow the program code.
    let mut combined_code = transpiled_prog.ops;
    combined_code.extend(TI::EnvelopeType::exit_user_code());
    combined_code.extend(TI::EnvelopeType::save_local_state());
    combined_code.extend(TI::EnvelopeType::load_global_state_and_ret());
    combined_code.extend(TI::EnvelopeType::ecall_exit());
    combined_code.extend(TI::EnvelopeType::save_global_state());
    combined_code.extend(TI::EnvelopeType::load_local_state());
    let entry_points = riscv_elf.entry_points();
    let n_hooks = checked_usize_to_u32(
        entry_points
            .init_array_addrs()
            .len()
            .saturating_add(entry_points.entry_point_addrs().len()),
        "n_hooks",
    )?;
    combined_code.extend(TI::EnvelopeType::enter_user_code(
        riscv_elf.symbol_values().text_end,
        n_hooks,
        mode.is_trusted(),
    ));
    combined_code.extend(init_array_and_entry_points(riscv_elf.entry_points()));

    // Assign addresses to the global labels before resolving branches to them.
    let global_addresses = resolve_and_remove_global_addresses(
        &mut combined_code,
        transpiled_prog.jump_table.len() as u32,
        riscv_elf.symbol_values().data_end,
        mode,
    )?;

    // Convert all branch targets to pc-relative in place. The returned address is the exact
    // byte length of the transpiled code section.
    let mut resolved_code = combined_code;
    let code_len_in_bytes = TI::EnvelopeType::resolve_in_place(
        &mut resolved_code,
        &TranspiledAddressResolver::new(&global_addresses, &transpiled_prog.jump_table),
    )?
    .as_u32();

    let normal_exit_addr = *global_addresses
        .get(&GlobalCodeLabel::ExitNormal.into())
        .ok_or_else(|| TranspilerError::missing_global_label(GlobalCodeLabel::ExitNormal))?;
    let code_section_end_addr = *global_addresses
        .get(&GlobalDataLabel::CodeSectionEnd.into())
        .ok_or_else(|| TranspilerError::missing_global_label(GlobalDataLabel::CodeSectionEnd))?;

    Ok(TranspilationResult {
        resolved_code,
        code_len_in_bytes,
        n_init_array_functions: riscv_elf.entry_points().init_array_addrs().len(),
        jump_table: transpiled_prog.jump_table,
        normal_exit_addr,
        code_section_end_addr,
        global_addresses,
    })
}

pub(crate) fn transpile_rv_prog_to_elf<TI: TranspilerImplementation>(
    riscv_elf: &RiscvElf,
    mode: &TranspilationMode,
) -> Result<Vec<u8>, TranspilerError> {
    let result = transpile_common::<TI>(riscv_elf, mode)?;
    // Both buffers are pre-sized from the exact lengths computed during resolution.
    let text_bytes =
        resolved_ops_to_bytecode::<TI::OpType>(&result.resolved_code, result.code_len_in_bytes)?;
    let jump_table_bytes = jump_table_to_bytes(
        &result.jump_table,
        result.normal_exit_addr,
        result.code_section_end_addr,
        mode,
    );
    let sections = ComputedSections {
        text_bytes,
        global_addresses: result.global_addresses,
        jump_table_bytes,
        n_init_array_functions: result.n_init_array_functions,
        original_risc_prog: riscv_elf.code_section().data().to_vec(),
    };
    build_elf_prog::<TI>(riscv_elf, sections, mode)
}

/// Collects the output sections and writes the native ELF with [`build_transpiled_elf`].
fn build_elf_prog<TI: TranspilerImplementation>(
    riscv_elf: &RiscvElf,
    sections: ComputedSections,
    mode: &TranspilationMode,
) -> Result<Vec<u8>, TranspilerError> {
    let start_transpiled_code_addr = sections
        .global_addresses
        .get(&GlobalCodeLabel::StartTranspiledCode.into())
        .ok_or_else(|| {
            TranspilerError::missing_global_label(GlobalCodeLabel::StartTranspiledCode)
        })?;
    let entry_points_hooks_begin_addr = sections
        .global_addresses
        .get(&GlobalDataLabel::EntryPointsHooksBegin.into())
        .ok_or_else(|| {
            TranspilerError::missing_global_label(GlobalDataLabel::EntryPointsHooksBegin)
        })?;
    let entry_points_hooks_end_addr = sections
        .global_addresses
        .get(&GlobalDataLabel::EntryPointsHooksEnd.into())
        .ok_or_else(|| {
            TranspilerError::missing_global_label(GlobalDataLabel::EntryPointsHooksEnd)
        })?;

    // Pick out `.rodata`, `.data` and `.bss`. Each is copied whole to its own output section, so
    // an input with more than one section of a kind (e.g. a `.sdata` the linker script did not
    // merge into `.data`) is rejected rather than having its sections run together.
    let mut rodata: Option<&RvElfSection> = None;
    let mut data: Option<&RvElfSection> = None;
    let mut bss: Option<&RvElfSection> = None;
    for section in riscv_elf.data_sections() {
        // `.ecall_data`, `.stack` and `.heap` are built separately, in `RuntimeSections`.
        if matches!(
            section.name(),
            ECALL_DATA_SECTION_NAME | STACK_SECTION_NAME | HEAP_SECTION_NAME
        ) {
            continue;
        }
        let slot = match section.kind() {
            SectionKind::ReadOnlyData | SectionKind::ReadOnlyString => &mut rodata,
            SectionKind::Data => &mut data,
            SectionKind::UninitializedData => &mut bss,
            _ => continue,
        };
        if section.size_in_bytes() == 0 {
            continue;
        }
        if let Some(first) = slot {
            return Err(TranspilerError::SectionsNotMerged {
                first: first.name().to_string(),
                second: section.name().to_string(),
            });
        }
        *slot = Some(section);
    }
    let aligned_bytes = |picked: Option<&RvElfSection>| match picked {
        Some(section) => AlignedBytes {
            bytes: section.data().to_vec(),
            align: section.alignment(),
            guest_addr: section.address().as_u32(),
        },
        None => AlignedBytes {
            bytes: Vec::new(),
            align: 1,
            guest_addr: 0,
        },
    };
    let rodata_section = aligned_bytes(rodata);
    let data_section = aligned_bytes(data);
    let bss_section = match bss {
        Some(section) => NoBitsSection {
            size: section.size_in_bytes(),
            align: section.alignment(),
            guest_addr: section.address().as_u32(),
        },
        None => NoBitsSection {
            size: 0,
            align: 1,
            guest_addr: 0,
        },
    };

    // The reader limits init functions to `MAX_NUM_OF_INIT_FUNCTIONS` (10), so the cast to `u32`
    // is lossless.
    let n_init_array_functions = sections.n_init_array_functions as u32;
    let elf_sections = OutputElfSections {
        text: TextSection {
            bytes: sections.text_bytes,
            start_transpiled_code_addr: *start_transpiled_code_addr,
            hooks_begin_addr: *entry_points_hooks_begin_addr,
            hooks_end_addr: *entry_points_hooks_end_addr,
            n_init_array_functions,
        },
        jump_table: sections.jump_table_bytes,
        original_risc_prog: sections.original_risc_prog,
        rodata: rodata_section,
        data: data_section,
        bss: bss_section,
        runtime: RuntimeSections {
            ecall_data_size: ECALL_DATA_SIZE_BYTES,
            stack_size: riscv_elf.stack_size_in_bytes(),
            heap_size: riscv_elf.heap_size_in_bytes(),
            register_store_size: REGISTER_STORE_SIZE_BYTES,
        },
        risc_symbols: riscv_elf.symbol_values().clone(),
    };

    let branch_byte_size = TI::OpType::branch_size_in_bytes();
    let arch_indicator = TI::EnvelopeType::elf_arch_indicator();
    Ok(build_transpiled_elf(
        elf_sections,
        branch_byte_size,
        arch_indicator,
        SEGMENT_BYTE_ALIGN,
        // Mark trusted output in the ELF header, so the loader can refuse it in an untrusted
        // context.
        to_e_flags(mode),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch64::envelope::Arm64Envelope;
    use crate::arch64::ops::{Arm64NativeOp, Arm64SymbolicOp};
    use crate::arch64::{Arm64Transpiler, ops::Arm64Op};
    use crate::traits::TranspiledEnvelope;
    use crate::x86_64::envelope::X86Envelope;
    use riscv32::immediate::{ImmU12_0, Immediate as _};

    // 0xFFFF_FFFF: opcode bits [6:0] = 0b1111111, a reserved RISC-V opcode the decoder
    // rejects. Used to exercise the illegal-instruction handling path end-to-end.
    const ILLEGAL_INSTRUCTION_WORD: i32 = -1_i32;

    fn dummy_symbol_values(code_size_in_bytes: u32) -> RvSymbolValues {
        RvSymbolValues {
            text_end: RiscvAddress::new(code_size_in_bytes),
            data_start: RiscvAddress::new(0x1000),
            stack_top: RiscvAddress::new(0x3000),
            data_end: RiscvAddress::new(0x3000),
        }
    }

    // Verifies the envelope invariant that makes RISC-V code without a terminating
    // `ret` (jalr) stumble into the illegal-opcode handler: the first item appended
    // after the user code is the `ExitIllegalOpcode` label. The previous transpiler
    // emitted an explicit `branch_to_illegal_code()` at the end of user code; that
    // was removed once `exit_user_code()` placed `ExitIllegalOpcode` first so the
    // natural PC fall-through reaches it.
    #[test]
    fn exit_user_code_places_illegal_opcode_label_first_for_natural_fall_through() {
        let arm_ops = Arm64Envelope::exit_user_code();
        assert!(
            matches!(
                arm_ops.first(),
                Some(UnresOp::Label(SymbolicLabel::Global(GlobalLabel::Code(
                    GlobalCodeLabel::ExitIllegalOpcode,
                )))),
            ),
            "arm64 envelope: fall-through must land on ExitIllegalOpcode (first item of exit_user_code)",
        );

        let x86_ops = X86Envelope::exit_user_code();
        assert!(
            matches!(
                x86_ops.first(),
                Some(UnresOp::Label(SymbolicLabel::Global(GlobalLabel::Code(
                    GlobalCodeLabel::ExitIllegalOpcode,
                )))),
            ),
            "x86_64 envelope: fall-through must land on ExitIllegalOpcode (first item of exit_user_code)",
        );
    }

    #[test]
    fn transpile_all_instructions_emits_branch_to_illegal_code_for_undecodable_instruction() {
        let symbol_values = dummy_symbol_values(N_BYTES_IN_WORD as u32);
        let risc_prog = [ILLEGAL_INSTRUCTION_WORD];
        let mut risc_block_boundaries: HashSet<RiscvAddress> = HashSet::new();

        let transpiled_code = transpile_all_instructions::<Arm64Transpiler>(
            &symbol_values,
            &risc_prog,
            &mut risc_block_boundaries,
            &TranspilationMode::default(),
        );

        // The illegal instruction is replaced by a single branch to the ExitIllegalOpcode
        // label (the envelope's `branch_to_illegal_code()` output) with zero gas cost.
        assert_eq!(transpiled_code.len(), 1);
        assert_eq!(transpiled_code[0].gas_cost.value(), 0);
        assert!(
            matches!(
                transpiled_code[0].ops.as_slice(),
                [UnresOp::Symbolic(Arm64SymbolicOp::B(
                    SymbolicBranchTarget::Label(GlobalCodeLabel::ExitIllegalOpcode,)
                ))],
            ),
            "expected single B(ExitIllegalOpcode) for an illegal instruction",
        );
    }

    fn nop_block(cost: u16) -> TranspiledCodeBlock<Arm64Op> {
        TranspiledCodeBlock {
            ops: vec![Arm64NativeOp::Nop.into()],
            gas_cost: RvGasCost::new(ImmU12_0(cost)),
        }
    }

    fn single_natural_chunk_boundaries(n_risc_instructions: u32) -> HashSet<RiscvAddress> {
        [0, n_risc_instructions * N_BYTES_IN_WORD as u32]
            .into_iter()
            .map(RiscvAddress::new)
            .collect()
    }

    #[test]
    fn build_transpiled_prog_splits_block_when_gas_would_overflow() {
        // Three instructions each costing 2000 gas sum to 6000, which exceeds ImmU12_0's max
        // (4095), the most one block can charge. The block must split into [inst0, inst1]
        // (4000) and [inst2] (2000).
        let transpiled_code = vec![nop_block(2000), nop_block(2000), nop_block(2000)];
        let risc_targets = single_natural_chunk_boundaries(3);

        let transpiled_prog = build_transpiled_prog::<Arm64Transpiler>(
            transpiled_code,
            risc_targets,
            &TranspilationMode::default(),
        )
        .expect("limiter should split the block instead of overflowing gas");

        // The first instruction of every pseudo basic block has its gas_cost
        // replaced with 0 (the original block-total moves into the trampoline
        // ops). Remaining instructions carry their tail-cumulative cost.
        assert_eq!(transpiled_prog.jump_table.len(), 3);
        assert_eq!(transpiled_prog.jump_table[0].gas_cost().value(), 0);
        assert_eq!(transpiled_prog.jump_table[1].gas_cost().value(), 2000);
        assert_eq!(transpiled_prog.jump_table[2].gas_cost().value(), 0);
    }

    #[test]
    fn build_transpiled_prog_keeps_block_when_under_gas_limit() {
        // Three instructions costing 1000 each total 3000, below ImmU12_0::MAX_VAL.
        // The limiter must keep them as a single pseudo basic block.
        let transpiled_code = vec![nop_block(1000), nop_block(1000), nop_block(1000)];
        let risc_targets = single_natural_chunk_boundaries(3);

        let transpiled_prog = build_transpiled_prog::<Arm64Transpiler>(
            transpiled_code,
            risc_targets,
            &TranspilationMode::default(),
        )
        .expect("under-limit blocks should not error");

        // One block: the costs from each instruction to the end are [3000, 2000, 1000], and
        // the first instruction's entry holds 0 because the block's own charge covers it.
        assert_eq!(transpiled_prog.jump_table.len(), 3);
        assert_eq!(transpiled_prog.jump_table[0].gas_cost().value(), 0);
        assert_eq!(transpiled_prog.jump_table[1].gas_cost().value(), 2000);
        assert_eq!(transpiled_prog.jump_table[2].gas_cost().value(), 1000);
    }

    #[test]
    fn build_transpiled_prog_splits_at_max_boundary() {
        // Two instructions of cost ImmU12_0::MAX_VAL/2 + 1 = 2048 each: their sum
        // (4096) exceeds ImmU12_0::MAX_VAL by 1, so the limiter must split them apart
        // into two single-instruction pseudo basic blocks.
        let half_plus_one = ImmU12_0::MAX_VAL / 2 + 1;
        let transpiled_code = vec![nop_block(half_plus_one), nop_block(half_plus_one)];
        let risc_targets = single_natural_chunk_boundaries(2);

        let transpiled_prog = build_transpiled_prog::<Arm64Transpiler>(
            transpiled_code,
            risc_targets,
            &TranspilationMode::default(),
        )
        .expect("split at exact overflow point should succeed");

        // Each instruction is now the sole member of its own pseudo basic block,
        // so each gets its gas_cost reset to 0 in the emit loop.
        assert_eq!(transpiled_prog.jump_table.len(), 2);
        assert_eq!(transpiled_prog.jump_table[0].gas_cost().value(), 0);
        assert_eq!(transpiled_prog.jump_table[1].gas_cost().value(), 0);
    }

    #[test]
    fn build_long_pseudo_basic_block() {
        let n_ops = ImmU12_0::MAX_VAL as usize;
        let transpiled_code = vec![nop_block(1); n_ops];
        let risc_targets = single_natural_chunk_boundaries(n_ops as u32);

        let transpiled_prog = build_transpiled_prog::<Arm64Transpiler>(
            transpiled_code,
            risc_targets,
            &TranspilationMode::default(),
        )
        .expect("should succeed");

        // The entire block is kept as one pseudo basic block, so its gas_cost is reset to 0.
        assert_eq!(transpiled_prog.jump_table.len(), n_ops);
        assert_eq!(transpiled_prog.jump_table[0].gas_cost().value(), 0);
        let mut expected_gas = ImmU12_0::MAX_VAL - 1;
        let jt_iter = transpiled_prog.jump_table.iter().skip(1);
        for entry in jt_iter {
            assert_eq!(entry.gas_cost().value(), expected_gas);
            expected_gas -= 1;
        }
    }

    #[test]
    fn build_split_long_pseudo_basic_blocks() {
        let n_ops = (ImmU12_0::MAX_VAL as usize * 3) + 10;
        let transpiled_code = vec![nop_block(1); n_ops];
        let risc_targets = single_natural_chunk_boundaries(n_ops as u32);

        let transpiled_prog = build_transpiled_prog::<Arm64Transpiler>(
            transpiled_code,
            risc_targets,
            &TranspilationMode::default(),
        )
        .expect("should succeed");

        assert_eq!(transpiled_prog.jump_table.len(), n_ops);
        // The natural chunk is split into three sub-chunks of ImmU12_0::MAX_VAL instructions
        // (cost-1 instructions saturate the per-block ImmU12_0 budget at ImmU12_0::MAX_VAL) plus
        // a remainder of 10 instructions. Each sub-chunk's first jt entry is reset
        // to 0; the rest carry tail-cumulative costs counting down from len - 1.
        let chunk_size = ImmU12_0::MAX_VAL as usize;
        let mut base = 0;
        for _ in 0..3 {
            assert_eq!(transpiled_prog.jump_table[base].gas_cost().value(), 0);
            for offset in 1..chunk_size {
                let expected = (chunk_size - offset) as u16;
                assert_eq!(
                    transpiled_prog.jump_table[base + offset].gas_cost().value(),
                    expected
                );
            }
            base += chunk_size;
        }
        // Remainder sub-chunk of 10 instructions: [0, 9, 8, ..., 1].
        assert_eq!(transpiled_prog.jump_table[base].gas_cost().value(), 0);
        for offset in 1..10 {
            let expected = (10 - offset) as u16;
            assert_eq!(
                transpiled_prog.jump_table[base + offset].gas_cost().value(),
                expected
            );
        }
    }

    fn boundaries(instruction_indices: &[u32]) -> HashSet<RiscvAddress> {
        instruction_indices
            .iter()
            .map(|&i| RiscvAddress::new(i * N_BYTES_IN_WORD as u32))
            .collect()
    }

    #[test]
    fn build_transpiled_prog_splits_at_every_boundary() {
        // Blocks [0, 2) and [2, 5): each block's first entry is 0, the rest count down.
        let transpiled_code = vec![nop_block(1); 5];
        let transpiled_prog = build_transpiled_prog::<Arm64Transpiler>(
            transpiled_code,
            boundaries(&[0, 2, 5]),
            &TranspilationMode::default(),
        )
        .expect("should succeed");
        let costs: Vec<u16> = transpiled_prog
            .jump_table
            .iter()
            .map(|entry| entry.gas_cost().value())
            .collect();
        assert_eq!(costs, [0, 1, 0, 2, 1]);
    }

    /// Splitting must stay linear in the number of blocks: a quadratic split of this input
    /// took seconds, and an input at the reader's size limit would take about an hour.
    #[test]
    fn build_transpiled_prog_handles_many_blocks() {
        let n_ops = 200_000_u32;
        let transpiled_code = vec![nop_block(1); n_ops as usize];
        let all_boundaries: Vec<u32> = (0..=n_ops).collect();
        let transpiled_prog = build_transpiled_prog::<Arm64Transpiler>(
            transpiled_code,
            boundaries(&all_boundaries),
            &TranspilationMode::default(),
        )
        .expect("should succeed");
        assert_eq!(transpiled_prog.jump_table.len(), n_ops as usize);
        assert!(
            transpiled_prog
                .jump_table
                .iter()
                .all(|entry| entry.gas_cost().value() == 0)
        );
    }

    #[test]
    fn instruction_costing_more_than_a_block_is_rejected() {
        let too_large = ImmU12_0::MAX_VAL + 1;
        let result = build_transpiled_prog::<Arm64Transpiler>(
            vec![nop_block(1), nop_block(too_large)],
            single_natural_chunk_boundaries(2),
            &TranspilationMode::default(),
        );
        assert!(
            matches!(
                result,
                Err(TranspilerError::GasCostTooLarge { cost, max })
                    if cost == too_large && max == ImmU12_0::MAX_VAL
            ),
            "expected GasCostTooLarge"
        );
    }

    #[test]
    fn block_boundary_beyond_the_code_is_rejected() {
        let result = build_transpiled_prog::<Arm64Transpiler>(
            vec![nop_block(1); 2],
            boundaries(&[0, 2, 0x400]),
            &TranspilationMode::default(),
        );
        assert!(
            matches!(
                result,
                Err(TranspilerError::BlockBoundaryBeyondCode {
                    boundary: 0x400,
                    n_instructions: 2,
                })
            ),
            "expected BlockBoundaryBeyondCode"
        );
    }
}
