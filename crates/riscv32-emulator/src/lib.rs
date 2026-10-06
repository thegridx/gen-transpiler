//! An interpreter for RV32IMA programs that follows the transpiler's rules.
//!
//! [`Emulator`] loads the same linked RISC-V ELF the transpiler reads and runs it on any host,
//! with the behavior of untrusted transpiled code: the same memory ranges, gas blocks and
//! charges, ecall ABI and exit values. It implements the `user_code_runner` traits, so a host
//! runs it with `call_entry_point` and serves its ecalls with the same `EcallHandler` it uses
//! for transpiled code.
//!
//! Use it to run a program where transpiled code cannot run, and as a reference model: a
//! program that behaves differently in the emulator and when transpiled is a bug in one of them.
mod emulator;
mod error;
mod memory;
mod program;

pub use emulator::{Emulator, GasMetering};
pub use error::EmulatorError;
