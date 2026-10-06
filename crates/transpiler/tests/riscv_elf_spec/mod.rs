//! Builds small RISC-V ELF files laid out as `riscv_elf_mem_layout.x` lays them out, for tests
//! that need inputs the toolchain would not produce.

use object::elf;
use object::write::StringId;
use object::write::elf::{FileHeader, SectionHeader, Sym, Writer};

pub(crate) const SEGMENT_ALIGN: u32 = 0x4000;
pub(crate) const STACK_SIZE: u32 = 0x10000;
pub(crate) const HEAP_SIZE: u32 = 0x10000;
pub(crate) const ECALL_DATA_SIZE: u32 = 0x10000;

/// `addi a0, a0, 1; ret`
pub(crate) const CODE: [u32; 2] = [0x0015_0513, 0x0000_8067];

pub(crate) struct TestSection {
    pub(crate) name: &'static str,
    pub(crate) sh_type: u32,
    pub(crate) sh_flags: u32,
    pub(crate) addr: u32,
    pub(crate) size: u32,
    pub(crate) data: Vec<u8>,
    pub(crate) align: u32,
}

impl TestSection {
    pub(crate) fn progbits(
        name: &'static str,
        sh_flags: u32,
        addr: u32,
        data: Vec<u8>,
        align: u32,
    ) -> Self {
        let size = u32::try_from(data.len()).expect("small section");
        Self {
            name,
            sh_type: elf::SHT_PROGBITS,
            sh_flags,
            addr,
            size,
            data,
            align,
        }
    }

    pub(crate) fn nobits(name: &'static str, addr: u32, size: u32, align: u32) -> Self {
        Self {
            name,
            sh_type: elf::SHT_NOBITS,
            sh_flags: elf::SHF_ALLOC | elf::SHF_WRITE,
            addr,
            size,
            data: Vec::new(),
            align,
        }
    }

    pub(crate) fn end(&self) -> u32 {
        self.addr + self.size
    }
}

pub(crate) fn align_up(value: u32, align: u32) -> u32 {
    value.div_ceil(align) * align
}

/// A RISC-V program laid out as `riscv_elf_mem_layout.x` lays it out. Tests adjust the fields
/// and then [`RiscvElfSpec::build`] it.
pub(crate) struct RiscvElfSpec {
    pub(crate) text_addr: u32,
    /// Instruction words of `.text`; the `main` entry point is the first one.
    pub(crate) code: Vec<u32>,
    /// `.init_array` entries, placed right after `.text`.
    pub(crate) init_array: Vec<u32>,
    pub(crate) rodata: Vec<u8>,
    /// Writable PROGBITS sections, placed one after another from the start of the data segment.
    pub(crate) data: Vec<(&'static str, Vec<u8>, u32)>,
    pub(crate) bss_size: u32,
    /// Extra sections, placed wherever the test puts them.
    pub(crate) extra: Vec<TestSection>,
    /// Overrides `_text_end_value`.
    pub(crate) text_end: Option<u32>,
    /// Overrides the address of `.rodata`.
    pub(crate) rodata_addr: Option<u32>,
    /// Moves the first writable PROGBITS section this many bytes up from where it belongs.
    pub(crate) data_shift: u32,
    pub(crate) e_type: u16,
    pub(crate) e_flags: u32,
    /// Binding of the `main` entry-point symbol.
    pub(crate) entry_binding: u8,
    /// Adds a second global `main` symbol.
    pub(crate) duplicate_entry: bool,
    /// Adds a local `main` symbol, as a `static` function of that name in another file would.
    pub(crate) local_entry_alias: bool,
    /// Overrides the values of layout symbols such as `_data_end_value`.
    pub(crate) layout_overrides: Vec<(&'static str, u32)>,
    /// Symbols added as they are.
    pub(crate) extra_symbols: Vec<TestSymbol>,
}

impl Default for RiscvElfSpec {
    fn default() -> Self {
        Self {
            text_addr: 0,
            code: CODE.to_vec(),
            init_array: Vec::new(),
            rodata: vec![1, 2, 3, 4],
            data: vec![(".data", vec![5, 6, 7, 8], 4)],
            bss_size: 16,
            extra: Vec::new(),
            text_end: None,
            rodata_addr: None,
            data_shift: 0,
            e_type: elf::ET_EXEC,
            e_flags: 0,
            entry_binding: elf::STB_GLOBAL,
            duplicate_entry: false,
            local_entry_alias: false,
            layout_overrides: Vec::new(),
            extra_symbols: Vec::new(),
        }
    }
}

impl RiscvElfSpec {
    pub(crate) fn build(self) -> Vec<u8> {
        let to_bytes = |words: &[u32]| -> Vec<u8> {
            words.iter().flat_map(|word| word.to_le_bytes()).collect()
        };
        let code = to_bytes(&self.code);
        let code_size = u32::try_from(code.len()).expect("small code");
        let mut sections = vec![TestSection::progbits(
            ".text",
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            self.text_addr,
            code,
            4,
        )];
        let mut code_end = code_size;
        if !self.init_array.is_empty() {
            let mut init_array = TestSection::progbits(
                ".init_array",
                elf::SHF_ALLOC | elf::SHF_WRITE,
                code_size,
                to_bytes(&self.init_array),
                4,
            );
            init_array.sh_type = elf::SHT_INIT_ARRAY;
            code_end = init_array.end();
            sections.push(init_array);
        }
        let rodata_addr = self
            .rodata_addr
            .unwrap_or_else(|| align_up(code_end, SEGMENT_ALIGN));
        let rodata_size = u32::try_from(self.rodata.len()).expect("small rodata");
        if rodata_size > 0 {
            sections.push(TestSection::progbits(
                ".rodata",
                elf::SHF_ALLOC,
                rodata_addr,
                self.rodata,
                4,
            ));
        }
        let stack_addr = align_up(rodata_addr + rodata_size, SEGMENT_ALIGN);
        sections.push(TestSection::nobits(".stack", stack_addr, STACK_SIZE, 16));
        let data_start = stack_addr + STACK_SIZE;
        let mut cur = data_start;
        for (i, (name, bytes, align)) in self.data.into_iter().enumerate() {
            let shift = if i == 0 { self.data_shift } else { 0 };
            let addr = align_up(cur, align) + shift;
            let section =
                TestSection::progbits(name, elf::SHF_ALLOC | elf::SHF_WRITE, addr, bytes, align);
            cur = section.end();
            sections.push(section);
        }
        if self.bss_size > 0 {
            let bss = TestSection::nobits(".bss", align_up(cur, 4), self.bss_size, 4);
            cur = bss.end();
            sections.push(bss);
        }
        let heap = TestSection::nobits(".heap", align_up(cur, 16), HEAP_SIZE, 16);
        let ecall_data = TestSection::nobits(".ecall_data", heap.end(), ECALL_DATA_SIZE, 16);
        let data_end = ecall_data.end();
        sections.extend([heap, ecall_data]);
        sections.extend(self.extra);

        let layout = |name: &'static str, default: u32| {
            let value = self
                .layout_overrides
                .iter()
                .find(|(overridden, _)| *overridden == name)
                .map_or(default, |(_, overriding)| *overriding);
            TestSymbol::absolute(name, value)
        };
        let mut symbols = vec![
            TestSymbol::function("main", self.text_addr, self.entry_binding),
            layout("_text_end_value", self.text_end.unwrap_or(code_size)),
            layout("_data_start_value", stack_addr),
            layout("_stack_top_value", data_start),
            layout("_data_end_value", data_end),
        ];
        if self.local_entry_alias {
            symbols.push(TestSymbol::function(
                "main",
                self.text_addr + 4,
                elf::STB_LOCAL,
            ));
        }
        if self.duplicate_entry {
            symbols.push(TestSymbol::function(
                "main",
                self.text_addr + 4,
                elf::STB_GLOBAL,
            ));
        }
        symbols.extend(self.extra_symbols);
        write_elf32(&sections, &symbols, self.e_type, self.e_flags)
    }
}

pub(crate) struct TestSymbol {
    pub(crate) name: &'static str,
    /// The address of a function in `.text`, or `None` for an absolute symbol.
    pub(crate) in_text: Option<u32>,
    pub(crate) value: u32,
    pub(crate) binding: u8,
}

impl TestSymbol {
    pub(crate) fn function(name: &'static str, address: u32, binding: u8) -> Self {
        Self {
            name,
            in_text: Some(address),
            value: address,
            binding,
        }
    }

    pub(crate) fn absolute(name: &'static str, value: u32) -> Self {
        Self {
            name,
            in_text: None,
            value,
            binding: elf::STB_GLOBAL,
        }
    }
}

/// Writes a little-endian ELF32 RISC-V file. The first section is `.text`.
pub(crate) fn write_elf32(
    sections: &[TestSection],
    unordered_symbols: &[TestSymbol],
    e_type: u16,
    e_flags: u32,
) -> Vec<u8> {
    // The symbol table lists local symbols first.
    let mut symbols: Vec<&TestSymbol> = unordered_symbols.iter().collect();
    symbols.sort_by_key(|symbol| symbol.binding != elf::STB_LOCAL);
    let n_local = symbols
        .iter()
        .filter(|symbol| symbol.binding == elf::STB_LOCAL)
        .count();
    let mut buffer = Vec::new();
    let mut writer = Writer::new(object::Endianness::Little, false, &mut buffer);
    writer.reserve_file_header();
    writer.reserve_null_section_index();
    let section_names: Vec<StringId> = sections
        .iter()
        .map(|section| writer.add_section_name(section.name.as_bytes()))
        .collect();
    let section_indices: Vec<_> = sections
        .iter()
        .map(|_| writer.reserve_section_index())
        .collect();
    writer.reserve_symtab_section_index();
    writer.reserve_strtab_section_index();
    writer.reserve_shstrtab_section_index();
    let symbol_names: Vec<StringId> = symbols
        .iter()
        .map(|symbol| writer.add_string(symbol.name.as_bytes()))
        .collect();
    writer.reserve_null_symbol_index();
    for symbol in &symbols {
        writer.reserve_symbol_index(symbol.in_text.map(|_| section_indices[0]));
    }
    let offsets: Vec<usize> = sections
        .iter()
        .map(|section| {
            if section.data.is_empty() {
                0
            } else {
                writer.reserve(section.data.len(), 4)
            }
        })
        .collect();
    writer.reserve_symtab();
    writer.reserve_strtab();
    writer.reserve_shstrtab();
    writer.reserve_section_headers();

    writer
        .write_file_header(&FileHeader {
            os_abi: elf::ELFOSABI_NONE,
            abi_version: 0,
            e_type,
            e_machine: elf::EM_RISCV,
            e_entry: 0,
            e_flags,
        })
        .expect("file header");
    for section in sections.iter().filter(|section| !section.data.is_empty()) {
        writer.write_align(4);
        writer.write(&section.data);
    }
    writer.write_null_symbol();
    for (symbol, name) in symbols.iter().zip(&symbol_names) {
        let (section, st_type) = match symbol.in_text {
            Some(_) => (Some(section_indices[0]), elf::STT_FUNC),
            None => (None, elf::STT_NOTYPE),
        };
        let st_value = symbol.value;
        writer.write_symbol(&Sym {
            name: Some(*name),
            section,
            st_info: (symbol.binding << 4) | st_type,
            st_other: elf::STV_DEFAULT,
            st_shndx: elf::SHN_ABS,
            st_value: u64::from(st_value),
            st_size: 0,
        });
    }
    writer.write_strtab();
    writer.write_shstrtab();
    writer.write_null_section_header();
    for ((section, name), offset) in sections.iter().zip(&section_names).zip(&offsets) {
        writer.write_section_header(&SectionHeader {
            name: Some(*name),
            sh_type: section.sh_type,
            sh_flags: u64::from(section.sh_flags),
            sh_addr: u64::from(section.addr),
            sh_offset: *offset as u64,
            sh_size: u64::from(section.size),
            sh_link: 0,
            sh_info: 0,
            sh_addralign: u64::from(section.align),
            sh_entsize: 0,
        });
    }
    writer.write_symtab_section_header(1 + u32::try_from(n_local).expect("few symbols"));
    writer.write_strtab_section_header();
    writer.write_shstrtab_section_header();
    buffer
}
