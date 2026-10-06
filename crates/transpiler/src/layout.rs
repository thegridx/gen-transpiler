//! Conventions shared by the transpiler and the loader: labels, jump-table entry formats,
//! size limits, section identifiers and the layout check.
use riscv32::common::{ValueError, checked_add_u32};
use riscv32::gas_cost::RvGasCost;
use riscv32_elf::consts::{
    BSS_SECTION_NAME, DATA_SECTION_NAME, ECALL_DATA_SECTION_NAME, HEAP_SECTION_NAME,
    JUMP_TABLE_SECTION_NAME, ORIGINAL_RISC_PROG_SECTION_NAME, REGISTER_STORE_SECTION_NAME,
    RODATA_SECTION_NAME, STACK_SECTION_NAME, TEXT_SECTION_NAME, TRANSPILED_SYMBOLS_SECTION_NAME,
};
use riscv32_elf::{data_layout::StoredRegister, types::RiscvAddress};
use std::{collections::HashMap, fmt};
use strum::EnumIter;
use thiserror::Error;

use crate::transpiled_address::{TranspiledAddress, TranspiledAddressAndSize};

const EXIT_NORMAL_LABEL: &str = "_normal_exit";
const EXIT_PANIC_LABEL: &str = "_panic";
const EXIT_ILLEGAL_OPCODE_LABEL: &str = "_illegal_opcode";
const SAVE_LOCAL_STATE_LABEL: &str = "_save_local_state";
const NOT_ECALL_RETURN_LABEL: &str = "_not_ecall_return";
const ECALL_EXIT_LABEL: &str = "_ecall_exit";
const CODE_SECTION_END_LABEL: &str = "_code_section_end";
const JUMP_TABLE_LABEL: &str = "_jump_table";
const USER_RODATA_START_LABEL: &str = "__user_rodata_start";
const REGISTER_STORE_LABEL: &str = "_register_store";

/// A named location in the envelope code that branches can target.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, EnumIter)]
pub(crate) enum GlobalCodeLabel {
    ExitNormal,
    ExitIllegalOpcode,
    ExitPanic,
    SaveLocalState,
    StartTranspiledCode,
    NotEcallReturn,
    EcallExit,
}
impl fmt::Display for GlobalCodeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            GlobalCodeLabel::ExitNormal => EXIT_NORMAL_LABEL,
            GlobalCodeLabel::ExitPanic => EXIT_PANIC_LABEL,
            GlobalCodeLabel::ExitIllegalOpcode => EXIT_ILLEGAL_OPCODE_LABEL,
            GlobalCodeLabel::SaveLocalState => SAVE_LOCAL_STATE_LABEL,
            GlobalCodeLabel::StartTranspiledCode => SYM_START_TRANSPILED_CODE,
            GlobalCodeLabel::NotEcallReturn => NOT_ECALL_RETURN_LABEL,
            GlobalCodeLabel::EcallExit => ECALL_EXIT_LABEL,
        };
        write!(f, "{label}")
    }
}

/// A named address that code computes or loads from, or that the ELF builder needs: the
/// entry-point hook table, the end of the code, the jump table, the start of the guest
/// address space and the register store.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, EnumIter)]
pub(crate) enum GlobalDataLabel {
    EntryPointsHooksBegin,
    InitArrayEnd,
    EntryPointsHooksEnd,
    CodeSectionEnd,
    JumpTable,
    RoDataStart,
    RegisterStore,
}
impl fmt::Display for GlobalDataLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            GlobalDataLabel::EntryPointsHooksBegin => SYM_ENTRY_POINTS_HOOKS_BEGIN,
            GlobalDataLabel::InitArrayEnd => SYM_INIT_ARRAY_END_LABEL,
            GlobalDataLabel::EntryPointsHooksEnd => SYM_ENTRY_POINTS_HOOKS_END,
            GlobalDataLabel::CodeSectionEnd => CODE_SECTION_END_LABEL,
            GlobalDataLabel::JumpTable => JUMP_TABLE_LABEL,
            GlobalDataLabel::RoDataStart => USER_RODATA_START_LABEL,
            GlobalDataLabel::RegisterStore => REGISTER_STORE_LABEL,
        };
        write!(f, "{label}")
    }
}

/// A code or data label with a single address in the output.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum GlobalLabel {
    Code(GlobalCodeLabel),
    Data(GlobalDataLabel),
}
impl fmt::Display for GlobalLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlobalLabel::Code(g) => write!(f, "{g}"),
            GlobalLabel::Data(g) => write!(f, "{g}"),
        }
    }
}
impl From<GlobalCodeLabel> for GlobalLabel {
    fn from(label: GlobalCodeLabel) -> Self {
        GlobalLabel::Code(label)
    }
}
impl From<GlobalDataLabel> for GlobalLabel {
    fn from(label: GlobalDataLabel) -> Self {
        GlobalLabel::Data(label)
    }
}
impl From<GlobalLabel> for SymbolicLabel {
    fn from(label: GlobalLabel) -> Self {
        SymbolicLabel::Global(label)
    }
}

/// A label that occurs many times; a branch to it resolves to a nearby occurrence. Its
/// `Display` form follows the GNU assembler's numeric local labels.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum LocalCodeLabel {
    /// Resolves to the next occurrence after the branch.
    SmallSkip,
    /// Resolves to the most recent jump to the panic exit before the branch (aarch64 only).
    PanicIndirection,
}
impl fmt::Display for LocalCodeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            LocalCodeLabel::SmallSkip => "1f",
            LocalCodeLabel::PanicIndirection => "2b",
        };
        write!(f, "{label}")
    }
}

/// A label in the instruction stream.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum SymbolicLabel {
    Global(GlobalLabel),
    Local(LocalCodeLabel),
}
impl fmt::Display for SymbolicLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymbolicLabel::Global(g) => write!(f, "{g}"),
            SymbolicLabel::Local(l) => write!(f, "{l}"),
        }
    }
}
impl From<GlobalCodeLabel> for SymbolicLabel {
    fn from(label: GlobalCodeLabel) -> Self {
        SymbolicLabel::Global(label.into())
    }
}
impl From<GlobalDataLabel> for SymbolicLabel {
    fn from(label: GlobalDataLabel) -> Self {
        SymbolicLabel::Global(label.into())
    }
}
impl From<LocalCodeLabel> for SymbolicLabel {
    fn from(label: LocalCodeLabel) -> Self {
        SymbolicLabel::Local(label)
    }
}

/// A memory operand at a byte offset from a data label. Only the x86_64 target uses it, for
/// RIP-relative accesses such as register-store slots.
#[derive(Clone, Debug)]
pub(crate) struct SymbolicMemoryLocation {
    pub(crate) label: GlobalDataLabel,
    pub(crate) offset: u32,
}
impl From<GlobalDataLabel> for SymbolicMemoryLocation {
    fn from(label: GlobalDataLabel) -> Self {
        Self { label, offset: 0 }
    }
}
impl From<StoredRegister> for SymbolicMemoryLocation {
    fn from(reg: StoredRegister) -> Self {
        Self {
            label: GlobalDataLabel::RegisterStore,
            offset: reg.get_offset(),
        }
    }
}

#[expect(
    variant_size_differences,
    reason = "The enum models instruction operands of intentionally different sizes; the size disparity is inherent to the representation."
)]
/// The target of a branch whose offset is not known yet.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum SymbolicBranchTarget {
    RiscLoc(RiscvAddress),
    Label(GlobalCodeLabel),
    Local(LocalCodeLabel),
}
impl fmt::Display for SymbolicBranchTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Not valid assembly; for debug output only.
            SymbolicBranchTarget::RiscLoc(addr) => write!(f, "risc_addr_{:#x}", addr.as_u32()),
            SymbolicBranchTarget::Label(g) => write!(f, "{g}"),
            SymbolicBranchTarget::Local(l) => write!(f, "{l}"),
        }
    }
}
impl From<RiscvAddress> for SymbolicBranchTarget {
    fn from(addr: RiscvAddress) -> Self {
        SymbolicBranchTarget::RiscLoc(addr)
    }
}
impl From<GlobalCodeLabel> for SymbolicBranchTarget {
    fn from(label: GlobalCodeLabel) -> Self {
        SymbolicBranchTarget::Label(label)
    }
}
impl From<LocalCodeLabel> for SymbolicBranchTarget {
    fn from(label: LocalCodeLabel) -> Self {
        SymbolicBranchTarget::Local(label)
    }
}

pub(crate) type GlobalLabelAddrMap = HashMap<GlobalLabel, TranspiledAddress>;

/// One jump-table entry before encoding.
#[derive(Debug, Clone)]
pub(crate) struct JumpTableEntry {
    /// Gas from this instruction to the end of its block; 0 for a block's first instruction,
    /// whose code deducts the whole block's cost.
    gas_cost: RvGasCost,
    /// Address of the instruction's native code, relative to the start of `.text`.
    transpiled_addr: TranspiledAddress,
}

impl JumpTableEntry {
    pub(crate) fn new(gas_cost: RvGasCost, transpiled_addr: TranspiledAddress) -> Self {
        JumpTableEntry {
            gas_cost,
            transpiled_addr,
        }
    }
    pub(crate) fn gas_cost(&self) -> RvGasCost {
        self.gas_cost
    }
    pub(crate) fn transpiled_addr(&self) -> TranspiledAddress {
        self.transpiled_addr
    }
}

/// Size of an untrusted jump-table entry: `i32` gas cost, then `i32` target offset relative to
/// the offset field itself.
pub(crate) const JUMP_TABLE_ENTRY_BYTE_SIZE: u32 = 8;
/// Byte offset of the target-offset field within an untrusted jump-table entry.
pub(crate) const JUMP_TABLE_OFFSET_POSITION: u32 = 4;
/// Size of a trusted jump-table entry: only the `i32` target offset.
pub(crate) const JUMP_TABLE_ENTRY_BYTE_SIZE_TRUSTED: u32 = 4;

/// ELF `e_flags` value that marks trusted output (the ASCII tag `TRST`). Untrusted output
/// leaves `e_flags` at 0. The loader reads it before it maps any code, so it can refuse a
/// trusted artifact in an untrusted context. It is a label, not a security boundary.
pub const EF_TRUSTED_TRANSPILED: u32 = u32::from_le_bytes(*b"TRST"); // 0x54535254

/// Largest `.text` size, after alignment to the segment boundary: 4 MiB.
pub(crate) const MAX_CODE_SECTION_SIZE_IN_BYTES: u32 = 0x400000;
/// Largest offset of `.register_store` from the start of `.text`: 8 MiB.
pub(crate) const MAX_TRANSPILED_ADDRESS: u32 = 0x800000;

/// The envelope's entry point. The host calls it for every entry; its address is the ELF
/// entry point and is recorded in `.transpiled.symbols`.
pub(crate) const SYM_START_TRANSPILED_CODE: &str = "_start_transpiled_code";
/// Start of the entry-point hook table in `.text`: one branch per init function, then one per
/// entry point.
pub(crate) const SYM_ENTRY_POINTS_HOOKS_BEGIN: &str = "__entry_points_hooks_begin";
/// End of the init-function branches and start of the entry-point branches in the hook table.
pub(crate) const SYM_INIT_ARRAY_END_LABEL: &str = "__init_array_end";
/// End of the entry-point hook table in `.text`.
pub(crate) const SYM_ENTRY_POINTS_HOOKS_END: &str = "__entry_points_hooks_end";

/// Identifies sections in the transpiled ELF file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter)]
#[repr(u8)]
pub(crate) enum SectionId {
    Text = 0,
    JumpTable,
    TranspiledSymbols,
    OriginalRiscProg,
    Rodata,
    Data,
    Bss,
    EcallData,
    Stack,
    Heap,
    RegisterStore,
}

impl SectionId {
    /// Number of section IDs.
    pub(crate) const COUNT: usize = (SectionId::RegisterStore as usize) + 1;

    pub(crate) fn name(&self) -> &'static str {
        match self {
            SectionId::Text => TEXT_SECTION_NAME,
            SectionId::JumpTable => JUMP_TABLE_SECTION_NAME,
            SectionId::TranspiledSymbols => TRANSPILED_SYMBOLS_SECTION_NAME,
            SectionId::OriginalRiscProg => ORIGINAL_RISC_PROG_SECTION_NAME,
            SectionId::Rodata => RODATA_SECTION_NAME,
            SectionId::Data => DATA_SECTION_NAME,
            SectionId::Bss => BSS_SECTION_NAME,
            SectionId::EcallData => ECALL_DATA_SECTION_NAME,
            SectionId::Stack => STACK_SECTION_NAME,
            SectionId::Heap => HEAP_SECTION_NAME,
            SectionId::RegisterStore => REGISTER_STORE_SECTION_NAME,
        }
    }
}

/// The output sections do not match the layout that the input's linker-script symbols define.
#[derive(Debug, Error)]
pub enum LayoutValidationError {
    #[error("missing section: {name}")]
    MissingSection { name: String },
    #[error("original_risc_prog size mismatch: expected {expected}, actual {actual}")]
    OriginalRiscProgSizeMismatch { expected: u32, actual: u32 },
    #[error("wrong address: {name}: expected {expected:#x}, actual {actual:#x}")]
    WrongAddress {
        name: String,
        expected: u32,
        actual: u32,
    },
    #[error("value error: {0}")]
    ValueError(#[from] ValueError),
}

const DATA_START_ADDRESS: &str = "data_start";
const STACK_TOP_ADDRESS: &str = "stack_top";
const DATA_END_ADDRESS: &str = "data_end";
const REGISTER_STORE_ADDRESS: &str = "register_store";

fn get_section<'a>(
    sections: &'a HashMap<SectionId, TranspiledAddressAndSize>,
    id: &SectionId,
) -> Result<&'a TranspiledAddressAndSize, LayoutValidationError> {
    sections
        .get(id)
        .ok_or(LayoutValidationError::MissingSection {
            name: id.name().to_string(),
        })
}

fn check_address(name: &str, expected: u32, actual: u32) -> Result<(), LayoutValidationError> {
    if expected != actual {
        return Err(LayoutValidationError::WrongAddress {
            name: name.to_string(),
            expected,
            actual,
        });
    }
    Ok(())
}

/// Checks that the output sections match the layout of `riscv_elf_mem_layout.x`, as given by
/// the input's linker-script symbols. The `*_value` arguments are guest addresses, that is,
/// offsets from the start of `.original_risc_prog`. Both the ELF builder and the loader run
/// this check.
pub(crate) fn validate_layout(
    sections: &HashMap<SectionId, TranspiledAddressAndSize>,
    orig_text_end_value: TranspiledAddress,
    data_start_value: TranspiledAddress,
    stack_top_value: TranspiledAddress,
    data_end_value: TranspiledAddress,
) -> Result<(), LayoutValidationError> {
    let orig_risc_prog = get_section(sections, &SectionId::OriginalRiscProg)?;
    // `.original_risc_prog` holds exactly the original code, `_text_end_value` bytes.
    if orig_risc_prog.size != orig_text_end_value.as_u32() {
        return Err(LayoutValidationError::OriginalRiscProgSizeMismatch {
            expected: orig_text_end_value.as_u32(),
            actual: orig_risc_prog.size,
        });
    }

    // Guest address 0 is the start of `.original_risc_prog`.
    let rodata_start = orig_risc_prog.addr.as_u32();

    // `.stack` starts at `_data_start_value`, the lowest writable address. It sits below
    // `.data` so a stack that grows past its bottom faults against the read-only segment below
    // it instead of corrupting `.data`. Stores may write from here up to `_data_end_value`.
    let expected_region_start =
        checked_add_u32(rodata_start, data_start_value.as_u32(), DATA_START_ADDRESS)?;
    let stack = get_section(sections, &SectionId::Stack)?;
    check_address(
        DATA_START_ADDRESS,
        expected_region_start,
        stack.addr.as_u32(),
    )?;

    // `.stack` ends at `_stack_top_value`.
    let expected_stack_top =
        checked_add_u32(rodata_start, stack_top_value.as_u32(), STACK_TOP_ADDRESS)?;
    let stack_top = checked_add_u32(stack.addr.as_u32(), stack.size, STACK_TOP_ADDRESS)?;
    check_address(STACK_TOP_ADDRESS, expected_stack_top, stack_top)?;

    // The first writable section after the stack starts at the top of the stack: the data
    // segment starts on the segment boundary right above the stack segment. `.data` and `.bss`
    // may be absent, in which case `.heap` comes first.
    let first_writable_after_stack = if let Some(data) = sections.get(&SectionId::Data) {
        data
    } else if let Some(bss) = sections.get(&SectionId::Bss) {
        bss
    } else {
        get_section(sections, &SectionId::Heap)?
    };
    check_address(
        STACK_TOP_ADDRESS,
        expected_stack_top,
        first_writable_after_stack.addr.as_u32(),
    )?;

    // `.ecall_data` ends at `_data_end_value`, the end of the guest-accessible memory.
    let expected_data_end =
        checked_add_u32(rodata_start, data_end_value.as_u32(), DATA_END_ADDRESS)?;
    let ecall_data = get_section(sections, &SectionId::EcallData)?;
    let ecall_data_end =
        checked_add_u32(ecall_data.addr.as_u32(), ecall_data.size, DATA_END_ADDRESS)?;
    check_address(DATA_END_ADDRESS, expected_data_end, ecall_data_end)?;

    // `.register_store` starts right at `_data_end_value`, outside guest-accessible memory.
    let expected_reg_store = checked_add_u32(
        rodata_start,
        data_end_value.as_u32(),
        REGISTER_STORE_ADDRESS,
    )?;
    let reg_store = get_section(sections, &SectionId::RegisterStore)?;
    check_address(
        REGISTER_STORE_ADDRESS,
        expected_reg_store,
        reg_store.addr.as_u32(),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use googletest::{
        assert_that,
        matchers::{eq, pat},
    };
    use riscv32_elf::data_layout::{ECALL_DATA_SIZE_BYTES, REGISTER_STORE_SIZE_BYTES};

    use super::*;

    const HEAP_SIZE_BYTES: u32 = 0x10000; // 64K
    const STACK_SIZE_BYTES: u32 = 0x10000; // 64K

    /// Test fixture for validate_layout tests with valid baseline configuration.
    struct ValidLayoutFixture {
        sections: HashMap<SectionId, TranspiledAddressAndSize>,
        rodata_addr: TranspiledAddress,
        orig_text_end_value: TranspiledAddress,
        data_start_value: TranspiledAddress,
        stack_top_value: TranspiledAddress,
        data_end_value: TranspiledAddress,
    }

    impl ValidLayoutFixture {
        fn validate(&self) -> Result<(), LayoutValidationError> {
            validate_layout(
                &self.sections,
                self.orig_text_end_value,
                self.data_start_value,
                self.stack_top_value,
                self.data_end_value,
            )
        }
    }

    /// Creates a valid layout fixture with all sections correctly configured.
    fn create_valid_layout() -> ValidLayoutFixture {
        let rodata_addr = TranspiledAddress::new(0x1000);
        let orig_text_end_value = TranspiledAddress::new(0x4c);
        // R+W layout (offsets from rodata start), `.stack` first so an underflow faults against
        // the read-only rodata segment:
        //   .stack       starts at data_start_value = 0x4000, ends at stack_top_value = 0x14000
        //   .data        starts at stack_top_value = 0x14000 (abuts the stack top)
        //   .heap        starts at 0x14000, ends at 0x24000 (HEAP_SIZE = 0x10000)
        //   .ecall_data  starts at 0x24000, ends at data_end_value = 0x34000
        let data_start_value = TranspiledAddress::new(0x4000);
        let stack_top_value = TranspiledAddress::new(0x14000);
        let data_end_value = TranspiledAddress::new(0x34000);

        let mut sections = HashMap::new();
        sections.insert(
            SectionId::OriginalRiscProg,
            TranspiledAddressAndSize {
                addr: rodata_addr,
                size: orig_text_end_value.as_u32(),
            },
        );
        sections.insert(
            SectionId::Stack,
            TranspiledAddressAndSize {
                addr: rodata_addr.checked_add(data_start_value.as_u32()).unwrap(),
                size: STACK_SIZE_BYTES,
            },
        );
        sections.insert(
            SectionId::Data,
            TranspiledAddressAndSize {
                addr: rodata_addr.checked_add(stack_top_value.as_u32()).unwrap(),
                size: 100,
            },
        );
        sections.insert(
            SectionId::Heap,
            TranspiledAddressAndSize {
                addr: rodata_addr.checked_add(stack_top_value.as_u32()).unwrap(),
                size: HEAP_SIZE_BYTES,
            },
        );
        sections.insert(
            SectionId::EcallData,
            TranspiledAddressAndSize {
                addr: rodata_addr
                    .checked_add(
                        data_end_value
                            .as_u32()
                            .checked_sub(ECALL_DATA_SIZE_BYTES)
                            .expect("data_end_value must be >= ECALL_DATA_SIZE_BYTES"),
                    )
                    .expect("ecall_data address overflow"),
                size: ECALL_DATA_SIZE_BYTES,
            },
        );
        sections.insert(
            SectionId::RegisterStore,
            TranspiledAddressAndSize {
                addr: rodata_addr.checked_add(data_end_value.as_u32()).unwrap(),
                size: REGISTER_STORE_SIZE_BYTES,
            },
        );

        ValidLayoutFixture {
            sections,
            rodata_addr,
            orig_text_end_value,
            data_start_value,
            stack_top_value,
            data_end_value,
        }
    }

    #[test]
    fn validate_layout_succeeds_with_correct_layout() {
        let fixture = create_valid_layout();
        let result = fixture.validate();
        assert!(result.is_ok(), "validate_layout should succeed: {result:?}");
    }

    #[test]
    fn validate_layout_fails_with_wrong_original_risc_prog_size() {
        let mut fixture = create_valid_layout();

        // Make the actual size in sections different from declared size
        fixture
            .sections
            .get_mut(&SectionId::OriginalRiscProg)
            .unwrap()
            .size = 0x100;

        let result = fixture.validate();
        assert!(result.is_err());
        assert_that!(
            result.unwrap_err(),
            pat!(LayoutValidationError::OriginalRiscProgSizeMismatch { .. })
        );
    }

    #[test]
    fn validate_layout_fails_with_wrong_data_address() {
        let mut fixture = create_valid_layout();

        // Set data section at wrong address (0x6000 instead of 0x5000)
        fixture.sections.get_mut(&SectionId::Data).unwrap().addr =
            fixture.rodata_addr.checked_add(0x6000).unwrap();

        let result = fixture.validate();
        assert!(result.is_err());
        assert_that!(
            result.unwrap_err(),
            pat!(LayoutValidationError::WrongAddress {
                name: eq(STACK_TOP_ADDRESS),
                ..
            })
        );
    }

    #[test]
    fn validate_layout_fails_with_wrong_reg_store_address() {
        let mut fixture = create_valid_layout();

        // Set register_store at wrong address (0x200000 instead of stack_top_value)
        fixture
            .sections
            .get_mut(&SectionId::RegisterStore)
            .unwrap()
            .addr = fixture.rodata_addr.checked_add(0x200000).unwrap();

        let result = fixture.validate();
        assert!(result.is_err());
        assert_that!(
            result.unwrap_err(),
            pat!(LayoutValidationError::WrongAddress {
                name: eq(REGISTER_STORE_ADDRESS),
                ..
            })
        );
    }

    #[test]
    fn validate_layout_handles_arithmetic_overflow() {
        let mut fixture = create_valid_layout();

        // Set rodata_addr near u32::MAX to trigger arithmetic overflow
        fixture.rodata_addr = TranspiledAddress::new(u32::MAX - 100);
        fixture.data_start_value = TranspiledAddress::new(200); // Adding this will overflow

        // Update all section addresses to use the new rodata_vaddr
        fixture
            .sections
            .get_mut(&SectionId::OriginalRiscProg)
            .unwrap()
            .addr = fixture.rodata_addr;
        fixture
            .sections
            .get_mut(&SectionId::RegisterStore)
            .unwrap()
            .addr = fixture.rodata_addr.wrapping_add(0x1000);
        fixture.sections.get_mut(&SectionId::Stack).unwrap().addr = fixture
            .rodata_addr
            .wrapping_add(0x1000)
            .wrapping_sub(STACK_SIZE_BYTES);
        fixture.stack_top_value = TranspiledAddress::new(0x1000);

        let result = fixture.validate();
        assert!(result.is_err());
        assert_that!(
            result.unwrap_err(),
            pat!(LayoutValidationError::ValueError(_))
        );
    }

    // When `.data` is empty it is dropped from the layout, so `.bss` becomes the first writable
    // section abutting the stack top (as in the multi-init smoke test). The adjacency check must
    // fall through to it instead of skipping the check or landing on `.heap`.
    #[test]
    fn validate_layout_succeeds_with_empty_data_and_bss_at_stack_top() {
        let mut fixture = create_valid_layout();
        let stack_top_addr = fixture
            .rodata_addr
            .checked_add(fixture.stack_top_value.as_u32())
            .unwrap();
        fixture.sections.remove(&SectionId::Data);
        fixture.sections.insert(
            SectionId::Bss,
            TranspiledAddressAndSize {
                addr: stack_top_addr,
                size: 16,
            },
        );

        let result = fixture.validate();
        assert!(result.is_ok(), "validate_layout should succeed: {result:?}");
    }

    #[test]
    fn validate_layout_fails_with_empty_data_and_wrong_bss_address() {
        let mut fixture = create_valid_layout();
        fixture.sections.remove(&SectionId::Data);
        fixture.sections.insert(
            SectionId::Bss,
            TranspiledAddressAndSize {
                // 16 bytes above the stack top: the exact regression the smoke test caught.
                addr: fixture
                    .rodata_addr
                    .checked_add(fixture.stack_top_value.as_u32())
                    .unwrap()
                    .checked_add(0x10)
                    .unwrap(),
                size: 16,
            },
        );

        let result = fixture.validate();
        assert!(result.is_err());
        assert_that!(
            result.unwrap_err(),
            pat!(LayoutValidationError::WrongAddress {
                name: eq(STACK_TOP_ADDRESS),
                ..
            })
        );
    }

    // With neither `.data` nor `.bss` present, `.heap` is the first writable section and must abut
    // the stack top.
    #[test]
    fn validate_layout_succeeds_with_only_heap_after_stack() {
        let mut fixture = create_valid_layout();
        fixture.sections.remove(&SectionId::Data);

        let result = fixture.validate();
        assert!(result.is_ok(), "validate_layout should succeed: {result:?}");
    }
}
