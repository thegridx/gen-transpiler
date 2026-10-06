//! Fixtures shared by the loader integration tests.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use transpiler::execute::{LoaderContext, LoaderError, TranspiledVm};
use transpiler::{TranspilationMode, TranspilerForArchitecture, TranspilerInput, TranspilerOutput};

/// A dependency-free RISC-V program that is already linked with the repository's linker script.
fn math_test_elf() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/rust_programs/math-test/artifacts/math_test.o");
    #[expect(
        clippy::disallowed_methods,
        reason = "Test reads a fixture synchronously."
    )]
    std::fs::read(&path).expect("math-test fixture should be readable")
}

/// Transpiles the math-test fixture, with `sum_1_to_x` as its only entry point.
pub(crate) fn transpile(
    transpiler: TranspilerForArchitecture,
    mode: &TranspilationMode,
) -> TranspilerOutput {
    let riscv_elf_bytes = math_test_elf();
    transpiler
        .transpile_to_native_elf(
            TranspilerInput {
                riscv_elf_bytes: &riscv_elf_bytes,
                entry_points: HashSet::from(["sum_1_to_x".to_string()]),
            },
            mode,
        )
        .expect("math-test should transpile")
}

/// Loads `elf_bytes` into `ctx`, with an empty entry-point mapping.
pub(crate) fn load(elf_bytes: &[u8], ctx: LoaderContext) -> Result<TranspiledVm, LoaderError> {
    TranspiledVm::load(elf_bytes, ctx, &HashMap::new())
}

/// The architecture of this host. Tests that call it are ignored on other hosts, where the
/// output cannot run.
pub(crate) fn native() -> TranspilerForArchitecture {
    TranspilerForArchitecture::get_native()
        .expect("tests that run output need an aarch64 or x86_64 host")
}
