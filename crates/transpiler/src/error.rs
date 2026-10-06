//! The error type of the transpiler.
use riscv32::{blocks::BlockError, common::ValueError, immediate::BadImmediateValue};
use riscv32_elf::{reader::RvElfParseError, types::RiscvAddress};
use thiserror::Error;

use crate::{
    elf_builder::ElfBuilderError,
    layout::{GlobalLabel, LayoutValidationError, LocalCodeLabel},
};

#[derive(Error, Debug)]
pub enum TranspilerError {
    #[error("RISC-V ELF parsing error: {0}")]
    RiscvElfParse(#[from] RvElfParseError),
    #[error("value error: {0}")]
    ValueError(#[from] ValueError),
    #[error("layout validation error: {0}")]
    LayoutValidation(#[from] LayoutValidationError),
    #[error("ELF builder error: {0}")]
    ElfBuilder(#[from] ElfBuilderError),
    #[error("untrusted output needs a gas cost above zero, or it is not metered")]
    ZeroGasCost,
    #[error("unexpected symbolic operation")]
    UnexpectedSymbolicOperation,
    #[error("code size too large: {code_size_in_bytes}, max size: {max_size_in_bytes}")]
    CodeSizeTooLarge {
        code_size_in_bytes: u32,
        max_size_in_bytes: u32,
    },
    #[error("missing global label: {0}")]
    MissingGlobalLabel(String),
    #[error("missing RISC-V address for label: {0:?}")]
    MissingRiscvAddress(RiscvAddress),
    #[error("duplicate global label: {0}")]
    DuplicateGlobalLabel(String),
    #[error("local label not found: {0}")]
    LocalLabelNotFound(String),
    #[error("transpiled address too big: {0}")]
    TranspiledAddressTooBig(u32),
    #[error(
        "sections {first} and {second} are of the same kind and must be merged into one; link with riscv_elf_mem_layout.x"
    )]
    SectionsNotMerged { first: String, second: String },
    #[error("an instruction's gas cost {cost} exceeds the per-block maximum {max}")]
    GasCostTooLarge { cost: u16, max: u16 },
    #[error(
        "block boundary at instruction {boundary} is beyond the code ({n_instructions} instructions)"
    )]
    BlockBoundaryBeyondCode { boundary: u32, n_instructions: u32 },
    #[error("PC-relative offset {offset} does not fit the {op} instruction: {source}")]
    PcRelativeOffsetOutOfRange {
        op: &'static str,
        offset: i32,
        #[source]
        source: BadImmediateValue,
    },
}

impl TranspilerError {
    pub(crate) fn missing_global_label(label: impl Into<GlobalLabel>) -> Self {
        Self::MissingGlobalLabel(format!("{:?}", label.into()))
    }
    pub(crate) fn duplicate_global_label(label: GlobalLabel) -> Self {
        Self::DuplicateGlobalLabel(format!("{label:?}"))
    }
    pub(crate) fn local_label_not_found(label: LocalCodeLabel) -> Self {
        Self::LocalLabelNotFound(format!("{label:?}"))
    }
}

impl From<BlockError> for TranspilerError {
    fn from(error: BlockError) -> Self {
        match error {
            BlockError::BoundaryBeyondCode {
                boundary,
                n_instructions,
            } => Self::BlockBoundaryBeyondCode {
                boundary,
                n_instructions,
            },
            BlockError::GasCostTooLarge { cost, max } => Self::GasCostTooLarge { cost, max },
            BlockError::Value(e) => Self::ValueError(e),
        }
    }
}
