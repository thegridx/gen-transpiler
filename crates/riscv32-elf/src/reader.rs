//! Reads and validates RISC-V ELF files for the transpiler.
use log::debug;
use object::elf::SHF_ALLOC;
use object::{
    FileFlags, Object, ObjectKind, ObjectSection, ObjectSymbol, Section, SectionFlags, SectionKind,
    SymbolKind, SymbolSection,
};
use riscv32::N_BYTES_IN_WORD;
use riscv32::common::{check_alignment, checked_add_u32, checked_u64_to_u32};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

use crate::consts::{
    ECALL_DATA_SECTION_NAME, HEAP_SECTION_NAME, INIT_ARRAY_SECTION_NAME, STACK_SECTION_NAME,
    TEXT_SECTION_NAME,
};
use crate::consts::{
    SYM_DATA_END_VALUE, SYM_DATA_START_VALUE, SYM_STACK_TOP_VALUE, SYM_TEXT_END_VALUE,
};
use crate::{
    consts::{MAX_NUM_OF_ENTRY_POINTS, MAX_NUM_OF_INIT_FUNCTIONS},
    types::{EntryPointMapping, RiscvAddress, RvSymbolValues},
};

/// The largest `.text` that can transpile. The output image (at most 8 MiB) holds the original
/// code (n bytes), its jump table (2n bytes) and at least n bytes of native code, so n is at
/// most 2 MiB.
pub(crate) const RISCV_MAX_CODE_SIZE_IN_BYTES: u32 = 0x20_0000; // 2 MiB

/// Every guest address must be below this. The aarch64 bounds checks encode their constants as
/// two 12-bit immediates (the high one shifted by 12), which reach just under 16 MiB.
pub(crate) const MAX_GUEST_ADDRESS_END: u32 = 0x100_0000; // 16 MiB

// `e_flags` bits of the RISC-V ELF psABI that select features the transpiler does not support.
const EF_RISCV_RVC: u32 = 0x0001;
const EF_RISCV_FLOAT_ABI: u32 = 0x0006;
const EF_RISCV_RVE: u32 = 0x0008;

/// Errors from ELF parsing and validation.
#[derive(Error, Debug)]
pub enum RvElfParseError {
    #[error("ELF object parsing error: {0}")]
    ObjectError(String),
    #[error("wrong binary format: {bin_format}")]
    WrongBinaryFormat { bin_format: String },
    #[error("wrong architecture: {arch}")]
    WrongArchitecture { arch: String },
    #[error("wrong endianness")]
    WrongEndianness,
    #[error("missing sections: {sections:?}")]
    MissingSections { sections: Vec<String> },
    #[error("multiple sections with name '{section}' found")]
    MultipleSection { section: String },
    #[error("section '{section}' data length ({len} bytes) is not a multiple of {word_size} bytes")]
    SectionDataNotWordAligned {
        section: String,
        len: usize,
        word_size: usize,
    },
    #[error("code section too big: {0}")]
    CodeSectionTooLarge(u32),
    #[error(
        "unsupported allocatable section '{section}' ({kind}); link with riscv_elf_mem_layout.x"
    )]
    UnsupportedSection { section: String, kind: String },
    #[error("section {section} holds data, but the runtime builds it empty")]
    InitializedRuntimeSection { section: String },
    #[error("expected a linked executable (ET_EXEC), not {kind}")]
    NotAnExecutable { kind: String },
    #[error("unsupported ELF flags {e_flags:#x}: {reason}")]
    UnsupportedElfFlags { e_flags: u32, reason: String },
    #[error("entry point '{name}' must be a defined, global function symbol in the code section")]
    BadEntryPointSymbol { name: String },
    #[error("more than one symbol named '{name}'")]
    DuplicateSymbol { name: String },
    #[error(
        "layout symbols must satisfy data_start <= stack_top <= data_end <= {max:#x}; got \
         data_start={data_start:#x}, stack_top={stack_top:#x}, data_end={data_end:#x}"
    )]
    BadLayoutSymbols {
        data_start: u32,
        stack_top: u32,
        data_end: u32,
        max: u32,
    },
    #[error("code section must start at address 0, not {address:#x}")]
    CodeSectionNotAtZero { address: u32 },
    #[error("{SYM_TEXT_END_VALUE} is {text_end:#x} but the code section ends at {code_end:#x}")]
    TextEndMismatch { text_end: u32, code_end: u32 },
    #[error("bad symbol kind for '{name}'")]
    BadSymbolType { name: String },
    #[error("too many init functions: {count} (max {max})")]
    TooManyInitFunctions { count: usize, max: usize },
    #[error("too many entry points: {count} (max {max})")]
    TooManyEntryPoints { count: usize, max: usize },
    #[error("missing entry points: {entry_points:?}")]
    MissingEntryPoints { entry_points: Vec<String> },
    #[error("missing symbol: {name}")]
    MissingSymbol { name: String },
    #[error("bad callable address: {address:#x} (code section: {code_start:#x} - {code_end:#x})")]
    BadCallableAddress {
        address: u32,
        code_start: u32,
        code_end: u32,
    },
    #[error("value error: {0}")]
    ValueError(#[from] riscv32::common::ValueError),
}

pub(crate) type RvSymbolMap = HashMap<String, RiscvAddress>;

#[derive(Default, Debug)]
pub struct RvEntryPoints {
    init_array: Vec<RiscvAddress>,
    entry_point_addrs: Vec<RiscvAddress>,
    /// Entry point name → assigned index for all entry points
    name_to_index: EntryPointMapping,
}
impl RvEntryPoints {
    /// Creates a new `RvEntryPoints` from the given init array and entry point addresses, assigning each a unique index (starting from 1).
    /// The init array functions are indexed first, followed by the entry points sorted alphabetically by name.
    pub(crate) fn new(
        init_array: Vec<RiscvAddress>,
        entry_points: RvSymbolMap,
    ) -> Result<Self, RvElfParseError> {
        // Index is 1-based since 0 is reserved for "ecall return" indicator
        let mut cur_index = 1_u32;
        for init_func_addr in init_array.iter() {
            let init_func_idx = cur_index;
            cur_index = checked_add_u32(cur_index, 1, "init array index")?;
            debug!(
                "[{init_func_idx}] init function address: {:#x}",
                init_func_addr.as_u32()
            );
        }
        // Entry point order is determined by their sorted names
        let mut entry_point_addrs = Vec::new();
        let mut name_to_index = EntryPointMapping::new();
        let mut entry_point_items = entry_points.into_iter().collect::<Vec<_>>();
        entry_point_items.sort_by_key(|(name, _)| name.clone());
        for (entry_point_name, entry_point_addr) in entry_point_items.iter() {
            let entry_point_idx = cur_index;
            cur_index = checked_add_u32(cur_index, 1, "entry point index")?;
            debug!(
                "[{entry_point_idx}] entry point '{entry_point_name}' address: {:#x}",
                entry_point_addr.as_u32()
            );
            name_to_index.insert((*entry_point_name).clone(), entry_point_idx);
            entry_point_addrs.push(*entry_point_addr);
        }
        Ok(Self {
            init_array,
            entry_point_addrs,
            name_to_index,
        })
    }
    pub fn init_array_addrs(&self) -> &[RiscvAddress] {
        &self.init_array
    }
    pub fn entry_point_addrs(&self) -> &[RiscvAddress] {
        &self.entry_point_addrs
    }
    pub fn name_to_index(&self) -> &EntryPointMapping {
        &self.name_to_index
    }
    #[cfg(test)]
    pub(crate) fn index_of(&self, entry_point_name: &str) -> Option<u32> {
        self.name_to_index.get(entry_point_name).copied()
    }
    #[cfg(test)]
    pub(crate) fn address_of(&self, index: u32) -> Option<RiscvAddress> {
        let position = usize::try_from(index.checked_sub(1)?).ok()?;
        self.init_array
            .iter()
            .chain(&self.entry_point_addrs)
            .nth(position)
            .copied()
    }
}

pub struct RvElfSection {
    name: String,
    kind: SectionKind,
    alignment: u32, // Power of 2
    address: RiscvAddress,
    // Size of the section (for nobits, this is the allocated size)
    size_in_bytes: u32,
    // Raw section data (empty for nobits/UninitializedData sections)
    data: Vec<u8>,
}
impl RvElfSection {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn kind(&self) -> SectionKind {
        self.kind
    }
    pub fn alignment(&self) -> u32 {
        self.alignment
    }
    pub fn address(&self) -> RiscvAddress {
        self.address
    }
    pub fn size_in_bytes(&self) -> u32 {
        self.size_in_bytes
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

pub struct RiscvElf {
    entry_points: RvEntryPoints,
    symbol_values: RvSymbolValues,
    code_section: RvElfSection,
    data_sections: Vec<RvElfSection>,
}
impl RiscvElf {
    pub fn entry_points(&self) -> &RvEntryPoints {
        &self.entry_points
    }
    pub fn symbol_values(&self) -> &RvSymbolValues {
        &self.symbol_values
    }
    pub fn code_section(&self) -> &RvElfSection {
        &self.code_section
    }
    pub fn code_section_as_i32_vec(&self) -> Result<Vec<i32>, RvElfParseError> {
        bytes_to_i32_vec(self.code_section.data())
    }
    pub fn data_sections(&self) -> &[RvElfSection] {
        &self.data_sections
    }
    /// Returns the sections with names in `section_names` if they exist
    #[cfg(test)]
    pub(crate) fn get_sections(&self, section_names: &[&str]) -> Vec<&RvElfSection> {
        let mut res = Vec::new();
        if section_names.contains(&self.code_section.name()) {
            res.push(&self.code_section);
        }
        for data_section in &self.data_sections {
            if section_names.contains(&data_section.name()) {
                res.push(data_section);
            }
        }
        res
    }
    /// Returns each section's name mapped to its `(address, size)`.
    #[cfg(test)]
    pub(crate) fn section_addresses_and_sizes(&self) -> HashMap<String, (u32, u32)> {
        let mut res = HashMap::new();
        res.insert(
            TEXT_SECTION_NAME.to_string(),
            (0, self.code_section.size_in_bytes()),
        );
        for section in &self.data_sections {
            res.insert(
                section.name().to_string(),
                (section.address().as_u32(), section.size_in_bytes()),
            );
        }
        res
    }
    pub fn stack_size_in_bytes(&self) -> u32 {
        self.data_sections
            .iter()
            .find(|section| section.name() == STACK_SECTION_NAME)
            .map_or(0, |section| section.size_in_bytes())
    }
    pub fn heap_size_in_bytes(&self) -> u32 {
        self.data_sections
            .iter()
            .find(|section| section.name() == HEAP_SECTION_NAME)
            .map_or(0, |section| section.size_in_bytes())
    }
}

fn get_section_name<'a>(section: &Section<'a, '_>) -> Result<&'a str, RvElfParseError> {
    section.name().map_err(|e| {
        RvElfParseError::ObjectError(format!(
            "Error getting section name for section with index {}: {e}",
            section.index()
        ))
    })
}

fn is_allocatable(section: &Section<'_, '_>) -> bool {
    matches!(section.flags(), SectionFlags::Elf { sh_flags } if sh_flags & u64::from(SHF_ALLOC) != 0)
}

fn get_section_data(section: &Section<'_, '_>) -> Result<Vec<u8>, RvElfParseError> {
    section.data().map(|data| data.to_vec()).map_err(|e| {
        RvElfParseError::ObjectError(format!(
            "Error getting data for section '{}': {e}",
            get_section_name(section).unwrap_or("<unknown>")
        ))
    })
}

fn get_symbol_name<'a>(symbol: &object::Symbol<'a, '_>) -> Result<&'a str, RvElfParseError> {
    symbol.name().map_err(|e| {
        RvElfParseError::ObjectError(format!(
            "Error getting symbol name for symbol with index {}: {e}",
            symbol.index()
        ))
    })
}

fn get_symbol_address(
    symbol: &object::Symbol<'_, '_>,
    name: &str,
) -> Result<RiscvAddress, RvElfParseError> {
    debug!("Symbol {name}: {:#x}", symbol.address());
    Ok(RiscvAddress::new(checked_u64_to_u32(
        symbol.address(),
        name,
    )?))
}

fn get_global_symbol_address(
    symbol: &object::Symbol<'_, '_>,
    name: &str,
) -> Result<RiscvAddress, RvElfParseError> {
    if symbol.kind() != SymbolKind::Unknown
        || !symbol.is_global()
        || !matches!(symbol.section(), SymbolSection::Absolute)
    {
        return Err(RvElfParseError::BadSymbolType {
            name: name.to_string(),
        });
    }
    get_symbol_address(symbol, name)
}

fn validate_callable_address(
    code_section: &RvElfSection,
    address: RiscvAddress,
) -> Result<(), RvElfParseError> {
    let code_start = code_section.address();
    let code_end = code_start.checked_add(code_section.size_in_bytes(), "code end")?;
    if (address.as_u32() & (N_BYTES_IN_WORD as u32 - 1)) != 0
        || address < code_start
        || address >= code_end
    {
        return Err(RvElfParseError::BadCallableAddress {
            address: address.as_u32(),
            code_start: code_start.as_u32(),
            code_end: code_end.as_u32(),
        });
    }
    Ok(())
}

fn load_symbols(
    obj_file: &object::File<'_>,
    entry_point_names: &HashSet<String>,
    code_section: &RvElfSection,
) -> Result<(RvSymbolMap, RvSymbolValues), RvElfParseError> {
    if code_section.size_in_bytes() > RISCV_MAX_CODE_SIZE_IN_BYTES {
        return Err(RvElfParseError::CodeSectionTooLarge(
            code_section.size_in_bytes(),
        ));
    }
    let mut entry_points = HashMap::new();
    let mut text_end_sym: Option<RiscvAddress> = None;
    let mut data_start_sym: Option<RiscvAddress> = None;
    let mut stack_top_sym: Option<RiscvAddress> = None;
    let mut data_end_sym: Option<RiscvAddress> = None;
    for symbol in obj_file.symbols() {
        match get_symbol_name(&symbol)? {
            // A local symbol is private to its object file, so it never names a layout value,
            // even when it shares a layout symbol's name.
            SYM_TEXT_END_VALUE | SYM_DATA_START_VALUE | SYM_STACK_TOP_VALUE
            | SYM_DATA_END_VALUE
                if !symbol.is_global() => {}
            SYM_TEXT_END_VALUE => {
                text_end_sym = Some(get_global_symbol_address(&symbol, SYM_TEXT_END_VALUE)?);
            }
            SYM_DATA_START_VALUE => {
                data_start_sym = Some(get_global_symbol_address(&symbol, SYM_DATA_START_VALUE)?);
            }
            SYM_STACK_TOP_VALUE => {
                stack_top_sym = Some(get_global_symbol_address(&symbol, SYM_STACK_TOP_VALUE)?);
            }
            SYM_DATA_END_VALUE => {
                data_end_sym = Some(get_global_symbol_address(&symbol, SYM_DATA_END_VALUE)?);
            }
            sym_name => {
                // A local symbol is private to its object file, so it never names an entry
                // point, even when another file's global has the same name.
                if entry_point_names.contains(sym_name) && symbol.is_global() {
                    let entry_point_name = sym_name.to_string();
                    // A function or an assembly label (no type), defined in a section, so the
                    // name identifies one place in the code.
                    if !matches!(symbol.kind(), SymbolKind::Text | SymbolKind::Unknown)
                        || !matches!(symbol.section(), SymbolSection::Section(_))
                    {
                        return Err(RvElfParseError::BadEntryPointSymbol {
                            name: entry_point_name,
                        });
                    }
                    if entry_points.contains_key(&entry_point_name) {
                        return Err(RvElfParseError::DuplicateSymbol {
                            name: entry_point_name,
                        });
                    }
                    let entry_point_addr = get_symbol_address(&symbol, sym_name)?;
                    validate_callable_address(code_section, entry_point_addr)?;
                    debug!(
                        "Entry point {entry_point_name}: {:#x}",
                        entry_point_addr.as_u32()
                    );
                    entry_points.insert(entry_point_name, entry_point_addr);
                }
            }
        }
    }
    // Validate entry points
    if entry_points.len() > MAX_NUM_OF_ENTRY_POINTS {
        return Err(RvElfParseError::TooManyEntryPoints {
            count: entry_points.len(),
            max: MAX_NUM_OF_ENTRY_POINTS,
        });
    }
    let entry_points_diff = entry_point_names
        .difference(&entry_points.keys().cloned().collect::<HashSet<String>>())
        .cloned()
        .collect::<Vec<String>>();
    if !entry_points_diff.is_empty() {
        return Err(RvElfParseError::MissingEntryPoints {
            entry_points: entry_points_diff,
        });
    }
    // Derive the remaining optional symbol from section data.
    let text_end = match text_end_sym {
        Some(addr) => addr,
        None => {
            // _text_end_value = end of .text section
            let addr = code_section
                .address()
                .checked_add(code_section.size_in_bytes(), "text end")?;
            debug!(
                "{SYM_TEXT_END_VALUE} not found, derived from .text section: {:#x}",
                addr.as_u32()
            );
            addr
        }
    };
    if text_end.as_u32() == 0 {
        return Err(RvElfParseError::MissingSections {
            sections: vec![TEXT_SECTION_NAME.to_string()],
        });
    }
    let data_start = data_start_sym.ok_or_else(|| RvElfParseError::MissingSymbol {
        name: SYM_DATA_START_VALUE.to_string(),
    })?;
    let stack_top = stack_top_sym.ok_or_else(|| RvElfParseError::MissingSymbol {
        name: SYM_STACK_TOP_VALUE.to_string(),
    })?;
    let data_end = data_end_sym.ok_or_else(|| RvElfParseError::MissingSymbol {
        name: SYM_DATA_END_VALUE.to_string(),
    })?;
    // Validate required symbols
    let memory_addresses = RvSymbolValues {
        text_end,
        data_start,
        stack_top,
        data_end,
    };
    Ok((entry_points, memory_addresses))
}

// By convention, RISC-V is "default to signed integer"
fn bytes_to_i32_vec(code_bytes: &[u8]) -> Result<Vec<i32>, RvElfParseError> {
    if !code_bytes.len().is_multiple_of(N_BYTES_IN_WORD) {
        return Err(RvElfParseError::SectionDataNotWordAligned {
            section: TEXT_SECTION_NAME.to_string(),
            len: code_bytes.len(),
            word_size: N_BYTES_IN_WORD,
        });
    }
    let (chunks, _) = code_bytes.as_chunks::<N_BYTES_IN_WORD>();
    Ok(chunks
        .iter()
        .map(|chunk| i32::from_le_bytes(*chunk))
        .collect())
}

fn load_section(section: Section<'_, '_>) -> Result<RvElfSection, RvElfParseError> {
    let name = get_section_name(&section)?.to_string();
    let alignment_u64 = if section.align() == 0 {
        1
    } else {
        section.align()
    };
    let alignment = checked_u64_to_u32(alignment_u64, &format!("{name} alignment"))?;
    check_alignment(alignment, &format!("{name} alignment"))?;
    let address = RiscvAddress::new(checked_u64_to_u32(
        section.address(),
        &format!("{name} address"),
    )?);
    let size_in_bytes = checked_u64_to_u32(section.size(), &format!("{name} size"))?;
    Ok(RvElfSection {
        name: name.clone(),
        kind: section.kind(),
        alignment,
        address,
        size_in_bytes,
        data: get_section_data(&section)?,
    })
}

fn parse_init_array(data: &[u8]) -> Result<Vec<RiscvAddress>, RvElfParseError> {
    if !data.len().is_multiple_of(N_BYTES_IN_WORD) {
        return Err(RvElfParseError::SectionDataNotWordAligned {
            section: INIT_ARRAY_SECTION_NAME.to_string(),
            len: data.len(),
            word_size: N_BYTES_IN_WORD,
        });
    }
    let (chunks, _) = data.as_chunks::<N_BYTES_IN_WORD>();
    let addresses = chunks
        .iter()
        .map(|chunk| RiscvAddress::new(u32::from_le_bytes(*chunk)))
        .collect::<Vec<_>>();
    if addresses.len() > MAX_NUM_OF_INIT_FUNCTIONS {
        return Err(RvElfParseError::TooManyInitFunctions {
            count: addresses.len(),
            max: MAX_NUM_OF_INIT_FUNCTIONS,
        });
    }
    debug!(
        "Read {} init function addresses from {}",
        addresses.len(),
        INIT_ARRAY_SECTION_NAME
    );
    Ok(addresses)
}

/// Reads a RISC-V ELF from raw bytes, validates its format, architecture, endianness, and required sections,
/// and parses its code, data sections, and symbols into a `RiscvElf` struct for use by the transpiler.
///
/// # Arguments
/// * `file_data` - Raw bytes of the ELF file to be read and parsed.
/// * `entry_point_names` - Set of expected entry point symbol names that must be present in the ELF's symbol table
///
/// # Returns
/// * `Ok(RiscvElf)` - The parsed and validated ELF data, including code, entry points, symbol values, and data sections.
///
/// # Errors
/// Returns a `RvElfParseError` if:
/// - the file is not a little-endian, linked RV32 ELF executable without the C or E extensions;
/// - it has more than one `.text` or `.init_array`, an unsupported allocatable section, or a
///   `.stack`, `.heap` or `.ecall_data` that is not a single NOBITS section;
/// - a layout symbol (`_text_end_value`, `_data_start_value`, `_stack_top_value`,
///   `_data_end_value`) is missing, not global and absolute, or out of order;
/// - a named entry point is missing, duplicated or not a global symbol in `.text`, or there are
///   too many entry points or init functions;
/// - an entry point or init function is misaligned or outside the code;
/// - a value, size or address does not fit in `u32`, or a section's alignment is invalid.
pub fn read_riscv_elf(
    file_data: &[u8],
    entry_point_names: &HashSet<String>,
) -> Result<RiscvElf, RvElfParseError> {
    let obj_file =
        object::File::parse(file_data).map_err(|e| RvElfParseError::ObjectError(format!("{e}")))?;
    // Validate
    if obj_file.format() != object::BinaryFormat::Elf {
        return Err(RvElfParseError::WrongBinaryFormat {
            bin_format: format!("{:?}", obj_file.format()),
        });
    }
    if obj_file.architecture() != object::Architecture::Riscv32 {
        return Err(RvElfParseError::WrongArchitecture {
            arch: format!("{:?}", obj_file.architecture()),
        });
    }
    if obj_file.endianness() == object::Endianness::Big {
        return Err(RvElfParseError::WrongEndianness);
    }
    if obj_file.kind() != ObjectKind::Executable {
        return Err(RvElfParseError::NotAnExecutable {
            kind: format!("{:?}", obj_file.kind()),
        });
    }
    if let FileFlags::Elf { e_flags, .. } = obj_file.flags() {
        let unsupported = [
            (EF_RISCV_RVC, "compressed instructions (C extension)"),
            (EF_RISCV_FLOAT_ABI, "a hard-float ABI (use ilp32)"),
            (EF_RISCV_RVE, "the RV32E base ISA"),
        ];
        if let Some((_, reason)) = unsupported.iter().find(|(bits, _)| e_flags & bits != 0) {
            return Err(RvElfParseError::UnsupportedElfFlags {
                e_flags,
                reason: (*reason).to_string(),
            });
        }
    }
    // Parse sections
    let mut code_section: Option<RvElfSection> = None;
    let mut init_array_section: Option<Vec<RiscvAddress>> = None;
    let mut data_sections: Vec<RvElfSection> = Vec::new();
    for section in obj_file.sections() {
        let section_name = get_section_name(&section)?;
        debug!(
            "Section: Name: {section_name}, Address: 0x{:x}, Size: 0x{:x} Alignment: 0x{:x} Kind: {:?}",
            section.address(),
            section.size(),
            section.align(),
            section.kind()
        );
        if section_name == INIT_ARRAY_SECTION_NAME {
            if init_array_section.is_some() {
                return Err(RvElfParseError::MultipleSection {
                    section: INIT_ARRAY_SECTION_NAME.to_string(),
                });
            }
            init_array_section = Some(parse_init_array(&get_section_data(&section)?)?);
            continue;
        }
        match section.kind() {
            SectionKind::Text => {
                if code_section.is_some() {
                    return Err(RvElfParseError::MultipleSection {
                        section: TEXT_SECTION_NAME.to_string(),
                    });
                }
                code_section = Some(load_section(section)?);
            }
            SectionKind::ReadOnlyData
            | SectionKind::ReadOnlyString
            | SectionKind::Data
            | SectionKind::UninitializedData
                if is_allocatable(&section) =>
            {
                // The runtime builds `.stack`, `.heap` and `.ecall_data` empty, so each must be a
                // single NOBITS section. An object that puts data in one of them gets a second
                // section of that name from the linker, whose bytes the output would drop.
                if matches!(
                    section_name,
                    STACK_SECTION_NAME | HEAP_SECTION_NAME | ECALL_DATA_SECTION_NAME
                ) {
                    if section.kind() != SectionKind::UninitializedData {
                        return Err(RvElfParseError::InitializedRuntimeSection {
                            section: section_name.to_string(),
                        });
                    }
                    if data_sections.iter().any(|s| s.name() == section_name) {
                        return Err(RvElfParseError::MultipleSection {
                            section: section_name.to_string(),
                        });
                    }
                }
                data_sections.push(load_section(section)?);
            }
            // The guest could reference any other allocatable section (TLS data, `.fini_array`,
            // notes, ...), but the output has nowhere to put it.
            kind if is_allocatable(&section) => {
                return Err(RvElfParseError::UnsupportedSection {
                    section: section_name.to_string(),
                    kind: format!("{kind:?}"),
                });
            }
            _ => {} // Skip non-allocatable sections such as debug info
        }
    }
    let Some(actual_code_section) = code_section else {
        return Err(RvElfParseError::MissingSections {
            sections: vec![TEXT_SECTION_NAME.to_string()],
        });
    };
    // RISC-V addresses double as instruction indices (address / 4) throughout the transpiler,
    // and the output places the code at guest address 0.
    if actual_code_section.address().as_u32() != 0 {
        return Err(RvElfParseError::CodeSectionNotAtZero {
            address: actual_code_section.address().as_u32(),
        });
    }
    // Parse symbols and update from init array
    let (entry_points_to_addr, memory_addresses) =
        load_symbols(&obj_file, entry_point_names, &actual_code_section)?;
    let (data_start, stack_top, data_end) = (
        memory_addresses.data_start.as_u32(),
        memory_addresses.stack_top.as_u32(),
        memory_addresses.data_end.as_u32(),
    );
    if !(data_start <= stack_top && stack_top <= data_end && data_end <= MAX_GUEST_ADDRESS_END) {
        return Err(RvElfParseError::BadLayoutSymbols {
            data_start,
            stack_top,
            data_end,
            max: MAX_GUEST_ADDRESS_END,
        });
    }
    // Branch targets are bounded by `_text_end_value`, so it must match the code that is
    // actually there before any target is used as an instruction index.
    if memory_addresses.text_end.as_u32() != actual_code_section.size_in_bytes() {
        return Err(RvElfParseError::TextEndMismatch {
            text_end: memory_addresses.text_end.as_u32(),
            code_end: actual_code_section.size_in_bytes(),
        });
    }
    let init_array = init_array_section.unwrap_or_default();
    for address in &init_array {
        validate_callable_address(&actual_code_section, *address)?;
    }
    let entry_points = RvEntryPoints::new(init_array, entry_points_to_addr)?;
    Ok(RiscvElf {
        entry_points,
        symbol_values: memory_addresses,
        code_section: actual_code_section,
        data_sections,
    })
}

#[cfg(test)]
mod tests {
    use crate::consts::{DATA_SECTION_NAME, RODATA_SECTION_NAME};
    use googletest::prelude::{assert_that, is_empty};

    use crate::types::{VirtualDataPointer, VirtualDataRange, VirtualDataSize};

    use super::*;

    // ── bytes_to_i32_vec ──────────────────────────────────────────────────────

    #[test]
    fn bytes_to_i32_vec_empty_slice_returns_empty_vec() {
        assert_eq!(bytes_to_i32_vec(&[]).unwrap(), Vec::<i32>::new());
    }

    #[test]
    fn bytes_to_i32_vec_non_multiple_of_4_returns_error() {
        for len in [1usize, 2, 3, 5, 7] {
            let bytes = vec![0u8; len];
            assert!(
                matches!(
                    bytes_to_i32_vec(&bytes),
                    Err(RvElfParseError::SectionDataNotWordAligned { .. })
                ),
                "expected SectionDataNotWordAligned for len={len}"
            );
        }
    }

    #[test]
    fn bytes_to_i32_vec_parses_single_word_little_endian() {
        assert_eq!(bytes_to_i32_vec(&[1, 0, 0, 0]).unwrap(), vec![1]);
        assert_eq!(bytes_to_i32_vec(&[0, 1, 0, 0]).unwrap(), vec![256]);
        assert_eq!(bytes_to_i32_vec(&[0, 0, 1, 0]).unwrap(), vec![65536]);
    }

    #[test]
    fn bytes_to_i32_vec_all_ones_parses_as_negative_one() {
        assert_eq!(
            bytes_to_i32_vec(&[0xFF, 0xFF, 0xFF, 0xFF]).unwrap(),
            vec![-1]
        );
    }

    #[test]
    fn bytes_to_i32_vec_parses_i32_extremes() {
        // i32::MAX = 0x7FFF_FFFF
        assert_eq!(
            bytes_to_i32_vec(&[0xFF, 0xFF, 0xFF, 0x7F]).unwrap(),
            vec![i32::MAX]
        );
        // i32::MIN = 0x8000_0000
        assert_eq!(
            bytes_to_i32_vec(&[0x00, 0x00, 0x00, 0x80]).unwrap(),
            vec![i32::MIN]
        );
    }

    #[test]
    fn bytes_to_i32_vec_parses_multiple_words() {
        let bytes = [1u8, 0, 0, 0, 2, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(bytes_to_i32_vec(&bytes).unwrap(), vec![1, 2, -1]);
    }

    // ── RiscvAddress ──────────────────────────────────────────────────────────

    #[test]
    fn risc_v_address_add_basic() {
        assert_eq!(RiscvAddress::new(0x100).wrapping_add(0x10).as_u32(), 0x110);
        assert_eq!(RiscvAddress::new(0).wrapping_add(0).as_u32(), 0);
    }

    #[test]
    fn risc_v_address_add_wraps_at_u32_max() {
        assert_eq!(RiscvAddress::new(u32::MAX).wrapping_add(1).as_u32(), 0);
        assert_eq!(RiscvAddress::new(u32::MAX).wrapping_add(4).as_u32(), 3);
    }

    #[test]
    fn risc_v_address_index_divides_by_word_size() {
        assert_eq!(RiscvAddress::new(0).index(), 0);
        assert_eq!(RiscvAddress::new(4).index(), 1);
        assert_eq!(RiscvAddress::new(8).index(), 2);
        assert_eq!(RiscvAddress::new(100).index(), 25);
    }

    #[test]
    fn risc_v_address_index_truncates_misaligned_address() {
        // index() does integer division; misaligned addresses lose the remainder
        assert_eq!(RiscvAddress::new(5).index(), 1);
        assert_eq!(RiscvAddress::new(7).index(), 1);
    }

    #[test]
    fn risc_v_address_checked_add_ok_and_overflow() {
        assert_eq!(
            RiscvAddress::new(0x100)
                .checked_add(0x10, "test")
                .unwrap()
                .as_u32(),
            0x110
        );
        assert_eq!(
            RiscvAddress::new(u32::MAX - 1)
                .checked_add(1, "test")
                .unwrap()
                .as_u32(),
            u32::MAX
        );
        assert!(RiscvAddress::new(u32::MAX).checked_add(1, "test").is_err());
        assert!(
            RiscvAddress::new(u32::MAX - 1)
                .checked_add(2, "test")
                .is_err()
        );
    }

    #[test]
    fn risc_v_address_as_i32_reinterprets_high_bit_as_negative() {
        assert_eq!(RiscvAddress::new(0).as_i32(), 0);
        assert_eq!(RiscvAddress::new(1).as_i32(), 1);
        assert_eq!(RiscvAddress::new(0x8000_0000).as_i32(), i32::MIN);
        assert_eq!(RiscvAddress::new(u32::MAX).as_i32(), -1);
    }

    // ── VirtualDataPointer / VirtualDataSize / VirtualDataRange ───────────────

    #[test]
    fn virtual_data_pointer_conversions() {
        let p = VirtualDataPointer::new(0x8000_0000);
        assert_eq!(p.as_u32(), 0x8000_0000);
        assert_eq!(p.as_usize(), 0x8000_0000usize);
        assert_eq!(p.as_i32(), i32::MIN);
    }

    #[test]
    fn virtual_data_pointer_checked_add_ok_and_overflow() {
        let p = VirtualDataPointer::new(10);
        let sum = p.checked_add(VirtualDataSize::new(32), "test").unwrap();
        assert_eq!(sum.as_u32(), 42);

        let p_max = VirtualDataPointer::new(u32::MAX);
        assert!(p_max.checked_add(VirtualDataSize::new(1), "test").is_err());
    }

    #[test]
    fn virtual_data_size_as_usize() {
        assert_eq!(VirtualDataSize::new(0).as_usize(), 0);
        assert_eq!(VirtualDataSize::new(42).as_usize(), 42);
        assert_eq!(VirtualDataSize::new(u32::MAX).as_usize(), u32::MAX as usize);
    }

    #[test]
    fn virtual_data_range_size_and_endpoints() {
        let start = VirtualDataPointer::new(0x100);
        let end = VirtualDataPointer::new(0x180);
        let range = VirtualDataRange::new(start, end);
        assert_eq!(range.start().as_u32(), 0x100);
        assert_eq!(range.end().as_u32(), 0x180);
        assert_eq!(range.size().as_u32(), 0x80);
    }

    #[test]
    fn virtual_data_range_size_saturates_when_end_before_start() {
        // Defensive: if misconstructed, size() should not panic or wrap.
        let range = VirtualDataRange::new(
            VirtualDataPointer::new(0x200),
            VirtualDataPointer::new(0x100),
        );
        assert_eq!(range.size().as_u32(), 0);
    }

    // ── parse_init_array ──────────────────────────────────────────────────────

    #[test]
    fn parse_init_array_empty_returns_empty_vec() {
        let addrs = parse_init_array(&[]).unwrap();
        assert_that!(addrs, is_empty());
    }

    #[test]
    fn parse_init_array_non_word_aligned_returns_error() {
        for len in [1usize, 2, 3, 5, 6, 7] {
            let bytes = vec![0u8; len];
            assert!(
                matches!(
                    parse_init_array(&bytes),
                    Err(RvElfParseError::SectionDataNotWordAligned {
                        section,
                        len: got_len,
                        word_size: 4,
                    }) if section == INIT_ARRAY_SECTION_NAME && got_len == len
                ),
                "expected SectionDataNotWordAligned for len={len}"
            );
        }
    }

    #[test]
    fn parse_init_array_parses_addresses_little_endian() {
        // Two addresses: 0x04 and 0xDEAD_BEEF
        let bytes = [0x04, 0x00, 0x00, 0x00, 0xEF, 0xBE, 0xAD, 0xDE];
        let addrs = parse_init_array(&bytes).unwrap();
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].as_u32(), 0x04);
        assert_eq!(addrs[1].as_u32(), 0xDEAD_BEEF);
    }

    #[test]
    fn parse_init_array_rejects_more_than_max_init_functions() {
        let count = MAX_NUM_OF_INIT_FUNCTIONS + 1;
        let bytes = vec![0u8; count * N_BYTES_IN_WORD];
        let result = parse_init_array(&bytes);
        assert!(
            matches!(
                result,
                Err(RvElfParseError::TooManyInitFunctions {
                    count: got_count,
                    max,
                }) if got_count == count && max == MAX_NUM_OF_INIT_FUNCTIONS
            ),
            "expected TooManyInitFunctions {{ count: {count}, max: {MAX_NUM_OF_INIT_FUNCTIONS} }}, got {result:?}"
        );
    }

    #[test]
    fn parse_init_array_accepts_exactly_max_init_functions() {
        let bytes = vec![0u8; MAX_NUM_OF_INIT_FUNCTIONS * N_BYTES_IN_WORD];
        let addrs = parse_init_array(&bytes).unwrap();
        assert_eq!(addrs.len(), MAX_NUM_OF_INIT_FUNCTIONS);
    }

    // ── validate_callable_address ─────────────────────────────────────────────

    fn make_code_section(address: u32, size_in_bytes: u32) -> RvElfSection {
        RvElfSection {
            name: TEXT_SECTION_NAME.to_string(),
            kind: SectionKind::Text,
            alignment: 4,
            address: RiscvAddress::new(address),
            size_in_bytes,
            data: Vec::new(),
        }
    }

    #[test]
    fn validate_callable_address_accepts_aligned_in_range() {
        let code = make_code_section(0x1000, 0x100);
        // First, middle, last valid word
        for addr in [0x1000u32, 0x1004, 0x10FC] {
            validate_callable_address(&code, RiscvAddress::new(addr))
                .unwrap_or_else(|e| panic!("expected ok for {addr:#x}: {e:?}"));
        }
    }

    #[test]
    fn validate_callable_address_rejects_below_code_start() {
        let code = make_code_section(0x1000, 0x100);
        let result = validate_callable_address(&code, RiscvAddress::new(0xFFC));
        assert!(
            matches!(result, Err(RvElfParseError::BadCallableAddress { .. })),
            "expected BadCallableAddress for 0xFFC, got {result:?}"
        );
    }

    #[test]
    fn validate_callable_address_rejects_at_or_past_code_end() {
        let code = make_code_section(0x1000, 0x100);
        // code_end is exclusive: 0x1100 is not callable
        let result_at_end = validate_callable_address(&code, RiscvAddress::new(0x1100));
        assert!(
            matches!(
                result_at_end,
                Err(RvElfParseError::BadCallableAddress { .. })
            ),
            "expected BadCallableAddress for 0x1100, got {result_at_end:?}"
        );
        let result_past_end = validate_callable_address(&code, RiscvAddress::new(0x2000));
        assert!(
            matches!(
                result_past_end,
                Err(RvElfParseError::BadCallableAddress { .. })
            ),
            "expected BadCallableAddress for 0x2000, got {result_past_end:?}"
        );
    }

    #[test]
    fn validate_callable_address_rejects_misaligned() {
        let code = make_code_section(0x1000, 0x100);
        for addr in [0x1001u32, 0x1002, 0x1003] {
            assert!(
                matches!(
                    validate_callable_address(&code, RiscvAddress::new(addr)),
                    Err(RvElfParseError::BadCallableAddress { .. })
                ),
                "expected error for misaligned {addr:#x}"
            );
        }
    }

    #[test]
    fn validate_callable_address_detects_code_end_overflow() {
        // code_start + size_in_bytes overflows u32
        let code = make_code_section(u32::MAX - 3, 0x100);
        let result = validate_callable_address(&code, RiscvAddress::new(u32::MAX - 3));
        assert!(
            matches!(result, Err(RvElfParseError::ValueError(_))),
            "expected ValueError error from code-end overflow, got {result:?}"
        );
    }

    // ── RvEntryPoints::new ─────────────────────────────────────────────────

    #[test]
    fn entry_points_empty_has_no_indices() {
        let ep = RvEntryPoints::new(Vec::new(), HashMap::new()).unwrap();
        assert_that!(ep.init_array_addrs(), is_empty());
        assert_that!(ep.entry_point_addrs(), is_empty());
        assert_that!(ep.name_to_index(), is_empty());
        assert_eq!(ep.index_of("anything"), None);
        assert_eq!(ep.address_of(0), None);
        assert_eq!(ep.address_of(1), None);
    }

    #[test]
    fn entry_points_indexing_starts_at_one_and_init_array_first() {
        // Init array addresses come first in index order (1-based), entry
        // points follow sorted alphabetically by name.
        let init = vec![RiscvAddress::new(0x1000), RiscvAddress::new(0x1004)];
        let mut entry_points = HashMap::new();
        entry_points.insert("zeta".to_string(), RiscvAddress::new(0x1010));
        entry_points.insert("alpha".to_string(), RiscvAddress::new(0x1014));
        entry_points.insert("mu".to_string(), RiscvAddress::new(0x1018));

        let ep = RvEntryPoints::new(init.clone(), entry_points).unwrap();

        // Init array preserved in order at indices 1, 2
        assert_eq!(ep.init_array_addrs(), init.as_slice());
        assert_eq!(ep.address_of(1), Some(RiscvAddress::new(0x1000)));
        assert_eq!(ep.address_of(2), Some(RiscvAddress::new(0x1004)));

        // Entry points sorted alphabetically: alpha=3, mu=4, zeta=5
        assert_eq!(ep.index_of("alpha"), Some(3));
        assert_eq!(ep.index_of("mu"), Some(4));
        assert_eq!(ep.index_of("zeta"), Some(5));
        assert_eq!(ep.address_of(3), Some(RiscvAddress::new(0x1014)));
        assert_eq!(ep.address_of(4), Some(RiscvAddress::new(0x1018)));
        assert_eq!(ep.address_of(5), Some(RiscvAddress::new(0x1010)));

        // entry_point_addrs is in the same sorted order (alpha, mu, zeta)
        assert_eq!(
            ep.entry_point_addrs(),
            &[
                RiscvAddress::new(0x1014),
                RiscvAddress::new(0x1018),
                RiscvAddress::new(0x1010),
            ]
        );

        // Index 0 is reserved for ecall return and should never map
        assert_eq!(ep.address_of(0), None);
        // Out-of-range index
        assert_eq!(ep.address_of(6), None);
        assert_eq!(ep.index_of("missing"), None);
    }

    #[test]
    fn entry_points_name_to_index_matches_index_to_address() {
        let mut entry_points = HashMap::new();
        entry_points.insert("foo".to_string(), RiscvAddress::new(0x2000));
        entry_points.insert("bar".to_string(), RiscvAddress::new(0x2004));
        let ep = RvEntryPoints::new(Vec::new(), entry_points).unwrap();

        for (name, idx) in ep.name_to_index() {
            let addr = ep.address_of(*idx).unwrap();
            let expected = match name.as_str() {
                "foo" => RiscvAddress::new(0x2000),
                "bar" => RiscvAddress::new(0x2004),
                _ => unreachable!(),
            };
            assert_eq!(addr, expected);
        }
    }

    // ── RiscvElf helpers ──────────────────────────────────────────────────────

    fn make_data_section(name: &str, address: u32, size_in_bytes: u32) -> RvElfSection {
        RvElfSection {
            name: name.to_string(),
            kind: SectionKind::Data,
            alignment: 4,
            address: RiscvAddress::new(address),
            size_in_bytes,
            data: Vec::new(),
        }
    }

    fn make_elf(code: RvElfSection, data_sections: Vec<RvElfSection>) -> RiscvElf {
        RiscvElf {
            entry_points: RvEntryPoints::new(Vec::new(), HashMap::new()).unwrap(),
            symbol_values: RvSymbolValues {
                text_end: RiscvAddress::new(0),
                data_start: RiscvAddress::new(0),
                stack_top: RiscvAddress::new(0),
                data_end: RiscvAddress::new(0),
            },
            code_section: code,
            data_sections,
        }
    }

    #[test]
    fn risc_v_elf_get_sections_matches_code_and_data_by_name() {
        let code = make_code_section(0x1000, 0x100);
        let rodata = make_data_section(RODATA_SECTION_NAME, 0x2000, 0x40);
        let data = make_data_section(DATA_SECTION_NAME, 0x3000, 0x40);
        let elf = make_elf(code, vec![rodata, data]);

        let found = elf.get_sections(&[TEXT_SECTION_NAME, DATA_SECTION_NAME]);
        let names: Vec<&str> = found.iter().map(|s| s.name()).collect();
        assert_eq!(names, vec![TEXT_SECTION_NAME, DATA_SECTION_NAME]);

        assert!(
            elf.get_sections(&["does.not.exist"]).is_empty(),
            "expected empty collection"
        );
        assert!(
            elf.get_sections(&[]).is_empty(),
            "expected empty collection"
        );
    }

    #[test]
    fn risc_v_elf_stack_size_returns_zero_when_no_stack_section() {
        let code = make_code_section(0x1000, 0x100);
        let elf = make_elf(
            code,
            vec![make_data_section(DATA_SECTION_NAME, 0x2000, 0x40)],
        );
        assert_eq!(elf.stack_size_in_bytes(), 0);
    }

    #[test]
    fn risc_v_elf_stack_size_returns_section_size_when_present() {
        let code = make_code_section(0x1000, 0x100);
        let elf = make_elf(
            code,
            vec![make_data_section(STACK_SECTION_NAME, 0x4000, 0x400)],
        );
        assert_eq!(elf.stack_size_in_bytes(), 0x400);
    }

    #[test]
    fn risc_v_elf_section_addresses_and_sizes_maps_text_at_zero_and_data_by_address() {
        let code = make_code_section(0x1000, 0x100);
        let data = make_data_section(DATA_SECTION_NAME, 0x3000, 0x80);
        let rodata = make_data_section(RODATA_SECTION_NAME, 0x2000, 0x40);
        let elf = make_elf(code, vec![rodata, data]);

        let map = elf.section_addresses_and_sizes();
        // .text is reported at offset 0 regardless of its actual address
        assert_eq!(map.get(TEXT_SECTION_NAME).copied(), Some((0, 0x100)));
        assert_eq!(map.get(DATA_SECTION_NAME).copied(), Some((0x3000, 0x80)));
        assert_eq!(map.get(RODATA_SECTION_NAME).copied(), Some((0x2000, 0x40)));
    }

    #[test]
    fn risc_v_elf_code_section_as_i32_vec_decodes_data() {
        let code = RvElfSection {
            name: TEXT_SECTION_NAME.to_string(),
            kind: SectionKind::Text,
            alignment: 4,
            address: RiscvAddress::new(0x1000),
            size_in_bytes: 8,
            data: vec![1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF],
        };
        let elf = make_elf(code, Vec::new());
        assert_eq!(elf.code_section_as_i32_vec().unwrap(), vec![1, -1]);
    }

    #[test]
    fn risc_v_elf_heap_size_returns_zero_when_no_heap_section() {
        let code = make_code_section(0x1000, 0x100);
        let elf = make_elf(
            code,
            vec![make_data_section(DATA_SECTION_NAME, 0x2000, 0x40)],
        );
        assert_eq!(elf.heap_size_in_bytes(), 0);
    }

    #[test]
    fn risc_v_elf_heap_size_returns_section_size_when_present() {
        let code = make_code_section(0x1000, 0x100);
        let elf = make_elf(
            code,
            vec![make_data_section(HEAP_SECTION_NAME, 0x5000, 0x800)],
        );
        assert_eq!(elf.heap_size_in_bytes(), 0x800);
    }
}
