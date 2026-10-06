//! A well-formed program transpiles, whatever its instruction words, for both targets and in
//! both modes. Undecodable or invalid instructions become exits; they never fail the
//! transpilation. On a host that can run it, the output for this host loads. This fuzzes the
//! encoders of both targets on any host.
#![no_main]

#[path = "../../crates/transpiler/tests/riscv_elf_spec/mod.rs"]
mod riscv_elf_spec;
#[path = "common/words.rs"]
mod words;

use std::collections::HashSet;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use riscv_elf_spec::RiscvElfSpec;
use transpiler::execute::{LoaderContext, TranspiledVm};
use transpiler::{TranspilationMode, TranspilerForArchitecture, TranspilerInput};
use words::Word;

#[derive(Arbitrary, Debug)]
struct Input {
    words: Vec<Word>,
    /// Whether `main` is also the init function.
    init: bool,
}

fuzz_target!(|input: Input| {
    let elf = RiscvElfSpec {
        code: words::program(&input.words, 4096),
        init_array: if input.init { vec![0] } else { Vec::new() },
        ..RiscvElfSpec::default()
    }
    .build();
    let runnable = cfg!(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    ));
    let native = TranspilerForArchitecture::get_native().map(|n| n.to_string());
    for architecture in [
        TranspilerForArchitecture::Arm64,
        TranspilerForArchitecture::X86_64,
    ] {
        for mode in [TranspilationMode::default(), TranspilationMode::Trusted] {
            let output = architecture
                .transpile_to_native_elf(
                    TranspilerInput {
                        riscv_elf_bytes: &elf,
                        entry_points: HashSet::from(["main".to_string()]),
                    },
                    &mode,
                )
                .unwrap_or_else(|e| panic!("{architecture} {mode:?}: should transpile: {e}"));
            if runnable && native.as_deref() == Some(&architecture.to_string()) {
                TranspiledVm::load(
                    &output.elf_bytes,
                    LoaderContext::Trusted,
                    &output.entry_point_mapping,
                )
                .unwrap_or_else(|e| panic!("{architecture} {mode:?}: output should load: {e}"));
            }
        }
    }
});
