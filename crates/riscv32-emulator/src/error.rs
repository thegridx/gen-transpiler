//! Why loading a program into the emulator failed.
use riscv32::blocks::BlockError;
use riscv32_elf::reader::RvElfParseError;
use thiserror::Error;

/// Why [`Emulator::load`](crate::Emulator::load) rejected a program.
#[derive(Debug, Error)]
pub enum EmulatorError {
    /// The input is not a valid RISC-V ELF for this toolchain.
    #[error("RISC-V ELF parsing error: {0}")]
    RiscvElfParse(#[from] RvElfParseError),
    /// The program could not be split into gas blocks.
    #[error("gas blocks: {0}")]
    Blocks(#[from] BlockError),
    /// Metered runs need a gas cost above zero, or gas never runs out.
    #[error("metered runs need a gas cost above zero")]
    ZeroGasCost,
    /// A section lies outside the guest memory, `[0, _data_end_value)`.
    #[error("section {section} at {address:#x} ({size} bytes) is outside guest memory")]
    SectionOutsideMemory {
        section: String,
        address: u32,
        size: usize,
    },
}
