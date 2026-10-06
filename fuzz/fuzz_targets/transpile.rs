//! The transpiler rejects any input it cannot use with an error, never a panic, on both
//! targets. Whatever it accepts for this host's architecture, the loader loads.
#![no_main]

use std::collections::HashSet;

use libfuzzer_sys::fuzz_target;
use transpiler::execute::{LoaderContext, TranspiledVm};
use transpiler::{TranspilationMode, TranspilerForArchitecture, TranspilerInput};

fuzz_target!(|data: &[u8]| {
    let native = TranspilerForArchitecture::get_native();
    for architecture in [
        TranspilerForArchitecture::Arm64,
        TranspilerForArchitecture::X86_64,
    ] {
        for mode in [TranspilationMode::default(), TranspilationMode::Trusted] {
            let input = TranspilerInput {
                riscv_elf_bytes: data,
                entry_points: HashSet::new(),
            };
            let Ok(output) = architecture.transpile_to_native_elf(input, &mode) else {
                continue;
            };
            let runnable = cfg!(any(
                target_arch = "aarch64",
                all(target_arch = "x86_64", target_os = "linux")
            ));
            if runnable && native.is_some_and(|n| n.to_string() == architecture.to_string()) {
                TranspiledVm::load(
                    &output.elf_bytes,
                    LoaderContext::Trusted,
                    &output.entry_point_mapping,
                )
                .unwrap_or_else(|e| panic!("transpiled output should load ({mode:?}): {e}"));
            }
        }
    }
});
