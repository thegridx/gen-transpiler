//! The RISC-V ELF reader rejects any input it cannot use with an error, never a panic.
#![no_main]

use std::collections::HashSet;

use libfuzzer_sys::fuzz_target;
use riscv32_elf::reader::read_riscv_elf;

fuzz_target!(|data: &[u8]| {
    let entry_points = HashSet::from(["main".to_string()]);
    let _result = read_riscv_elf(data, &entry_points);
});
