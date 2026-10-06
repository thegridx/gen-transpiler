# riscv32-emulator

An interpreter for RV32IMA programs that follows the transpiler's rules.

`Emulator::load` reads the same linked RISC-V ELF the transpiler reads, and runs it on any
host, including the ones transpiled code cannot run on. It behaves like the program transpiled
in untrusted mode:

- **Memory:** loads may read `[0, _data_end_value)` and stores may write
  `[_data_start_value, _data_end_value)`; anything else takes the panic exit. Code is decoded
  once at load time, so data never runs as code.
- **Gas:** the program is split into the same pseudo basic blocks as the transpiler's output
  (`riscv32::blocks`), and each block is charged on entry. A jump into the middle of a block is
  charged the rest of the block. `GasMetering::Unmetered` charges nothing, like trusted output,
  but still checks every memory access.
- **Calls, ecalls and exits:** the same `a7` protocol, reserved ecall numbers, resume rules and
  exit values.

It implements the `user_code_runner` traits, so a host drives it exactly like a
`TranspiledCodeRunner`:

```rust
use riscv32_emulator::{Emulator, GasMetering};
use user_code_runner::runner::{call_entry_point, CodeRunner as _, MemoryAccessor as _};

let mut emulator = Emulator::load(&riscv_elf_bytes, &entry_points, GasMetering::default())?;
let stack_top = emulator.virtual_stack_range().end();
emulator.set_sp(stack_top);
emulator.set_gas(gas);
let outcome = call_entry_point(&mut emulator, &mut ecall_handler, &mut registers)?;
```

## As a reference model

Because it follows the same rules, any difference between a program's behaviour in the emulator
and its behaviour when transpiled is a bug in one of them. Two tests rely on this:

- `crates/transpiler/tests/emulator_differential.rs` runs thousands of random programs in both
  and compares the outcome, every register, the gas left, the ecalls served and the writable
  memory.
- The smoke tests run every test program in the emulator too, with the same expected results
  and gas.

What this cannot catch: the emulator and the transpiler share the instruction decoder, the
gas-block rules (`riscv32::blocks`) and the ELF reader, so a bug there affects both alike and
the runs still agree. Those parts have their own checks: the decoder is tested against the
system assembler and fuzzed for stability, and the reader and transpiler are fuzzed with
malformed input (see `fuzz/`).
