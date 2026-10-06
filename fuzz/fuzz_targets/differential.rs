//! Coverage-guided version of the differential test: a program of arbitrary instruction words
//! behaves the same in the emulator and transpiled for this host, over an optional init
//! function and two calls, resumed after ecalls the host halts in.
#![no_main]

#[path = "../../crates/transpiler/tests/differential/mod.rs"]
mod differential;
#[path = "../../crates/transpiler/tests/riscv_elf_spec/mod.rs"]
mod riscv_elf_spec;
#[path = "common/words.rs"]
mod words;

use arbitrary::Arbitrary;
use differential::{Start, assert_same_behavior};
use libfuzzer_sys::fuzz_target;
use riscv_elf_spec::RiscvElfSpec;
use words::Word;

#[derive(Arbitrary, Debug)]
struct Input {
    words: Vec<Word>,
    arguments: [i32; 7],
    saved_registers: [i32; 23],
    gas: u32,
    /// Whether `main` is also the init function.
    init: bool,
    halt_on_ecall: bool,
}

fuzz_target!(|input: Input| {
    if !cfg!(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )) {
        return;
    }
    let elf = RiscvElfSpec {
        code: words::program(&input.words, 256),
        init_array: if input.init { vec![0] } else { Vec::new() },
        ..RiscvElfSpec::default()
    }
    .build();
    let start = Start {
        arguments: input.arguments,
        saved_registers: input.saved_registers.to_vec(),
        // Arbitrary words can loop, so gas bounds every run.
        gas: i64::from(input.gas % 200_000),
        halt_on_ecall: input.halt_on_ecall,
    };
    assert_same_behavior(&elf, &start);
});
