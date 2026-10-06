//! The x86_64 backend: native ops and their encodings, the guest-to-host register mapping,
//! the per-instruction transpiler, the envelope, and the in-process entry wrapper.
use crate::{
    traits::TranspilerImplementation,
    x86_64::{
        code_wrapper::X86CodeWrapper, envelope::X86Envelope, ops::X86Op,
        transpile_inst::X86InstructionTranspiler,
    },
};

// Assembly-text rendering exists only as the oracle for the encoder tests, which compare
// each op's encoding with the system assembler's output, so it is compiled only for tests.
#[cfg(test)]
pub(crate) mod assembly;
pub(crate) mod bytecode;
pub(crate) mod code_wrapper;
pub(crate) mod envelope;
pub(crate) mod ops;
pub(crate) mod registers;
pub(crate) mod transpile_inst;

/// Marker type that selects the x86_64 backend's op type, instruction transpiler, envelope and
/// entry wrapper. Callers choose it with `TranspilerForArchitecture::X86_64`.
pub(crate) struct X86Transpiler {}

impl TranspilerImplementation for X86Transpiler {
    type OpType = X86Op;
    type InstructionTranspiler = X86InstructionTranspiler;
    type EnvelopeType = X86Envelope;
    type CodeWrapperType = X86CodeWrapper;
}
