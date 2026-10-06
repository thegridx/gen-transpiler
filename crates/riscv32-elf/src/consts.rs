/// Maximum number of entry points allowed in a transpiled program.
pub const MAX_NUM_OF_ENTRY_POINTS: usize = 100;
/// Maximum number of init array functions allowed in a transpiled program.
pub const MAX_NUM_OF_INIT_FUNCTIONS: usize = 10;

/// Sections common to both RISC-V and transpiled files
pub const TEXT_SECTION_NAME: &str = ".text";
pub const RODATA_SECTION_NAME: &str = ".rodata";
pub const BSS_SECTION_NAME: &str = ".bss";
pub const DATA_SECTION_NAME: &str = ".data";
pub const ECALL_DATA_SECTION_NAME: &str = ".ecall_data";
pub const STACK_SECTION_NAME: &str = ".stack";
pub const HEAP_SECTION_NAME: &str = ".heap";

/// Sections expected only in the RISC-V input files
pub(crate) const INIT_ARRAY_SECTION_NAME: &str = ".init_array";

/// Sections expected only in the transpiled output files
pub const REGISTER_STORE_SECTION_NAME: &str = ".register_store";
pub const ORIGINAL_RISC_PROG_SECTION_NAME: &str = ".original_risc_prog";
pub const JUMP_TABLE_SECTION_NAME: &str = ".jump_table";
pub const TRANSPILED_SYMBOLS_SECTION_NAME: &str = ".transpiled.symbols";

/// RISC-V ELF symbol names (shared between the ELF reader and the transpiler)
pub(crate) const SYM_TEXT_END_VALUE: &str = "_text_end_value";
pub(crate) const SYM_DATA_START_VALUE: &str = "_data_start_value";
pub(crate) const SYM_STACK_TOP_VALUE: &str = "_stack_top_value";
pub(crate) const SYM_DATA_END_VALUE: &str = "_data_end_value";

/// Contents of the `.transpiled.symbols` ELF section.
///
/// Stored as its eight fields in order, each a little-endian `u32`. The transpiler's ELF
/// builder writes it and the loader reads it.
///
/// All addresses are virtual addresses (u32) as they appear in the ELF file
/// before loading. The loader adds the load bias to obtain runtime addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranspiledSymbolsSection {
    /// Virtual address of `_start_transpiled_code` in `.text`.
    pub start_transpiled_code: u32,
    /// Virtual address of `__entry_points_hooks_begin` in `.text`.
    pub entry_points_hooks_begin: u32,
    /// Virtual address of `__init_array_end` in `.text`.
    pub init_array_end: u32,
    /// Virtual address of `__entry_points_hooks_end` in `.text`.
    pub entry_points_hooks_end: u32,
    /// Absolute value: size of `.original_risc_prog` (= end of original text).
    pub text_end_value: u32,
    /// Absolute value: guest address of the first writable byte, the start of `.stack`.
    pub data_start_value: u32,
    /// Absolute value: guest address of the top of the stack.
    pub stack_top_value: u32,
    /// Absolute value: offset from rodata start to the end of the R+W
    /// segment (the end of `.ecall_data`, which is the last R+W section).
    pub data_end_value: u32,
}

impl TranspiledSymbolsSection {
    /// Size of the encoded section in bytes.
    pub const SIZE_IN_BYTES: usize = size_of::<[u32; 8]>();

    /// Encodes the section: its fields in order, each a little-endian `u32`.
    pub fn to_bytes(&self) -> Vec<u8> {
        [
            self.start_transpiled_code,
            self.entry_points_hooks_begin,
            self.init_array_end,
            self.entry_points_hooks_end,
            self.text_end_value,
            self.data_start_value,
            self.stack_top_value,
            self.data_end_value,
        ]
        .iter()
        .flat_map(|field| field.to_le_bytes())
        .collect()
    }

    /// Decodes a section encoded by [`Self::to_bytes`], or returns `None` if `bytes` is not
    /// exactly [`Self::SIZE_IN_BYTES`] long.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::SIZE_IN_BYTES {
            return None;
        }
        let mut fields = bytes.as_chunks().0.iter().copied().map(u32::from_le_bytes);
        let mut next = || fields.next();
        Some(Self {
            start_transpiled_code: next()?,
            entry_points_hooks_begin: next()?,
            init_array_end: next()?,
            entry_points_hooks_end: next()?,
            text_end_value: next()?,
            data_start_value: next()?,
            stack_top_value: next()?,
            data_end_value: next()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TranspiledSymbolsSection;

    #[test]
    fn transpiled_symbols_round_trip() {
        let symbols = TranspiledSymbolsSection {
            start_transpiled_code: 1,
            entry_points_hooks_begin: 2,
            init_array_end: 3,
            entry_points_hooks_end: 4,
            text_end_value: 5,
            data_start_value: 6,
            stack_top_value: 7,
            data_end_value: 0x0102_0304,
        };
        let bytes = symbols.to_bytes();
        assert_eq!(bytes.len(), TranspiledSymbolsSection::SIZE_IN_BYTES);
        assert_eq!(bytes[..4], [1, 0, 0, 0]);
        assert_eq!(bytes[28..], [4, 3, 2, 1]);
        assert_eq!(TranspiledSymbolsSection::from_bytes(&bytes), Some(symbols));
        assert_eq!(TranspiledSymbolsSection::from_bytes(&bytes[1..]), None);
        assert_eq!(
            TranspiledSymbolsSection::from_bytes(&[bytes, vec![0]].concat()),
            None
        );
    }
}
