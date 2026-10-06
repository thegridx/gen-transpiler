//! The decoder never panics, and decoding is stable: re-encoding a decoded instruction and
//! decoding it again gives the same instruction.
#![no_main]

use libfuzzer_sys::fuzz_target;
use riscv32::ops::RiscvOp;

fuzz_target!(|word: u32| {
    if let Ok(op) = RiscvOp::decode(word as i32) {
        let again = RiscvOp::decode(op.encode())
            .unwrap_or_else(|e| panic!("{op:?} re-encodes to an undecodable word: {e:?}"));
        assert_eq!(format!("{op:?}"), format!("{again:?}"), "word {word:#010x}");
    }
});
