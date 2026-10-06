//! Loads a transpiled native ELF from its bytes.
//!
//! The loader checks that the ELF is native code for this host, that its `e_flags` are 0 or
//! `EF_TRUSTED_TRANSPILED`, that the host page size is at most 16 KiB, and that the guest's
//! readable and writable ranges and the register store lie in PT_LOAD segments with the right
//! permissions. It then reserves one address range, copies each segment into anonymous memory
//! there and applies the segment's permissions. No file is mapped and no dynamic linker is used.
//!
//! Loading and calling native code cannot be checked by the compiler. The unsafe code that does
//! it is kept small and lives here, in `code_runner.rs` and in the per-architecture run wrappers:
//! mapping and protecting segments, deriving addresses, the call into the envelope, and copying
//! to and from guest memory and the register store.

use log::debug;
use object::{File, Object, ObjectSection};
use riscv32::N_BYTES_IN_WORD;
use riscv32::common::{
    SEGMENT_BYTE_ALIGN, checked_add_u32, checked_add_usize, checked_sub_usize, checked_u64_to_u32,
    checked_usize_to_u32,
};
use riscv32_elf::consts::{MAX_NUM_OF_ENTRY_POINTS, MAX_NUM_OF_INIT_FUNCTIONS};
use riscv32_elf::consts::{TRANSPILED_SYMBOLS_SECTION_NAME, TranspiledSymbolsSection};
use riscv32_elf::data_layout::REGISTER_STORE_SIZE_BYTES;
use riscv32_elf::types::{RiscvAddress, VirtualDataPointer};
use rustix::mm::{MapFlags, MprotectFlags, ProtFlags, mprotect};
use std::collections::HashMap;
use std::ffi::c_void;

use crate::arch64::ops::Arm64Op;
use crate::execute::LoadedLayout;
use crate::execute::error::LoaderError;
use crate::layout::{EF_TRUSTED_TRANSPILED, SectionId, validate_layout};
use crate::traits::TranspiledOp as _;
use crate::transpiled_address::{TranspiledAddress, TranspiledAddressAndSize};
use crate::x86_64::ops::X86Op;
use strum::IntoEnumIterator as _;

/// A code address in a loaded library, such as the envelope entry point.
///
/// The architecture-specific run wrappers cast `handle` to a function pointer.
#[derive(Debug)]
pub(crate) struct SymbolHandle {
    pub(crate) handle: *mut c_void,
}

/// Information extracted from a transpiled ELF file for address derivation.
struct TranspiledElfInfo {
    /// Section addresses and sizes, indexed by `SectionId`.
    sections: [Option<TranspiledAddressAndSize>; SectionId::COUNT],
    /// Symbol values from `.transpiled.symbols`.
    symbols: TranspiledSymbolsSection,
    /// ELF target architecture.
    architecture: object::Architecture,
    /// Whether the ELF `e_flags` mark a trusted artifact.
    trusted: bool,
}

/// Reads the loader's metadata from the `.transpiled.symbols` section.
fn read_transpiled_symbols(elf: &File<'_>) -> Result<TranspiledSymbolsSection, LoaderError> {
    let section = elf
        .section_by_name(TRANSPILED_SYMBOLS_SECTION_NAME)
        .ok_or_else(|| LoaderError::SectionNotFound {
            section: TRANSPILED_SYMBOLS_SECTION_NAME.to_string(),
        })?;
    let data = section.data().map_err(|e| LoaderError::ElfParsingError {
        message: format!("Failed to read .transpiled.symbols data: {e}"),
    })?;
    TranspiledSymbolsSection::from_bytes(data).ok_or_else(|| LoaderError::ElfParsingError {
        message: format!(
            ".transpiled.symbols is {} bytes, expected {}",
            data.len(),
            TranspiledSymbolsSection::SIZE_IN_BYTES
        ),
    })
}

/// Parses the ELF headers, checks the architecture and `e_flags`, and reads the section
/// addresses and `.transpiled.symbols`.
fn parse_elf_info(elf_bytes: &[u8]) -> Result<TranspiledElfInfo, LoaderError> {
    let elf = File::parse(elf_bytes).map_err(|e| LoaderError::ElfParsingError {
        message: format!("Failed to parse ELF: {e}"),
    })?;
    // Only native code for this host can run here.
    if Some(elf.architecture()) != host_architecture() {
        return Err(LoaderError::CompiledWithWrongArchitecture);
    }
    // The transpile mode is stamped into the ELF `e_flags`: 0, or `EF_TRUSTED_TRANSPILED`.
    let trusted = match elf.flags() {
        object::FileFlags::Elf { e_flags: 0, .. } => false,
        object::FileFlags::Elf {
            e_flags: EF_TRUSTED_TRANSPILED,
            ..
        } => true,
        object::FileFlags::Elf { e_flags, .. } => {
            return Err(LoaderError::UnknownElfFlags { e_flags });
        }
        _ => {
            return Err(LoaderError::ElfParsingError {
                message: "Unexpected non-ELF file type".to_string(),
            });
        }
    };

    let mut sections: [Option<TranspiledAddressAndSize>; SectionId::COUNT] =
        [None; SectionId::COUNT];
    for section in elf.sections() {
        let Ok(name) = section.name() else { continue };
        if let Some(section_id) = SectionId::iter().find(|id| id.name() == name) {
            sections[section_id as usize] = Some(TranspiledAddressAndSize {
                addr: TranspiledAddress::new(checked_u64_to_u32(section.address(), name)?),
                size: checked_u64_to_u32(section.size(), name)?,
            });
        }
    }

    let required_sections = [
        SectionId::Text,
        SectionId::JumpTable,
        SectionId::OriginalRiscProg,
        SectionId::EcallData,
        SectionId::Stack,
        SectionId::RegisterStore,
    ];

    for &required_id in &required_sections {
        if sections[required_id as usize].is_none() {
            return Err(LoaderError::SectionNotFound {
                section: required_id.name().to_string(),
            });
        }
    }

    let gs = read_transpiled_symbols(&elf)?;

    Ok(TranspiledElfInfo {
        sections,
        symbols: gs,
        architecture: elf.architecture(),
        trusted,
    })
}

/// The context a transpiled artifact is being loaded into.
///
/// A trusted artifact (see [`EF_TRUSTED_TRANSPILED`]) has no gas metering and no bounds
/// checks, so the loader refuses to load one into an [`LoaderContext::Untrusted`] context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoaderContext {
    /// The host trusts the code; trusted and untrusted artifacts may both be loaded.
    Trusted,
    /// The code may be hostile; only artifacts with full sandboxing may be loaded.
    Untrusted,
}
impl LoaderContext {
    pub(crate) fn is_trusted(&self) -> bool {
        matches!(self, LoaderContext::Trusted)
    }
}

/// Builds the `LoadedLayout` of a library loaded at `base_address`.
///
/// The three hook addresses are loaded host addresses. The function validates the section
/// layout and counts the init functions and entry points from the size of the hook table.
fn build_loaded_layout(
    base_address: usize,
    entry_points_begin_addr: usize,
    init_array_end_addr: usize,
    entry_points_end_addr: usize,
    elf_info: &TranspiledElfInfo,
    branch_byte_size: usize,
) -> Result<LoadedLayout, LoaderError> {
    let start_transpiled_code_addr = checked_add_usize(
        base_address,
        elf_info.symbols.start_transpiled_code as usize,
        "start_transpiled_code_addr",
    )?;

    let section_vaddr = |id: SectionId| -> Result<TranspiledAddressAndSize, LoaderError> {
        elf_info.sections[id as usize].ok_or_else(|| LoaderError::SectionNotFound {
            section: id.name().to_string(),
        })
    };

    let original_risc_prog = section_vaddr(SectionId::OriginalRiscProg)?;
    let stack = section_vaddr(SectionId::Stack)?;
    let register_store = section_vaddr(SectionId::RegisterStore)?;

    let mut section_map = HashMap::new();
    for id in SectionId::iter() {
        if let Some(vas) = elf_info.sections[id as usize] {
            section_map.insert(id, vas);
        }
    }

    validate_layout(
        &section_map,
        TranspiledAddress::new(elf_info.symbols.text_end_value),
        TranspiledAddress::new(elf_info.symbols.data_start_value),
        TranspiledAddress::new(elf_info.symbols.stack_top_value),
        TranspiledAddress::new(elf_info.symbols.data_end_value),
    )
    .map_err(LoaderError::LayoutValidation)?;

    let rodata_ptr = checked_add_usize(
        base_address,
        original_risc_prog.addr.as_usize(),
        "rodata_ptr",
    )?;
    let data_start_ptr = checked_add_usize(
        rodata_ptr,
        elf_info.symbols.data_start_value as usize,
        "data_start_ptr",
    )?;
    let stack_bottom_ptr =
        checked_add_usize(base_address, stack.addr.as_usize(), "stack_bottom_ptr")?;
    let data_end_ptr = checked_add_usize(
        rodata_ptr,
        elf_info.symbols.data_end_value as usize,
        "data_end_ptr",
    )?;
    let register_store_ptr = checked_add_usize(
        base_address,
        register_store.addr.as_usize(),
        "register_store_ptr",
    )?;

    // The hook table holds one branch per init function, then one per entry point.
    if !(entry_points_begin_addr..=entry_points_end_addr).contains(&init_array_end_addr) {
        return Err(LoaderError::InitArrayOutsideEntryPoints {
            init_array_end: init_array_end_addr,
            entry_points_begin: entry_points_begin_addr,
            entry_points_end: entry_points_end_addr,
        });
    }
    let init_array_byte_size = init_array_end_addr - entry_points_begin_addr;
    if !init_array_byte_size.is_multiple_of(branch_byte_size) {
        return Err(LoaderError::InitArrayUnaligned {
            init_array_size_in_bytes: init_array_byte_size,
            branch_size_in_bytes: branch_byte_size,
        });
    }
    let n_init_array_functions = init_array_byte_size / branch_byte_size;
    if n_init_array_functions > MAX_NUM_OF_INIT_FUNCTIONS {
        return Err(LoaderError::TooManyInitFunctions {
            count: n_init_array_functions,
            max: MAX_NUM_OF_INIT_FUNCTIONS,
        });
    }
    let entry_points_byte_size = entry_points_end_addr - entry_points_begin_addr;
    if !entry_points_byte_size.is_multiple_of(branch_byte_size) {
        return Err(LoaderError::EntryPointsUnaligned {
            entry_points_size_in_bytes: entry_points_byte_size,
            branch_size_in_bytes: branch_byte_size,
        });
    }
    let n_entry_point_functions =
        (entry_points_byte_size / branch_byte_size) - n_init_array_functions;
    if n_entry_point_functions > MAX_NUM_OF_ENTRY_POINTS {
        return Err(LoaderError::TooManyEntryPoints {
            count: n_entry_point_functions,
            max: MAX_NUM_OF_ENTRY_POINTS,
        });
    }

    let data_start_from_rodata = checked_usize_to_u32(
        checked_sub_usize(data_start_ptr, rodata_ptr, "data_start_from_rodata")?,
        "data_start_from_rodata",
    )?;
    let data_end_from_rodata = checked_usize_to_u32(
        checked_sub_usize(data_end_ptr, rodata_ptr, "data_end_from_rodata")?,
        "data_end_from_rodata",
    )?;
    let stack_bottom_from_rodata = checked_usize_to_u32(
        checked_sub_usize(stack_bottom_ptr, rodata_ptr, "stack_bottom_from_rodata")?,
        "stack_bottom_from_rodata",
    )?;
    let stack_top_from_rodata = checked_add_u32(
        stack_bottom_from_rodata,
        stack.size,
        "stack_top_from_rodata",
    )?;

    let risc_code_end_addr = RiscvAddress::new(elf_info.symbols.text_end_value);

    let layout = LoadedLayout {
        n_init_array_functions,
        n_entry_point_functions,
        start_transpiled_code: start_transpiled_code_addr,
        virtual_data_start: VirtualDataPointer::new(data_start_from_rodata),
        virtual_stack_bottom: VirtualDataPointer::new(stack_bottom_from_rodata),
        virtual_stack_top: VirtualDataPointer::new(stack_top_from_rodata),
        virtual_data_end: VirtualDataPointer::new(data_end_from_rodata),
        derived_rodata_start: rodata_ptr,
        derived_register_store_start: register_store_ptr,
        risc_code_end_addr,
    };
    layout.log_summary("build_loaded_layout");
    Ok(layout)
}

/// Owns the memory a library is loaded into. Unmaps it on drop.
#[derive(Debug)]
pub(crate) struct MmapHandle {
    base: *mut c_void,
    total_len_in_bytes: usize,
}

impl Drop for MmapHandle {
    fn drop(&mut self) {
        // SAFETY: `base` and `total_len_in_bytes` are the address and length of the reservation
        // made in `load_library`, which this handle alone owns. Pointers into the mapping are
        // reached only through the `TranspiledVm` that owns this handle, so none outlive it.
        #[expect(unsafe_code, reason = "See SAFETY comment")]
        let _ = unsafe { rustix::mm::munmap(self.base, self.total_len_in_bytes) };
    }
}

// SAFETY: `MmapHandle` holds a pointer to a process-private (`MAP_PRIVATE`) mapping and uses
// it only to call `munmap` in `Drop`, which any thread may do.
#[expect(unsafe_code, reason = "See SAFETY comment")]
unsafe impl Send for MmapHandle {}

/// Upper bounds that keep a malformed ELF from using unbounded memory. They are policy limits,
/// well above any valid output, not OS constraints.
const MAX_ELF_SIZE_IN_BYTES: usize = 256 * 1024 * 1024;
const MAX_MMAP_SPAN_SIZE_IN_BYTES: usize = 256 * 1024 * 1024;

/// Unmaps an mmap region on drop unless disarmed, so a failed load releases it.
struct MmapRegionGuard {
    ptr: *mut c_void,
    len_in_bytes: usize,
}

impl MmapRegionGuard {
    fn disarm(mut self) -> (*mut c_void, usize) {
        let result = (self.ptr, self.len_in_bytes);
        self.ptr = std::ptr::null_mut();
        result
    }
}

impl Drop for MmapRegionGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: `ptr` and `len_in_bytes` are the address and length of a successful
            // `mmap`. `disarm` nulls `ptr`, so a region handed to `MmapHandle` is not unmapped
            // here.
            #[expect(unsafe_code, reason = "See SAFETY comment")]
            let _ = unsafe { rustix::mm::munmap(self.ptr, self.len_in_bytes) };
        }
    }
}

// --- Segments vs pages ------------------------------------------------------
//
// Segments are ELF program-header (PT_LOAD) entries: a virtual address, file
// offset, sizes and permissions. The transpiler aligns every segment to
// `SEGMENT_BYTE_ALIGN` (16 KiB). Pages are the host's mapping granularity,
// read at run time: 4 KiB on x86_64 and most aarch64 Linux, 16 KiB on Apple
// Silicon. Segments can be mapped with their own permissions only when the page
// size divides the segment alignment, so hosts with larger pages (64 KiB-page
// aarch64 Linux) are rejected.

/// Returns the host page size, or an error if segments cannot be mapped with it.
fn host_page_size() -> Result<usize, LoaderError> {
    let page_size = rustix::param::page_size();
    if page_size > SEGMENT_BYTE_ALIGN as usize || !page_size.is_power_of_two() {
        return Err(LoaderError::UnsupportedPageSize {
            page_size,
            max_page_size: SEGMENT_BYTE_ALIGN as usize,
        });
    }
    Ok(page_size)
}

/// Rounds `addr` up to a multiple of `page_size` (a power of two), with overflow checking.
fn align_up_page(addr: usize, page_size: usize, context: &str) -> Result<usize, LoaderError> {
    Ok(checked_add_usize(addr, page_size - 1, context)? & !(page_size - 1))
}

/// The architecture of native code this host runs.
fn host_architecture() -> Option<object::Architecture> {
    if cfg!(target_arch = "aarch64") {
        Some(object::Architecture::Aarch64)
    } else if cfg!(target_arch = "x86_64") {
        Some(object::Architecture::X86_64)
    } else {
        None
    }
}

/// Converts ELF `PF_*` flags to `rustix::mm::ProtFlags`.
fn prot_from_elf_flags(flags: u32) -> ProtFlags {
    use object::elf::{PF_R, PF_W, PF_X};
    let mut prot = ProtFlags::empty();
    if flags & PF_R != 0 {
        prot |= ProtFlags::READ;
    }
    if flags & PF_W != 0 {
        prot |= ProtFlags::WRITE;
    }
    if flags & PF_X != 0 {
        prot |= ProtFlags::EXEC;
    }
    prot
}

/// Converts `mmap`'s [`ProtFlags`] into `mprotect`'s [`MprotectFlags`]; rustix uses distinct
/// types for the same bits.
fn prot_to_mprotect(p: ProtFlags) -> MprotectFlags {
    let mut m = MprotectFlags::empty();
    if p.contains(ProtFlags::READ) {
        m |= MprotectFlags::READ;
    }
    if p.contains(ProtFlags::WRITE) {
        m |= MprotectFlags::WRITE;
    }
    if p.contains(ProtFlags::EXEC) {
        m |= MprotectFlags::EXEC;
    }
    m
}

/// Returns the size of one branch in the entry-point hook table for `arch`.
fn branch_byte_size_for_arch(arch: object::Architecture) -> Result<usize, LoaderError> {
    match arch {
        object::Architecture::Aarch64 => Ok(Arm64Op::branch_size_in_bytes() as usize),
        object::Architecture::X86_64 => Ok(X86Op::branch_size_in_bytes() as usize),
        _ => Err(LoaderError::CompiledWithWrongArchitecture),
    }
}

/// Maps anonymous memory at `addr`, replacing part of the caller's reserved range.
///
/// The caller must ensure that `[addr, addr + len_in_bytes)` lies inside the range it reserved:
/// `MAP_FIXED` silently replaces any mapping already there.
#[expect(unsafe_code, reason = "See SAFETY comment")]
fn safe_mmap_anon_segment(
    addr: *mut c_void,
    len_in_bytes: usize,
    prot: ProtFlags,
    seg_vaddr: usize,
) -> Result<(), LoaderError> {
    // SAFETY: the caller guarantees the range lies inside the region it reserved, so no other
    // mapping is replaced.
    unsafe {
        rustix::mm::mmap_anonymous(
            addr,
            len_in_bytes,
            prot,
            MapFlags::PRIVATE | MapFlags::FIXED,
        )
    }
    .map_err(|e| LoaderError::MmapSegmentFailed {
        vaddr: seg_vaddr,
        error: e.to_string(),
    })?;
    Ok(())
}

/// A validated PT_LOAD segment extracted from an ELF file.
struct LoadSegment {
    vaddr: usize,
    offset: usize,
    filesz: usize,
    memsz: usize,
    prot: ProtFlags,
}

/// The virtual address range spanned by all PT_LOAD segments.
struct VaddrRange {
    min: usize,
    max_end: usize,
}

/// Parses the PT_LOAD segments of an ELF64 file and checks their alignment, permissions (never
/// W+X) and file ranges.
fn parse_load_segments(
    elf_bytes: &[u8],
    page_size: usize,
) -> Result<(Vec<LoadSegment>, VaddrRange), LoaderError> {
    let elf = File::parse(elf_bytes).map_err(|e| LoaderError::ElfParsingError {
        message: format!("Failed to parse ELF: {e}"),
    })?;

    let File::Elf64(ref elf64) = elf else {
        return Err(LoaderError::UnsupportedElfFormat);
    };
    let endian = elf64.endian();

    let mut segments = Vec::new();
    let mut min_vaddr = usize::MAX;
    let mut max_vaddr_end: usize = 0;

    for ph in elf64.elf_program_headers() {
        use object::elf::PT_LOAD;
        use object::read::elf::ProgramHeader as _;
        if ph.p_type(endian) != PT_LOAD {
            continue;
        }
        let vaddr =
            usize::try_from(ph.p_vaddr(endian)).map_err(|_| LoaderError::ElfParsingError {
                message: format!("p_vaddr overflows usize: {:#x}", ph.p_vaddr(endian)),
            })?;
        if !vaddr.is_multiple_of(page_size) {
            return Err(LoaderError::UnalignedSegmentVaddr { vaddr });
        }
        let prot = prot_from_elf_flags(ph.p_flags(endian));
        if prot.contains(ProtFlags::WRITE) && prot.contains(ProtFlags::EXEC) {
            return Err(LoaderError::WritableExecutableSegment { vaddr });
        }
        let offset =
            usize::try_from(ph.p_offset(endian)).map_err(|_| LoaderError::ElfParsingError {
                message: format!("p_offset overflows usize: {:#x}", ph.p_offset(endian)),
            })?;
        let filesz =
            usize::try_from(ph.p_filesz(endian)).map_err(|_| LoaderError::ElfParsingError {
                message: format!("p_filesz overflows usize: {:#x}", ph.p_filesz(endian)),
            })?;
        let memsz =
            usize::try_from(ph.p_memsz(endian)).map_err(|_| LoaderError::ElfParsingError {
                message: format!("p_memsz overflows usize: {:#x}", ph.p_memsz(endian)),
            })?;
        if filesz > memsz {
            return Err(LoaderError::InvalidSegmentFileszExceedsMemsz { filesz, memsz });
        }
        // A zero-filesz segment copies no file bytes, so its `p_offset` is unused and need
        // not be within the file.
        if filesz > 0 {
            let in_file = checked_add_usize(offset, filesz, "segment_file_end")
                .is_ok_and(|file_end| file_end <= elf_bytes.len());
            if !in_file {
                return Err(LoaderError::SegmentFileRangeOutOfBounds {
                    offset,
                    filesz,
                    file_size_in_bytes: elf_bytes.len(),
                });
            }
        }

        min_vaddr = min_vaddr.min(vaddr);
        max_vaddr_end = max_vaddr_end.max(checked_add_usize(vaddr, memsz, "segment_end")?);
        segments.push(LoadSegment {
            vaddr,
            offset,
            filesz,
            memsz,
            prot,
        });
    }

    if segments.is_empty() {
        return Err(LoaderError::NoLoadSegments);
    }
    // Each segment is mapped with `MAP_FIXED` over whole pages, so overlapping segments would
    // silently replace each other's pages and permissions.
    segments.sort_by_key(|seg| seg.vaddr);
    for pair in segments.windows(2) {
        if let [first, second] = pair {
            let first_end = align_up_page(
                checked_add_usize(first.vaddr, first.memsz, "segment_end")?,
                page_size,
                "segment_end",
            )?;
            if second.vaddr < first_end {
                return Err(LoaderError::OverlappingSegments {
                    first_start: first.vaddr,
                    first_end,
                    second_start: second.vaddr,
                });
            }
        }
    }

    Ok((
        segments,
        VaddrRange {
            min: min_vaddr,
            max_end: max_vaddr_end,
        },
    ))
}

/// Checks that the ELF virtual-address range `[start, end)` lies entirely inside PT_LOAD memory
/// ranges whose permissions include `prot`.
///
/// Coverage is checked at ELF granularity, not host-page granularity: a gap that a large host
/// page happens to hide on one platform is unmapped on a host with smaller pages, where a
/// bounds-checked guest access into it faults the host process.
fn ensure_covered_by_segments(
    segments: &[LoadSegment],
    start: usize,
    end: usize,
    prot: ProtFlags,
    access: &str,
) -> Result<(), LoaderError> {
    let mut covered_to = start;
    while covered_to < end {
        let containing = segments.iter().find(|seg| {
            seg.prot.contains(prot)
                && (seg.vaddr..seg.vaddr.saturating_add(seg.memsz)).contains(&covered_to)
        });
        let Some(seg) = containing else {
            return Err(LoaderError::RangeNotMapped {
                access: access.to_string(),
                start,
                end,
                uncovered: covered_to,
            });
        };
        covered_to = checked_add_usize(seg.vaddr, seg.memsz, "segment_end")?;
    }
    Ok(())
}

/// Checks that the memory the code, the guest and the host access is mapped with the right
/// permissions:
/// - `.text` is executable, and the jump table it reads is readable and in no writable segment;
/// - guest loads reach `[0, _data_end_value)` and stores `[_data_start_value, _data_end_value)`,
///   as offsets from `.original_risc_prog`;
/// - the host reads and writes the register store, which must also be 8-byte aligned for its
///   64-bit gas slot.
fn ensure_accessed_ranges_mapped(
    segments: &[LoadSegment],
    elf_info: &TranspiledElfInfo,
) -> Result<(), LoaderError> {
    let section = |id: SectionId| {
        elf_info.sections[id as usize].ok_or_else(|| LoaderError::SectionNotFound {
            section: id.name().to_string(),
        })
    };
    let section_range = |id: SectionId| -> Result<(usize, usize), LoaderError> {
        let placed = section(id)?;
        let start = placed.addr.as_usize();
        Ok((
            start,
            checked_add_usize(start, placed.size as usize, id.name())?,
        ))
    };
    let (text_start, text_end) = section_range(SectionId::Text)?;
    ensure_covered_by_segments(
        segments,
        text_start,
        text_end,
        ProtFlags::EXEC,
        "executable",
    )?;
    let (jump_table_start, jump_table_end) = section_range(SectionId::JumpTable)?;
    ensure_covered_by_segments(
        segments,
        jump_table_start,
        jump_table_end,
        ProtFlags::READ,
        "jump-table",
    )?;
    // The code jumps through the table, so nothing may write it once loaded.
    if let Some(seg) = segments.iter().find(|seg| {
        seg.prot.contains(ProtFlags::WRITE)
            && seg.vaddr < jump_table_end
            && jump_table_start < seg.vaddr.saturating_add(seg.memsz)
    }) {
        return Err(LoaderError::WritableJumpTable { vaddr: seg.vaddr });
    }

    let guest_base = section(SectionId::OriginalRiscProg)?.addr.as_usize();
    let guest_addr = |offset: u32, name: &str| {
        checked_add_usize(guest_base, offset as usize, name).map_err(LoaderError::from)
    };
    let data_start = guest_addr(elf_info.symbols.data_start_value, "guest_data_start")?;
    let data_end = guest_addr(elf_info.symbols.data_end_value, "guest_data_end")?;
    ensure_covered_by_segments(segments, guest_base, data_end, ProtFlags::READ, "readable")?;
    ensure_covered_by_segments(segments, data_start, data_end, ProtFlags::WRITE, "writable")?;

    let register_store = section(SectionId::RegisterStore)?.addr.as_usize();
    if !register_store.is_multiple_of(align_of::<i64>()) {
        return Err(LoaderError::MisalignedSection {
            section: SectionId::RegisterStore.name().to_string(),
            addr: register_store,
        });
    }
    let register_store_end = checked_add_usize(
        register_store,
        REGISTER_STORE_SIZE_BYTES as usize,
        "register_store_end",
    )?;
    ensure_covered_by_segments(
        segments,
        register_store,
        register_store_end,
        ProtFlags::READ | ProtFlags::WRITE,
        "register-store",
    )
}

/// Maps one PT_LOAD segment into the reserved address range by copying its bytes.
///
/// The segment is first mapped as anonymous read-write memory, the file bytes are copied in,
/// and then the mapping is switched to the segment's own protection. Copying (rather than
/// mapping a file) means later changes to the source bytes cannot change the loaded code, and it
/// works where file-backed executable mappings are not allowed. Anonymous pages start zeroed,
/// so the `memsz > filesz` tail needs no separate handling.
///
/// The caller guarantees that `base_offset + seg.vaddr` is page-aligned and that the
/// page-rounded segment lies inside the address range it reserved.
#[expect(
    unsafe_code,
    reason = "Copies into and re-protects memory this loader just mapped; see the SAFETY comments."
)]
fn map_segment(
    elf_bytes: &[u8],
    base_offset: usize,
    seg: &LoadSegment,
    page_size: usize,
) -> Result<(), LoaderError> {
    let target_addr = checked_add_usize(base_offset, seg.vaddr, "segment_target")?;
    let total_len = align_up_page(seg.memsz, page_size, "segment_total_len")?;
    if total_len == 0 {
        return Ok(());
    }

    // Stage 1: replace the reserved (PROT_NONE) range with R+W anonymous pages.
    safe_mmap_anon_segment(
        target_addr as *mut c_void,
        total_len,
        ProtFlags::READ | ProtFlags::WRITE,
        seg.vaddr,
    )?;

    // Stage 2: copy the segment's file bytes.
    if seg.filesz > 0 {
        let src = checked_add_usize(seg.offset, seg.filesz, "segment_file_end")
            .ok()
            .and_then(|file_end| elf_bytes.get(seg.offset..file_end))
            .ok_or(LoaderError::SegmentFileRangeOutOfBounds {
                offset: seg.offset,
                filesz: seg.filesz,
                file_size_in_bytes: elf_bytes.len(),
            })?;
        // SAFETY: `target_addr..target_addr + filesz` lies inside the R+W mapping created above,
        // since `filesz <= memsz <= total_len`, and nothing else references that memory yet.
        // `src` is a separate allocation owned by the caller, so the ranges do not overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), target_addr as *mut u8, src.len());
        }
    }

    // Stage 3: switch to the segment's own protection (R+X for .text, R for read-only data).
    if seg.prot != (ProtFlags::READ | ProtFlags::WRITE) {
        // SAFETY: this function mapped the range above, and nothing else references it yet. The
        // flags come from `seg.prot`, which `parse_load_segments` checked is not W+X.
        unsafe {
            mprotect(
                target_addr as *mut c_void,
                total_len,
                prot_to_mprotect(seg.prot),
            )
        }
        .map_err(|e| LoaderError::MmapSegmentFailed {
            vaddr: seg.vaddr,
            error: format!("mprotect: {e}"),
        })?;
    }

    // Stage 4: make the CPU fetch the instructions just written as data.
    if seg.prot.contains(ProtFlags::EXEC) {
        flush_instruction_cache(target_addr, total_len);
    }

    Ok(())
}

/// Makes freshly written code in `[start, start + len)` visible to instruction fetch.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[expect(
    unsafe_code,
    reason = "Calls a libSystem function; see the SAFETY comment."
)]
fn flush_instruction_cache(start: usize, len: usize) {
    // SAFETY: the range is a mapping this loader owns and has just written; the function
    // only performs cache maintenance on it.
    unsafe { sys_icache_invalidate(start as *mut c_void, len) }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[expect(unsafe_code, reason = "See SAFETY comment")]
// SAFETY: libSystem provides `sys_icache_invalidate` on macOS, and this declaration matches
// its C signature. The only caller passes a range this loader has just mapped.
unsafe extern "C" {
    /// Invalidates the instruction cache for a virtual-address range.
    /// Declared in `<libkern/OSCacheControl.h>`; every macOS binary links libSystem, so no
    /// `#[link]` attribute is needed.
    fn sys_icache_invalidate(start: *mut c_void, length_in_bytes: usize);
}

/// Makes freshly written code in `[start, start + len)` visible to instruction fetch.
///
/// AArch64 does not keep the instruction cache coherent with data writes. This is the
/// sequence `__clear_cache` performs: clean the data cache to the point of unification,
/// invalidate the instruction cache, then synchronise.
#[cfg(all(target_arch = "aarch64", not(target_os = "macos")))]
#[expect(
    unsafe_code,
    reason = "Cache-maintenance instructions; see the SAFETY comments."
)]
fn flush_instruction_cache(start: usize, len: usize) {
    use std::arch::asm;
    let ctr_el0: u64;
    // SAFETY: Linux lets user space read CTR_EL0 (it sets SCTLR_EL1.UCT); the read has no
    // side effects.
    unsafe { asm!("mrs {}, ctr_el0", out(reg) ctr_el0, options(nomem, nostack, preserves_flags)) };
    let dcache_line = 4_usize << ((ctr_el0 >> 16) & 0xF);
    let icache_line = 4_usize << (ctr_el0 & 0xF);
    let end = start.saturating_add(len);
    let mut addr = start & !(dcache_line - 1);
    while addr < end {
        // SAFETY: `addr` lies in a mapping this loader owns; cleaning a cache line does not
        // change memory contents.
        unsafe { asm!("dc cvau, {}", in(reg) addr, options(nostack, preserves_flags)) };
        addr = addr.saturating_add(dcache_line);
    }
    // SAFETY: a barrier has no memory-safety preconditions.
    unsafe { asm!("dsb ish", options(nostack, preserves_flags)) };
    addr = start & !(icache_line - 1);
    while addr < end {
        // SAFETY: as above; invalidating instruction-cache lines does not change memory.
        unsafe { asm!("ic ivau, {}", in(reg) addr, options(nostack, preserves_flags)) };
        addr = addr.saturating_add(icache_line);
    }
    // SAFETY: barriers have no memory-safety preconditions.
    unsafe { asm!("dsb ish", "isb", options(nostack, preserves_flags)) };
}

/// Makes freshly written code visible to instruction fetch: x86_64 keeps the instruction
/// cache coherent, so there is nothing to do.
#[cfg(not(target_arch = "aarch64"))]
fn flush_instruction_cache(_start: usize, _len: usize) {}

/// Loads a transpiled library from its ELF bytes.
///
/// Validates the ELF, reserves a contiguous address range, copies each PT_LOAD segment into it
/// with the segment's permissions, and derives the [`LoadedLayout`] from the section headers
/// and `.transpiled.symbols`. The returned [`MmapHandle`] owns the mapping; every
/// pointer in the layout is valid only while it is alive.
///
/// # Errors
/// Returns [`LoaderError`] if the host page size is above 16 KiB, if the ELF is malformed, was
/// not built for this host, carries unknown flags, is trusted while `loader_ctx` is untrusted,
/// or does not map the memory the guest and host access, or if mapping it fails.
#[expect(
    unsafe_code,
    reason = "Reserves address space with mmap; see the SAFETY comment."
)]
pub(crate) fn load_library(
    elf_bytes: &[u8],
    loader_ctx: LoaderContext,
) -> Result<(MmapHandle, LoadedLayout), LoaderError> {
    if elf_bytes.len() > MAX_ELF_SIZE_IN_BYTES {
        return Err(LoaderError::ElfFileTooLarge {
            size_in_bytes: elf_bytes.len(),
            max_in_bytes: MAX_ELF_SIZE_IN_BYTES,
        });
    }
    let page_size = host_page_size()?;

    let elf_info = parse_elf_info(elf_bytes)?;
    if elf_info.trusted && !loader_ctx.is_trusted() {
        return Err(LoaderError::TrustedArtifactInUntrustedContext);
    }
    let branch_byte_size = branch_byte_size_for_arch(elf_info.architecture)?;
    let (segments, vaddr_range) = parse_load_segments(elf_bytes, page_size)?;
    ensure_accessed_ranges_mapped(&segments, &elf_info)?;

    let total_span = checked_sub_usize(vaddr_range.max_end, vaddr_range.min, "total_span")?;
    let total_span_aligned = align_up_page(total_span, page_size, "total_span_aligned")?;
    if total_span_aligned > MAX_MMAP_SPAN_SIZE_IN_BYTES {
        return Err(LoaderError::MmapSpanTooLarge {
            span_in_bytes: total_span_aligned,
            max_in_bytes: MAX_MMAP_SPAN_SIZE_IN_BYTES,
        });
    }

    // SAFETY: an anonymous `PROT_NONE` mapping at an address the kernel chooses only reserves
    // address space; it replaces no existing mapping.
    let base_ptr = unsafe {
        rustix::mm::mmap_anonymous(
            std::ptr::null_mut(),
            total_span_aligned,
            ProtFlags::empty(),
            MapFlags::PRIVATE,
        )
    }
    .map_err(|e| LoaderError::MmapSyscallFailed {
        syscall: "mmap (reserve)".to_string(),
        error: e.to_string(),
    })?;
    let base_guard = MmapRegionGuard {
        ptr: base_ptr,
        len_in_bytes: total_span_aligned,
    };
    let base_offset = checked_sub_usize(base_ptr as usize, vaddr_range.min, "base_offset")?;

    debug!(
        "load_library: base={:#x} min_vaddr={:#x} total_span={:#x} segments={}",
        base_ptr as usize,
        vaddr_range.min,
        total_span_aligned,
        segments.len()
    );

    for seg in &segments {
        map_segment(elf_bytes, base_offset, seg, page_size)?;
    }

    // Resolve the code addresses in `.transpiled.symbols` to host addresses. Each must lie in
    // `.text` and, on aarch64, be 4-byte aligned.
    let text_section = elf_info.sections[SectionId::Text as usize].ok_or_else(|| {
        LoaderError::SectionNotFound {
            section: SectionId::Text.name().to_string(),
        }
    })?;
    let text_vaddr_start = text_section.addr.as_u32();
    let text_vaddr_end = checked_add_u32(text_vaddr_start, text_section.size, "text_end")?;
    let resolve_code_symbol = |name: &str, vaddr: u32| {
        if !(text_vaddr_start..=text_vaddr_end).contains(&vaddr) {
            return Err(LoaderError::ElfParsingError {
                message: format!(
                    "{name} vaddr {vaddr:#x} outside .text \
                     [{text_vaddr_start:#x}..{text_vaddr_end:#x}]"
                ),
            });
        }
        let addr = checked_add_usize(base_offset, vaddr as usize, name)?;
        if elf_info.architecture == object::Architecture::Aarch64
            && !addr.is_multiple_of(N_BYTES_IN_WORD)
        {
            return Err(LoaderError::UnalignedSymbol {
                symbol: name.to_string(),
            });
        }
        Ok(addr)
    };
    let symbols = &elf_info.symbols;
    // The other symbols may mark the end of `.text`, but the code entry must be inside it.
    if symbols.start_transpiled_code >= text_vaddr_end {
        return Err(LoaderError::ElfParsingError {
            message: format!(
                "start_transpiled_code vaddr {:#x} is not inside .text \
                 [{text_vaddr_start:#x}..{text_vaddr_end:#x})",
                symbols.start_transpiled_code
            ),
        });
    }
    resolve_code_symbol("start_transpiled_code", symbols.start_transpiled_code)?;
    let entry_points_begin_addr =
        resolve_code_symbol("entry_points_hooks_begin", symbols.entry_points_hooks_begin)?;
    let init_array_end_addr = resolve_code_symbol("init_array_end", symbols.init_array_end)?;
    let entry_points_end_addr =
        resolve_code_symbol("entry_points_hooks_end", symbols.entry_points_hooks_end)?;

    debug!(
        "load_library: hooks_begin={entry_points_begin_addr:#x} \
         init_array_end={init_array_end_addr:#x} hooks_end={entry_points_end_addr:#x}",
    );

    let layout = build_loaded_layout(
        base_offset,
        entry_points_begin_addr,
        init_array_end_addr,
        entry_points_end_addr,
        &elf_info,
        branch_byte_size,
    )?;

    let (base, total_len_in_bytes) = base_guard.disarm();
    let handle = MmapHandle {
        base,
        total_len_in_bytes,
    };

    Ok((handle, layout))
}
