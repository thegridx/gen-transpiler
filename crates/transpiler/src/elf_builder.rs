//! Writes the native ELF64 file with the `object` crate's writer, so no external linker is needed.
//!
//! The output is simple: no relocations, no dynamic-linker tables, and a fixed layout of four loadable
//! segments that matches `riscv_elf_mem_layout.x` (see "Output layout" in the crate README).
use object::Endianness;
use object::elf;
use object::write::StringId;
use object::write::elf::{FileHeader, ProgramHeader, SectionHeader, Writer};
use riscv32::common::ValueError;
use riscv32::common::{
    checked_add_u32, checked_align_segment, checked_align_to, checked_mul_u32, checked_sub_u32,
    checked_usize_to_u32,
};
use riscv32_elf::consts::{ORIGINAL_RISC_PROG_SECTION_NAME, TranspiledSymbolsSection};
use riscv32_elf::types::RvSymbolValues;
use std::collections::HashMap;
use thiserror::Error;

use crate::layout::{LayoutValidationError, SectionId, validate_layout};
use crate::transpiled_address::{TranspiledAddress, TranspiledAddressAndSize};

/// Size of the `.transpiled.symbols` section (8 `u32`s, 32 bytes).
const TRANSPILED_SYMBOLS_SIZE_IN_BYTES: u32 = TranspiledSymbolsSection::SIZE_IN_BYTES as u32;

const JUMP_TABLE_POST_ALIGN: u32 = 8;
const POINTER_ALIGN: u32 = 8;
const ECALL_DATA_SECTION_ALIGN: u32 = 16;
const STACK_SECTION_ALIGN: u32 = 16;
const HEAP_SECTION_ALIGN: u32 = 16;
const REGISTER_STORE_ALIGN: u32 = 8;

/// The output ELF could not be built.
#[derive(Error, Debug)]
pub enum ElfBuilderError {
    #[error("Layout validation error: {0}")]
    LayoutValidation(#[from] LayoutValidationError),
    #[error("Value error: {0}")]
    ValueError(#[from] ValueError),
    #[error("Writer error: {0}")]
    WriterError(#[from] object::write::Error),
    #[error("Missing section: {0}")]
    MissingSection(String),
    #[error("Anonymous NOBITS section")]
    AnonymousNoBitsSection,
    #[error(
        "stack segment is not contiguous with the data segment: stack ends at {stack_end:#x}, data starts at {data_start:#x} (stack size must be a multiple of SEGMENT_BYTE_ALIGN)"
    )]
    NonContiguousStackSegment { stack_end: u32, data_start: u32 },
    #[error("rodata start mismatch: transpiler baked {baked:#x}, layout placed {laid_out:#x}")]
    RodataStartMismatch { baked: u32, laid_out: u32 },
    #[error(
        "{section} is at guest address {input:#x} in the input but would be at {output:#x} in the output; link with riscv_elf_mem_layout.x"
    )]
    GuestAddressMismatch {
        section: String,
        input: u32,
        output: u32,
    },
}

/// Returns the address of `.original_risc_prog` (guest address 0), which the transpiler
/// builds into the code as `RoDataStart`.
///
/// It follows the order of the read-only segment in `build_segment_specs`: `.jump_table`
/// padded to [`JUMP_TABLE_POST_ALIGN`], then `.transpiled.symbols`, then segment alignment.
/// [`build_transpiled_elf`] rejects a layout that disagrees with this value.
pub(crate) fn rodata_start_vaddr(jump_table_end: u32) -> Result<u32, ValueError> {
    let symbols_start = checked_align_to(
        jump_table_end,
        JUMP_TABLE_POST_ALIGN,
        "jump_table post-align",
    )?;
    let symbols_end = checked_add_u32(
        symbols_start,
        TRANSPILED_SYMBOLS_SIZE_IN_BYTES,
        "transpiled_symbols end",
    )?;
    checked_align_segment(symbols_end, "ro_data_addr")
}

// ============================================================================
// Input structure
// ============================================================================

/// `.text` and the offsets of the labels in it that `.transpiled.symbols` records.
pub(crate) struct TextSection {
    /// `.text` bytes, including the entry-point hook table.
    pub(crate) bytes: Vec<u8>,
    /// Offset of `_start_transpiled_code` in `.text`.
    pub(crate) start_transpiled_code_addr: TranspiledAddress,
    /// Offset of the start of the entry-point hook table in `.text`.
    pub(crate) hooks_begin_addr: TranspiledAddress,
    /// Offset of the end of the entry-point hook table in `.text`.
    pub(crate) hooks_end_addr: TranspiledAddress,
    /// Number of init functions, which sets where `__init_array_end` falls in the hook table.
    pub(crate) n_init_array_functions: u32,
}

/// PROGBITS section with an explicit alignment.
pub(crate) struct AlignedBytes {
    pub(crate) bytes: Vec<u8>,
    pub(crate) align: u32,
    /// The section's address in the input RISC-V ELF, where the guest code expects it.
    pub(crate) guest_addr: u32,
}

/// NOBITS section: size + alignment, no file bytes.
pub(crate) struct NoBitsSection {
    pub(crate) size: u32,
    pub(crate) align: u32,
    /// The section's address in the input RISC-V ELF, where the guest code expects it.
    pub(crate) guest_addr: u32,
}

/// Sizes of the runtime sections, which are all NOBITS.
pub(crate) struct RuntimeSections {
    pub(crate) ecall_data_size: u32,
    pub(crate) stack_size: u32,
    pub(crate) heap_size: u32,
    pub(crate) register_store_size: u32,
}

/// Input sections for building an output ELF file.
pub(crate) struct OutputElfSections {
    pub(crate) text: TextSection,
    /// `.jump_table` bytes.
    pub(crate) jump_table: Vec<u8>,
    /// The original RISC-V code.
    pub(crate) original_risc_prog: Vec<u8>,
    /// The input's `.rodata`.
    pub(crate) rodata: AlignedBytes,
    /// The input's `.data`.
    pub(crate) data: AlignedBytes,
    /// The input's `.bss`.
    pub(crate) bss: NoBitsSection,
    /// `.stack`, `.heap`, `.ecall_data` and `.register_store`.
    pub(crate) runtime: RuntimeSections,
    /// The input's linker-script symbol values.
    pub(crate) risc_symbols: RvSymbolValues,
}

// ============================================================================
// Segment / section layout description
// ============================================================================

/// Declarative description of a single section's layout properties.
struct SectionSpec {
    id: SectionId,
    size: u32,
    /// Alignment of the section's start (1 for none).
    pre_align: u32,
    /// If non-zero, the space the section takes is its size rounded up to this alignment.
    post_align: u32,
    /// Omit the section when it is empty.
    optional: bool,
    /// The section has file bytes and counts toward the segment's `p_filesz`.
    progbits: bool,
    /// Place the section at this offset from `.original_risc_prog`, i.e. at this guest address,
    /// leaving a gap after the previous section. Applied after `pre_align`.
    guest_offset: Option<u32>,
}

impl SectionSpec {
    fn new(id: SectionId, size: u32) -> Self {
        Self {
            id,
            size,
            pre_align: 1,
            post_align: 0,
            optional: false,
            progbits: true,
            guest_offset: None,
        }
    }
    fn at_guest_offset(mut self, offset: u32) -> Self {
        self.guest_offset = Some(offset);
        self
    }
    fn pre_align(mut self, align: u32) -> Self {
        self.pre_align = align;
        self
    }
    fn post_align(mut self, align: u32) -> Self {
        self.post_align = align;
        self
    }
    fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    fn nobits(mut self) -> Self {
        self.progbits = false;
        self
    }
}

/// An entry in a segment's layout specification.
enum SegEntry {
    /// A regular section.
    Section(SectionSpec),
    /// Align the current address to `SEGMENT_BYTE_ALIGN`.
    SegmentAlign,
}

/// Flags of the four `PT_LOAD` segments, each aligned to `SEGMENT_BYTE_ALIGN`:
/// 0: text (R+X); 1: read-only data (R); 2: stack (R+W, NOBITS only); 3: data (R+W).
///
/// `.stack` has its own segment right below the data segment, so it is the lowest writable
/// region: a stack that grows past `_stack_bottom` faults against the read-only segment instead
/// of corrupting `.data` or `.bss`. The stack segment has no file bytes, and the file offsets
/// above it are shifted down (see [`ComputedLayout::file_offset_of`]), so it takes no space in
/// the file.
const SEGMENT_FLAGS: [u32; 4] = [
    elf::PF_R | elf::PF_X,
    elf::PF_R,
    elf::PF_R | elf::PF_W,
    elf::PF_R | elf::PF_W,
];
const N_SEGMENTS: usize = SEGMENT_FLAGS.len();
/// Segment index of the NOBITS-only stack segment.
const STACK_SEGMENT_INDEX: usize = 2;
/// Segment index of the writable data segment (`.data`, `.bss`, `.heap`, `.ecall_data`,
/// `.register_store`).
const DATA_SEGMENT_INDEX: usize = 3;

/// Section placements, indexed by `SectionId`, and segment bounds.
struct ComputedLayout {
    sections: [Option<TranspiledAddressAndSize>; SectionId::COUNT],
    segments: [TranspiledAddressAndSize; N_SEGMENTS],
    /// `p_filesz` of each segment: up to the end of its last PROGBITS section, without any
    /// trailing padding, which is not written to the file.
    seg_filesz: [u32; N_SEGMENTS],
    seg_align: u32,
    /// Start of the data segment. File offsets of addresses at or above it are shifted down by
    /// [`Self::file_hole`] bytes.
    file_hole_vaddr: u32,
    /// Size of the stack segment, which takes address space but no file bytes.
    file_hole: u32,
}

impl ComputedLayout {
    /// Returns a mandatory section's placement.
    fn require(&self, id: SectionId) -> Result<TranspiledAddressAndSize, ElfBuilderError> {
        self.sections[id as usize]
            .ok_or_else(|| ElfBuilderError::MissingSection(id.name().to_string()))
    }

    /// Returns an optional section's placement.
    fn maybe(&self, id: SectionId) -> Option<TranspiledAddressAndSize> {
        self.sections[id as usize]
    }

    /// Returns the file offset of a virtual address.
    ///
    /// The file mirrors the address space, offset by `seg_align`, except that the stack segment
    /// has no file bytes: addresses at or above the data segment are shifted down by
    /// `file_hole`. The data segment start and the stack size are both multiples of
    /// `seg_align`, so `p_offset ≡ p_vaddr (mod p_align)` still holds for every `PT_LOAD`.
    fn file_offset_of(
        &self,
        vaddr: TranspiledAddress,
        context: &str,
    ) -> Result<u32, ElfBuilderError> {
        let raw = checked_add_u32(self.seg_align, vaddr.as_u32(), context)?;
        if vaddr.as_u32() >= self.file_hole_vaddr {
            Ok(checked_sub_u32(raw, self.file_hole, context)?)
        } else {
            Ok(raw)
        }
    }
}

/// Describes the sections of each segment, in address order.
fn build_segment_specs(
    sections: &OutputElfSections,
) -> Result<[Vec<SegEntry>; N_SEGMENTS], ElfBuilderError> {
    // Segment 0: code (R+X), only `.text`. `.jump_table` starts the next segment.
    let text_segment = vec![SegEntry::Section(SectionSpec::new(
        SectionId::Text,
        checked_usize_to_u32(sections.text.bytes.len(), "text")?,
    ))];

    // Segment 1: read-only data (R).
    //
    // `.original_risc_prog` must start at `rodata_start_vaddr`, the `RoDataStart` address
    // that `resolve_and_remove_global_addresses` builds into the code. Keep this order in sync
    // with `rodata_start_vaddr`.
    let rodata_segment = vec![
        SegEntry::Section(
            SectionSpec::new(
                SectionId::JumpTable,
                checked_usize_to_u32(sections.jump_table.len(), "jump_table")?,
            )
            .post_align(JUMP_TABLE_POST_ALIGN),
        ),
        SegEntry::Section(SectionSpec::new(
            SectionId::TranspiledSymbols,
            TRANSPILED_SYMBOLS_SIZE_IN_BYTES,
        )),
        SegEntry::SegmentAlign,
        SegEntry::Section(
            SectionSpec::new(
                SectionId::OriginalRiscProg,
                checked_usize_to_u32(sections.original_risc_prog.len(), "original_risc_prog")?,
            )
            .post_align(4),
        ),
        SegEntry::SegmentAlign,
        // `.rodata` keeps its input guest address. The linker script puts it on the first
        // segment boundary after `.text` and `.init_array`; the output has no `.init_array`, so
        // the boundary after `.original_risc_prog` alone can be too low.
        SegEntry::Section(
            SectionSpec::new(
                SectionId::Rodata,
                checked_usize_to_u32(sections.rodata.bytes.len(), "rodata")?,
            )
            .at_guest_offset(sections.rodata.guest_addr)
            .post_align(4)
            .optional(),
        ),
        // Extend `p_memsz` up to the stack segment. Guest loads may read anywhere from
        // `.original_risc_prog` to the end of `.ecall_data`, so the segments must leave no
        // unmapped gap: on a host whose page size is smaller than `SEGMENT_BYTE_ALIGN`, a
        // bounds-checked guest load into such a gap would fault the host process.
        SegEntry::SegmentAlign,
    ];

    // Segment 2: stack (R+W, NOBITS only), the lowest writable region. Its size is a multiple
    // of `SEGMENT_BYTE_ALIGN`, so the data segment starts right at its end, with no gap.
    let stack_segment = vec![SegEntry::Section(
        SectionSpec::new(SectionId::Stack, sections.runtime.stack_size)
            .pre_align(STACK_SECTION_ALIGN)
            .nobits(),
    )];

    // Segment 3: writable data and runtime sections (R+W).
    let data_segment = vec![
        SegEntry::Section(
            SectionSpec::new(
                SectionId::Data,
                checked_usize_to_u32(sections.data.bytes.len(), "data")?,
            )
            .optional(),
        ),
        SegEntry::Section(
            SectionSpec::new(SectionId::Bss, sections.bss.size)
                .pre_align(sections.bss.align.max(1))
                .optional()
                .nobits(),
        ),
        SegEntry::Section(
            SectionSpec::new(SectionId::Heap, sections.runtime.heap_size)
                .pre_align(HEAP_SECTION_ALIGN)
                .nobits(),
        ),
        SegEntry::Section(
            SectionSpec::new(SectionId::EcallData, sections.runtime.ecall_data_size)
                .pre_align(ECALL_DATA_SECTION_ALIGN)
                .nobits(),
        ),
        SegEntry::Section(
            SectionSpec::new(
                SectionId::RegisterStore,
                sections.runtime.register_store_size,
            )
            .post_align(POINTER_ALIGN) // No effect: the size, 104 bytes, is already aligned.
            .nobits(),
        ),
    ];

    Ok([text_segment, rodata_segment, stack_segment, data_segment])
}

/// Computes the address of every section, walking the segments in order with a current
/// address, like a linker script's `.`.
fn compute_layout(
    sections: &OutputElfSections,
    seg_align: u32,
) -> Result<ComputedLayout, ElfBuilderError> {
    let segment_specs = build_segment_specs(sections)?;
    let mut computed_sections: [Option<TranspiledAddressAndSize>; SectionId::COUNT] =
        [None; SectionId::COUNT];
    let mut segments = [TranspiledAddressAndSize {
        addr: TranspiledAddress::new(0),
        size: 0,
    }; N_SEGMENTS];
    let mut seg_filesz: [u32; N_SEGMENTS] = [0; N_SEGMENTS];
    let mut cur: u32 = 0;

    for (seg_idx, segment) in segment_specs.iter().enumerate() {
        if seg_idx > 0 {
            cur = checked_align_segment(cur, "segment align")?;
        }
        let seg_start = cur;

        for entry in segment {
            match entry {
                SegEntry::Section(spec) => {
                    if spec.optional && spec.size == 0 {
                        continue;
                    }
                    if spec.pre_align > 1 {
                        cur = checked_align_to(cur, spec.pre_align, &format!("{:?}", spec.id))?;
                    }
                    if let Some(offset) = spec.guest_offset {
                        let guest_base = computed_sections[SectionId::OriginalRiscProg as usize]
                            .ok_or_else(|| {
                                ElfBuilderError::MissingSection(
                                    SectionId::OriginalRiscProg.name().to_string(),
                                )
                            })?
                            .addr
                            .as_u32();
                        let target = checked_add_u32(guest_base, offset, spec.id.name())?;
                        if target < cur {
                            return Err(ElfBuilderError::GuestAddressMismatch {
                                section: spec.id.name().to_string(),
                                input: offset,
                                output: checked_sub_u32(cur, guest_base, spec.id.name())?,
                            });
                        }
                        cur = target;
                    }
                    let section_addr = cur;
                    computed_sections[spec.id as usize] = Some(TranspiledAddressAndSize {
                        addr: TranspiledAddress::new(section_addr),
                        size: spec.size,
                    });
                    if spec.progbits {
                        let section_end =
                            checked_add_u32(section_addr, spec.size, &format!("{:?}", spec.id))?;
                        seg_filesz[seg_idx] = checked_sub_u32(section_end, seg_start, "filesz")?;
                    }
                    let advance = if spec.post_align > 0 {
                        checked_align_to(spec.size, spec.post_align, &format!("{:?}", spec.id))?
                    } else {
                        spec.size
                    };
                    cur = checked_add_u32(cur, advance, &format!("{:?}", spec.id))?;
                }
                SegEntry::SegmentAlign => {
                    cur = checked_align_segment(cur, "segment align")?;
                }
            }
        }
        let size = checked_sub_u32(cur, seg_start, "segment size")?;
        segments[seg_idx] = TranspiledAddressAndSize {
            addr: TranspiledAddress::new(seg_start),
            size,
        };
    }

    // The stack segment has no file bytes, so file offsets from the data segment on are shifted
    // down by its size. The data segment must start right at the end of the stack, so that the
    // writable memory is one contiguous range; this holds when the stack size is a multiple of
    // `SEGMENT_BYTE_ALIGN`.
    let stack_seg_start = segments[STACK_SEGMENT_INDEX].addr.as_u32();
    let data_seg_start = segments[DATA_SEGMENT_INDEX].addr.as_u32();
    let stack_seg_end = checked_add_u32(
        stack_seg_start,
        segments[STACK_SEGMENT_INDEX].size,
        "stack_seg_end",
    )?;
    if stack_seg_end != data_seg_start {
        return Err(ElfBuilderError::NonContiguousStackSegment {
            stack_end: stack_seg_end,
            data_start: data_seg_start,
        });
    }
    let file_hole = checked_sub_u32(data_seg_start, stack_seg_start, "stack file hole")?;

    Ok(ComputedLayout {
        sections: computed_sections,
        segments,
        seg_filesz,
        seg_align,
        file_hole_vaddr: data_seg_start,
        file_hole,
    })
}

// ============================================================================
// Layout validation
// ============================================================================

/// Runs the loader's layout check, [`validate_layout`], on the computed layout.
fn loader_validate_layout(
    computed_layout: &ComputedLayout,
    sections: &OutputElfSections,
) -> Result<(), ElfBuilderError> {
    let mut section_map = HashMap::new();
    for id in [
        SectionId::OriginalRiscProg,
        SectionId::Data,
        // `.bss` is the first writable section when `.data` is empty, so the stack-top adjacency
        // check needs it in the map to fall through `.data` -> `.bss` -> `.heap`.
        SectionId::Bss,
        SectionId::EcallData,
        SectionId::Stack,
        SectionId::Heap,
        SectionId::RegisterStore,
    ] {
        if let Some(s) = computed_layout.sections[id as usize] {
            section_map.insert(id, s);
        }
    }

    let orig_text_end = TranspiledAddress::new(checked_usize_to_u32(
        sections.original_risc_prog.len(),
        ORIGINAL_RISC_PROG_SECTION_NAME,
    )?);

    validate_layout(
        &section_map,
        orig_text_end,
        TranspiledAddress::new(sections.risc_symbols.data_start.as_u32()),
        TranspiledAddress::new(sections.risc_symbols.stack_top.as_u32()),
        TranspiledAddress::new(sections.risc_symbols.data_end.as_u32()),
    )?;
    Ok(())
}

/// Checks that `.rodata`, `.data` and `.bss` land at the guest addresses the input was linked
/// for. Guest code reaches them through absolute addresses, so a section placed anywhere else
/// would silently read and write the wrong bytes.
fn validate_guest_addresses(
    computed_layout: &ComputedLayout,
    sections: &OutputElfSections,
) -> Result<(), ElfBuilderError> {
    let guest_base = computed_layout.require(SectionId::OriginalRiscProg)?.addr;
    for (id, input) in [
        (SectionId::Rodata, sections.rodata.guest_addr),
        (SectionId::Data, sections.data.guest_addr),
        (SectionId::Bss, sections.bss.guest_addr),
    ] {
        let Some(placed) = computed_layout.maybe(id) else {
            continue; // Absent: empty in the input.
        };
        let output = checked_sub_u32(placed.addr.as_u32(), guest_base.as_u32(), id.name())?;
        if output != input {
            return Err(ElfBuilderError::GuestAddressMismatch {
                section: id.name().to_string(),
                input,
                output,
            });
        }
    }
    Ok(())
}

// ============================================================================
// Symbols
// ============================================================================

/// Builds the `.transpiled.symbols` bytes: an encoded [`TranspiledSymbolsSection`].
fn build_transpiled_symbols_bytes(
    computed_layout: &ComputedLayout,
    sections: &OutputElfSections,
    branch_size_in_bytes: u32,
) -> Result<Vec<u8>, ElfBuilderError> {
    let text = &sections.text;
    let text_vaddr = computed_layout.require(SectionId::Text)?.addr;

    let start_value = text_vaddr.checked_add(text.start_transpiled_code_addr.as_u32())?;
    let hooks_begin_value = text_vaddr.checked_add(text.hooks_begin_addr.as_u32())?;
    let hooks_end_value = text_vaddr.checked_add(text.hooks_end_addr.as_u32())?;

    let init_array_size = checked_mul_u32(
        text.n_init_array_functions,
        branch_size_in_bytes,
        "init_array_size",
    )?;
    let init_array_end_value = hooks_begin_value.checked_add(init_array_size)?;

    let transpiled_symbols = TranspiledSymbolsSection {
        start_transpiled_code: start_value.as_u32(),
        entry_points_hooks_begin: hooks_begin_value.as_u32(),
        init_array_end: init_array_end_value.as_u32(),
        entry_points_hooks_end: hooks_end_value.as_u32(),
        text_end_value: computed_layout.require(SectionId::OriginalRiscProg)?.size,
        data_start_value: sections.risc_symbols.data_start.as_u32(),
        stack_top_value: sections.risc_symbols.stack_top.as_u32(),
        data_end_value: sections.risc_symbols.data_end.as_u32(),
    };
    Ok(transpiled_symbols.to_bytes())
}

// ============================================================================
// Section descriptors
// ============================================================================

/// Describes how a section is reserved, written, and headered.
enum SectionBehavior<'a> {
    /// PROGBITS with explicit bytes to write.
    Progbits {
        bytes: &'a [u8],
        sh_flags: u64,
        align: u32,
        entsize: u32,
    },
    /// NOBITS (no file data, only a header).
    NoBits { align: u32 },
    /// .data: PROGBITS with SHF_WRITE.
    Data { bytes: &'a [u8], align: u32 },
}

/// A section and how to write it.
struct SectionDescriptor<'a> {
    id: SectionId,
    behavior: SectionBehavior<'a>,
}

/// Returns descriptors for all present sections, in address order, which `reserve_until`
/// requires.
fn build_section_descriptors<'a>(
    sections: &'a OutputElfSections,
    computed_layout: &ComputedLayout,
    transpiled_symbols_bytes: &'a [u8],
) -> Vec<SectionDescriptor<'a>> {
    let mut descs: Vec<SectionDescriptor<'a>> = vec![
        // Segment 0: `.text`.
        SectionDescriptor {
            id: SectionId::Text,
            behavior: SectionBehavior::Progbits {
                bytes: &sections.text.bytes,
                sh_flags: (elf::SHF_ALLOC | elf::SHF_EXECINSTR).into(),
                align: 4,
                entsize: 0,
            },
        },
        // Segment 1: `.jump_table`, `.transpiled.symbols`, `.original_risc_prog`, `.rodata`.
        SectionDescriptor {
            id: SectionId::JumpTable,
            behavior: SectionBehavior::Progbits {
                bytes: &sections.jump_table,
                sh_flags: elf::SHF_ALLOC.into(),
                align: 8,
                entsize: 0,
            },
        },
        SectionDescriptor {
            id: SectionId::TranspiledSymbols,
            behavior: SectionBehavior::Progbits {
                bytes: transpiled_symbols_bytes,
                sh_flags: elf::SHF_ALLOC.into(),
                align: 4,
                entsize: 0,
            },
        },
        SectionDescriptor {
            id: SectionId::OriginalRiscProg,
            behavior: SectionBehavior::Progbits {
                bytes: &sections.original_risc_prog,
                sh_flags: elf::SHF_ALLOC.into(),
                align: 4,
                entsize: 0,
            },
        },
    ];
    if computed_layout.maybe(SectionId::Rodata).is_some() {
        descs.push(SectionDescriptor {
            id: SectionId::Rodata,
            behavior: SectionBehavior::Progbits {
                bytes: &sections.rodata.bytes,
                sh_flags: elf::SHF_ALLOC.into(),
                align: sections.rodata.align.max(1),
                entsize: 0,
            },
        });
    }

    // Segment 2: `.stack`.
    descs.push(SectionDescriptor {
        id: SectionId::Stack,
        behavior: SectionBehavior::NoBits {
            align: STACK_SECTION_ALIGN,
        },
    });

    // Segment 3: `.data`, `.bss`, `.heap`, `.ecall_data`, `.register_store`.
    if computed_layout.maybe(SectionId::Data).is_some() {
        descs.push(SectionDescriptor {
            id: SectionId::Data,
            behavior: SectionBehavior::Data {
                bytes: &sections.data.bytes,
                align: sections.data.align.max(1),
            },
        });
    }
    if computed_layout.maybe(SectionId::Bss).is_some() {
        descs.push(SectionDescriptor {
            id: SectionId::Bss,
            behavior: SectionBehavior::NoBits {
                align: sections.bss.align.max(1),
            },
        });
    }
    descs.push(SectionDescriptor {
        id: SectionId::Heap,
        behavior: SectionBehavior::NoBits {
            align: HEAP_SECTION_ALIGN,
        },
    });
    descs.push(SectionDescriptor {
        id: SectionId::EcallData,
        behavior: SectionBehavior::NoBits {
            align: ECALL_DATA_SECTION_ALIGN,
        },
    });
    descs.push(SectionDescriptor {
        id: SectionId::RegisterStore,
        behavior: SectionBehavior::NoBits {
            align: REGISTER_STORE_ALIGN,
        },
    });

    descs
}

// ============================================================================
// Section header helpers
// ============================================================================

fn nobits_rw_header(
    name: StringId,
    s: TranspiledAddressAndSize,
    sh_offset: u32,
    align: u32,
) -> SectionHeader {
    SectionHeader {
        name: Some(name),
        sh_type: elf::SHT_NOBITS,
        sh_flags: (elf::SHF_ALLOC | elf::SHF_WRITE).into(),
        sh_addr: s.addr.as_u64(),
        sh_offset: sh_offset as u64,
        sh_size: s.size as u64,
        sh_link: 0,
        sh_info: 0,
        sh_addralign: align as u64,
        sh_entsize: 0,
    }
}

// ============================================================================
// Loop helper functions
// ============================================================================

/// Reserves file space for one section (step 2 of [`build_transpiled_elf`]).
fn reserve_section_space(
    writer: &mut Writer<'_>,
    desc: &SectionDescriptor<'_>,
    computed_layout: &ComputedLayout,
) -> Result<(), ElfBuilderError> {
    let s = computed_layout.require(desc.id)?;
    let fo = computed_layout.file_offset_of(s.addr, &format!("{:?}", desc.id))? as usize;
    match &desc.behavior {
        SectionBehavior::Progbits { bytes, align, .. } => {
            writer.reserve_until(fo);
            writer.reserve(bytes.len(), *align as usize);
        }
        SectionBehavior::NoBits { .. } => {} // No file space.
        SectionBehavior::Data { bytes, align } => {
            writer.reserve_until(fo);
            writer.reserve(bytes.len(), *align as usize);
        }
    }
    Ok(())
}

/// Writes one section's bytes (step 3 of [`build_transpiled_elf`]).
fn write_section_data(
    writer: &mut Writer<'_>,
    desc: &SectionDescriptor<'_>,
    computed_layout: &ComputedLayout,
) -> Result<(), ElfBuilderError> {
    let s = computed_layout.require(desc.id)?;
    let fo = computed_layout.file_offset_of(s.addr, &format!("{:?}", desc.id))? as usize;
    match &desc.behavior {
        SectionBehavior::Progbits { bytes, .. } => {
            writer.pad_until(fo);
            writer.write(bytes);
        }
        SectionBehavior::NoBits { .. } => {}
        SectionBehavior::Data { bytes, .. } => {
            writer.pad_until(fo);
            writer.write(bytes);
        }
    }
    Ok(())
}

/// Writes one section header (step 4 of [`build_transpiled_elf`]).
fn write_section_header(
    writer: &mut Writer<'_>,
    desc: &SectionDescriptor<'_>,
    computed_layout: &ComputedLayout,
    section_names: &[Option<StringId>; SectionId::COUNT],
) -> Result<(), ElfBuilderError> {
    let s = computed_layout.require(desc.id)?;
    let name = section_names[desc.id as usize];
    match &desc.behavior {
        SectionBehavior::Progbits {
            sh_flags,
            align,
            entsize,
            ..
        } => {
            writer.write_section_header(&SectionHeader {
                name,
                sh_type: elf::SHT_PROGBITS,
                sh_flags: *sh_flags,
                sh_addr: s.addr.as_u64(),
                sh_offset: computed_layout.file_offset_of(s.addr, &format!("{:?}", desc.id))?
                    as u64,
                sh_size: s.size as u64,
                sh_link: 0,
                sh_info: 0,
                sh_addralign: *align as u64,
                sh_entsize: *entsize as u64,
            });
        }
        SectionBehavior::NoBits { align } => {
            let name_id = name.ok_or(ElfBuilderError::AnonymousNoBitsSection)?;
            let sh_offset = computed_layout.file_offset_of(s.addr, &format!("{:?}", desc.id))?;
            writer.write_section_header(&nobits_rw_header(name_id, s, sh_offset, *align));
        }
        SectionBehavior::Data { align, .. } => {
            writer.write_section_header(&SectionHeader {
                name,
                sh_type: elf::SHT_PROGBITS,
                sh_flags: (elf::SHF_ALLOC | elf::SHF_WRITE).into(),
                sh_addr: s.addr.as_u64(),
                sh_offset: computed_layout.file_offset_of(s.addr, &format!("{:?}", desc.id))?
                    as u64,
                sh_size: s.size as u64,
                sh_link: 0,
                sh_info: 0,
                sh_addralign: *align as u64,
                sh_entsize: 0,
            });
        }
    }
    Ok(())
}

// ============================================================================
// Main builder
// ============================================================================

/// Lays out the sections, checks the layout, and writes the native ELF.
///
/// `branch_byte_size` is the size of one branch in the hook table, `arch_indicator` the
/// `e_machine` value, `seg_align` the segment alignment and `elf_flags` the `e_flags` value.
pub(crate) fn build_transpiled_elf(
    sections: OutputElfSections,
    branch_byte_size: u32,
    arch_indicator: u16,
    seg_align: u32,
    elf_flags: u32,
) -> Result<Vec<u8>, ElfBuilderError> {
    let computed_layout = compute_layout(&sections, seg_align)?;
    validate_guest_addresses(&computed_layout, &sections)?;
    loader_validate_layout(&computed_layout, &sections)?;

    // The code has `RoDataStart` (the address of `.original_risc_prog`) built in, from
    // `rodata_start_vaddr`; reject a layout that places the section anywhere else.
    let jump_table = computed_layout.require(SectionId::JumpTable)?;
    let jump_table_end =
        checked_add_u32(jump_table.addr.as_u32(), jump_table.size, "jump_table end")?;
    let baked = rodata_start_vaddr(jump_table_end)?;
    let laid_out = computed_layout
        .require(SectionId::OriginalRiscProg)?
        .addr
        .as_u32();
    if baked != laid_out {
        return Err(ElfBuilderError::RodataStartMismatch { baked, laid_out });
    }

    let transpiled_symbols_bytes =
        build_transpiled_symbols_bytes(&computed_layout, &sections, branch_byte_size)?;

    let text_vaddr = computed_layout.require(SectionId::Text)?.addr;
    let e_entry = text_vaddr.checked_add(sections.text.start_transpiled_code_addr.as_u32())?;

    let descs = build_section_descriptors(&sections, &computed_layout, &transpiled_symbols_bytes);

    let mut buffer = Vec::new();
    let mut writer = Writer::new(Endianness::Little, true, &mut buffer);

    writer.reserve_file_header();
    writer.reserve_program_headers(N_SEGMENTS as u32); // One `PT_LOAD` per segment, nothing else.
    writer.reserve_null_section_index();

    // Step 1: reserve section indices and names.

    let mut section_names = [None::<StringId>; SectionId::COUNT];

    for desc in &descs {
        let idx = desc.id as usize;
        writer.reserve_section_index();
        section_names[idx] = Some(writer.add_section_name(desc.id.name().as_bytes()));
    }

    writer.reserve_shstrtab_section_index();

    // Step 2: reserve file space.

    for desc in &descs {
        reserve_section_space(&mut writer, desc, &computed_layout)?;
    }

    writer.reserve_shstrtab();
    writer.reserve_section_headers();

    // Write the file header and program headers.

    writer.write_file_header(&FileHeader {
        os_abi: elf::ELFOSABI_NONE,
        abi_version: 0,
        e_type: elf::ET_DYN,
        e_machine: arch_indicator,
        e_entry: e_entry.as_u64(),
        e_flags: elf_flags,
    })?;

    writer.write_align_program_headers();

    for (seg_idx, &flags) in SEGMENT_FLAGS.iter().enumerate() {
        let seg = &computed_layout.segments[seg_idx];
        let fo = computed_layout.file_offset_of(seg.addr, &format!("segment {seg_idx}"))?;
        let filesz = computed_layout.seg_filesz[seg_idx];
        writer.write_program_header(&ProgramHeader {
            p_type: elf::PT_LOAD,
            p_flags: flags,
            p_offset: fo as u64,
            p_vaddr: seg.addr.as_u64(),
            p_paddr: seg.addr.as_u64(),
            p_filesz: filesz as u64,
            p_memsz: seg.size as u64,
            p_align: seg_align as u64,
        });
    }

    // Step 3: write section data.

    for desc in &descs {
        write_section_data(&mut writer, desc, &computed_layout)?;
    }

    // Step 4: write section headers.

    writer.write_shstrtab();
    writer.write_null_section_header();

    for desc in &descs {
        write_section_header(&mut writer, desc, &computed_layout, &section_names)?;
    }

    writer.write_shstrtab_section_header();

    Ok(buffer)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use riscv32::common::SEGMENT_BYTE_ALIGN;
    use riscv32_elf::{
        data_layout::{ECALL_DATA_SIZE_BYTES, REGISTER_STORE_SIZE_BYTES},
        types::RiscvAddress,
    };

    use super::*;

    const HEAP_SIZE_BYTES: u32 = 0x10000; // 64K
    const STACK_SIZE_BYTES: u32 = 0x10000; // 64K

    /// Minimal valid section set with a caller-chosen `.jump_table` size.
    fn minimal_sections(jump_table_len: usize) -> OutputElfSections {
        // R/W NOBITS layout: `.stack` is its own segment one SEGMENT_BYTE_ALIGN above rodata
        // (data/bss are empty), followed by the data segment `heap -> ecall_data ->
        // register_store`. `stack_top` therefore lands one stack-size above SEGMENT_BYTE_ALIGN.
        let stack_top = SEGMENT_BYTE_ALIGN
            .checked_add(STACK_SIZE_BYTES)
            .expect("stack_top overflow in test fixture");
        let data_end = stack_top
            .checked_add(HEAP_SIZE_BYTES)
            .and_then(|v| v.checked_add(ECALL_DATA_SIZE_BYTES))
            .expect("data_end overflow in test fixture");
        OutputElfSections {
            text: TextSection {
                bytes: vec![0; 0x200],
                start_transpiled_code_addr: TranspiledAddress::new(0x148),
                hooks_begin_addr: TranspiledAddress::new(0x1f0),
                hooks_end_addr: TranspiledAddress::new(0x1f8),
                n_init_array_functions: 0,
            },
            jump_table: vec![0; jump_table_len],
            original_risc_prog: vec![0; 0x4c],
            rodata: AlignedBytes {
                bytes: vec![],
                align: 1,
                guest_addr: 0,
            },
            data: AlignedBytes {
                bytes: vec![],
                align: 1,
                guest_addr: 0,
            },
            bss: NoBitsSection {
                size: 0,
                align: 1,
                guest_addr: 0,
            },
            runtime: RuntimeSections {
                ecall_data_size: ECALL_DATA_SIZE_BYTES,
                stack_size: STACK_SIZE_BYTES,
                heap_size: HEAP_SIZE_BYTES,
                register_store_size: REGISTER_STORE_SIZE_BYTES,
            },
            risc_symbols: RvSymbolValues {
                text_end: RiscvAddress::new(0),
                // The writable region (`_data_start_value`) starts at `.stack`, one
                // SEGMENT_BYTE_ALIGN above rodata since `.data`/`.bss` are empty here.
                data_start: RiscvAddress::new(SEGMENT_BYTE_ALIGN),
                stack_top: RiscvAddress::new(stack_top),
                data_end: RiscvAddress::new(data_end),
            },
        }
    }

    #[test]
    fn empty_so_builds_successfully_with_minimal_sections() {
        let sections = minimal_sections(16);

        let elf = build_transpiled_elf(sections, 4, 62, SEGMENT_BYTE_ALIGN, 0)
            .expect("build_transpiled_elf should succeed with minimal valid sections");

        // PROGBITS bytes + a generous overhead of 8 segments worth of alignment padding.
        // Stays comfortably under the NOBITS sizes (512 KiB heap + 64 KiB stack + 64 KiB ecall_data)
        // that must NOT be on disk.
        let progbits_bytes: u32 = 0x200 + 16 + 0x4c;
        let upper_bound = progbits_bytes + 8 * SEGMENT_BYTE_ALIGN;
        assert!(
            (elf.len() as u32) < upper_bound,
            "File size {} should be < {upper_bound} (NOBITS sections shouldn't occupy disk space)",
            elf.len()
        );

        assert_eq!(&elf[0..4], &[0x7f, b'E', b'L', b'F']); // ELF magic
        assert_eq!(elf[4], 2); // ELFCLASS64
        assert_eq!(elf[5], 1); // ELFDATA2LSB
        assert_eq!(u16::from_le_bytes([elf[16], elf[17]]), 3); // ET_DYN
        assert_eq!(u16::from_le_bytes([elf[18], elf[19]]), 62); // Architecture indicator

        // Each PT_LOAD's file range must lie inside the actual file. The loader
        // mmaps `[p_offset, p_offset + p_filesz)` from disk, so an over-claimed
        // `p_filesz` (e.g. from including trailing alignment padding that was
        // never written) would SIGBUS or fail loader validation.
        assert_pt_load_filesz_within_file(&elf);
    }

    /// `.jump_table` ends 8 bytes before a segment boundary, so the 32-byte
    /// `.transpiled.symbols` spills across it and pushes `.original_risc_prog` to the
    /// next boundary. The layout and the transpiler-baked `rodata_start_vaddr`
    /// must agree on the spilled address; a naive `align_segment(jump_table
    /// end)` lands one segment too low and corrupts every baked guest-memory
    /// address.
    #[test]
    fn rodata_start_matches_layout_when_transpiled_symbols_crosses_segment_boundary() {
        let jump_table_len = (SEGMENT_BYTE_ALIGN - 8) as usize;
        let sections = minimal_sections(jump_table_len);

        let layout = compute_layout(&sections, SEGMENT_BYTE_ALIGN).expect("layout should compute");
        let jump_table = layout
            .require(SectionId::JumpTable)
            .expect("jump_table section");
        let jump_table_end = jump_table.addr.as_u32() + jump_table.size;
        let laid_out = layout
            .require(SectionId::OriginalRiscProg)
            .expect("original_risc_prog section")
            .addr
            .as_u32();

        let baked = rodata_start_vaddr(jump_table_end).expect("rodata_start_vaddr");
        assert_eq!(baked, laid_out);
        // The fixture must actually exercise the spill: the naive computation
        // disagrees here.
        let naive = checked_align_segment(jump_table_end, "test").expect("align");
        assert_ne!(naive, laid_out);

        // The full build (including its baked-vs-layout cross-check) succeeds.
        build_transpiled_elf(sections, 4, 62, SEGMENT_BYTE_ALIGN, 0)
            .expect("build_transpiled_elf should succeed at the segment-boundary edge case");
    }

    /// [`minimal_sections`] with a `.rodata` of `rodata_len` bytes, and the writable-region
    /// symbols moved up to match.
    fn sections_with_rodata(rodata_len: usize) -> OutputElfSections {
        let mut sections = minimal_sections(16);
        let rodata_span = checked_align_segment(
            u32::try_from(rodata_len).expect("rodata_len fits u32"),
            "test rodata span",
        )
        .expect("rodata span");
        // `.original_risc_prog` is one segment, and `.rodata` its aligned span, below `.stack`.
        let data_start = SEGMENT_BYTE_ALIGN + rodata_span;
        let stack_top = data_start + STACK_SIZE_BYTES;
        let data_end = stack_top + HEAP_SIZE_BYTES + ECALL_DATA_SIZE_BYTES;
        sections.rodata = AlignedBytes {
            bytes: vec![0xAB; rodata_len],
            align: 4,
            // `.rodata` starts one segment above `.original_risc_prog`.
            guest_addr: SEGMENT_BYTE_ALIGN,
        };
        sections.risc_symbols = RvSymbolValues {
            text_end: RiscvAddress::new(0),
            data_start: RiscvAddress::new(data_start),
            stack_top: RiscvAddress::new(stack_top),
            data_end: RiscvAddress::new(data_end),
        };
        sections
    }

    /// Every guest-readable byte, from `.original_risc_prog` up to `_data_end_value`, must lie
    /// in some PT_LOAD's memory range. The loader leaves a gap unmapped when the host page size
    /// is smaller than SEGMENT_BYTE_ALIGN, so a bounds-checked guest load into it would fault
    /// the host.
    #[test]
    fn pt_loads_cover_the_guest_readable_range() {
        use object::{Object as _, ObjectSection as _, ObjectSegment as _};

        for rodata_len in [0, 0x10, 0x1004, SEGMENT_BYTE_ALIGN as usize] {
            let sections = sections_with_rodata(rodata_len);
            let data_end = u64::from(sections.risc_symbols.data_end.as_u32());
            let elf = build_transpiled_elf(sections, 4, 62, SEGMENT_BYTE_ALIGN, 0)
                .unwrap_or_else(|e| panic!("rodata_len={rodata_len:#x}: build failed: {e}"));
            let file = object::File::parse(elf.as_slice()).expect("parse built ELF");
            let readable_start = file
                .section_by_name(ORIGINAL_RISC_PROG_SECTION_NAME)
                .expect(".original_risc_prog section")
                .address();
            let readable_end = readable_start + data_end;

            let mut covered_to = readable_start;
            for segment in file.segments() {
                let (start, end) = (segment.address(), segment.address() + segment.size());
                if start <= covered_to && covered_to < end {
                    covered_to = end;
                }
            }
            assert!(
                covered_to >= readable_end,
                "rodata_len={rodata_len:#x}: guest-readable range \
                 [{readable_start:#x}, {readable_end:#x}) is unmapped from {covered_to:#x}"
            );
        }
    }

    /// Parse PT_LOAD program headers from a built ELF and assert that
    /// `p_offset + p_filesz` does not exceed the file length for any segment.
    fn assert_pt_load_filesz_within_file(elf: &[u8]) {
        let e_phoff = u64::from_le_bytes(elf[32..40].try_into().unwrap()) as usize;
        let e_phentsize = u16::from_le_bytes(elf[54..56].try_into().unwrap()) as usize;
        let e_phnum = u16::from_le_bytes(elf[56..58].try_into().unwrap()) as usize;
        assert_eq!(e_phentsize, 56, "ELF64 program header size");

        for i in 0..e_phnum {
            let base = e_phoff
                .checked_add(i.checked_mul(e_phentsize).expect("phentsize overflow"))
                .expect("phoff + i*phentsize overflow");
            let p_type_end = base.checked_add(4).expect("p_type end overflow");
            let p_type = u32::from_le_bytes(elf[base..p_type_end].try_into().unwrap());
            if p_type != elf::PT_LOAD {
                continue;
            }
            let p_offset_start = base.checked_add(8).expect("p_offset start overflow");
            let p_offset_end = base.checked_add(16).expect("p_offset end overflow");
            let p_filesz_start = base.checked_add(32).expect("p_filesz start overflow");
            let p_filesz_end = base.checked_add(40).expect("p_filesz end overflow");
            let p_offset =
                u64::from_le_bytes(elf[p_offset_start..p_offset_end].try_into().unwrap());
            let p_filesz =
                u64::from_le_bytes(elf[p_filesz_start..p_filesz_end].try_into().unwrap());
            if p_filesz == 0 {
                // No file bytes are mapped from this segment; p_offset is
                // unused by the loader.
                continue;
            }
            let end = p_offset
                .checked_add(p_filesz)
                .expect("p_offset + p_filesz overflow");
            assert!(
                end <= elf.len() as u64,
                "PT_LOAD {i}: p_offset={p_offset} + p_filesz={p_filesz} exceeds file size {}",
                elf.len()
            );
        }
    }
}
