//! The host reads and writes guest memory only within the guest's bounds: reads anywhere in
//! `[0, _data_end_value)`, writes only in the writable region. Anything else is an error.

mod common;

use common::{load, native, transpile};
use transpiler::TranspilationMode;
use transpiler::execute::{LoaderContext, TranspiledCodeRunner};
use user_code_runner::runner::{
    CodeRunner as _, MemoryAccessor as _, UserCodeRunnerError, VirtualDataPointer,
};

fn is_out_of_bounds(result: Result<(), UserCodeRunnerError>, addr: u32, len: u32) -> bool {
    matches!(
        result,
        Err(UserCodeRunnerError::GuestMemoryOutOfBounds { addr: a, len: l }) if a == addr && l == len
    )
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn host_access_outside_guest_memory_is_an_error() {
    let output = transpile(native(), &TranspilationMode::default());
    let mut vm = load(&output.elf_bytes, LoaderContext::Untrusted).expect("math-test should load");
    let mut runner = TranspiledCodeRunner::new(&mut vm);
    let stack = runner.virtual_stack_range();
    let stack_bottom = stack.start().as_u32();
    let mut buf = [0_u8; 4];

    // The code at address 0 is readable but not writable.
    assert!(
        runner
            .read_memory(VirtualDataPointer::new(0), &mut buf)
            .is_ok()
    );
    assert!(is_out_of_bounds(
        runner.write_memory(VirtualDataPointer::new(0), &buf),
        0,
        4
    ));

    // The stack is writable.
    assert!(
        runner
            .write_memory(VirtualDataPointer::new(stack_bottom), &buf)
            .is_ok()
    );

    // A range that ends past the guest's memory, or wraps around, cannot be read.
    let mut big = vec![0_u8; 0x0100_0000];
    assert!(is_out_of_bounds(
        runner.read_memory(VirtualDataPointer::new(0), &mut big),
        0,
        0x0100_0000
    ));
    assert!(is_out_of_bounds(
        runner.read_memory(VirtualDataPointer::new(u32::MAX - 1), &mut buf),
        u32::MAX - 1,
        4
    ));
}
