//! The aarch64 backend: native ops and their encodings, the guest-to-host register mapping,
//! the per-instruction transpiler, the envelope, and the in-process entry wrapper.
use crate::{
    arch64::{
        code_wrapper::Arm64CodeWrapper, envelope::Arm64Envelope, ops::Arm64Op,
        transpile_inst::Arm64InstructionTranspiler,
    },
    traits::TranspilerImplementation,
};

// Assembly-text rendering exists only as the oracle for the encoder tests, which compare
// each op's encoding with the GNU assembler's output, so it is compiled only for tests.
#[cfg(test)]
pub(crate) mod assembly;
pub(crate) mod bytecode;
pub(crate) mod code_wrapper;
pub(crate) mod envelope;
pub(crate) mod ops;
pub(crate) mod registers;
pub(crate) mod transpile_inst;

/// Marker type that selects the aarch64 transpiler implementation.
///
/// Reached through `TranspilerForArchitecture::Arm64`.
pub(crate) struct Arm64Transpiler {}

impl TranspilerImplementation for Arm64Transpiler {
    type OpType = Arm64Op;
    type InstructionTranspiler = Arm64InstructionTranspiler;
    type EnvelopeType = Arm64Envelope;
    type CodeWrapperType = Arm64CodeWrapper;
}
