# gen-transpiler

gen-transpiler converts RISC-V 32-bit (RV32IMA) ELF programs into native aarch64 or x86_64
ELF files. The output runs directly on the host CPU, but it stays sandboxed and gas-metered:
every memory access is bounds-checked, every indirect jump is validated, and execution stops
when the program runs out of gas. This lets a host run programs it does not trust at close to
native speed.

The repository contains the transpiler library and CLI, a loader that maps and runs the
output, and an example host program. To learn how the transpiled code works, read
[`crates/transpiler/README.md`](crates/transpiler/README.md).

## Repository layout

| Path | Contents |
|---|---|
| `crates/transpiler/` | Transpiler library, loader (`execute` module) and `transpiler` CLI |
| `crates/transpiler/examples/test_loader/` | Example host: loads a transpiled program and runs its entry points |
| `crates/user-code-runner/` | Run loop, and the `CodeRunner` and `EcallHandler` traits a host implements |
| `crates/riscv32-emulator/` | Interpreter that runs a RISC-V program by the same rules as transpiled code, on any host |
| `crates/riscv32/` | RV32IMA instruction model, registers and gas-cost schemes |
| `crates/riscv32-elf/` | RISC-V ELF reader and the register-store layout |
| `data/` | Linker script and test programs in assembly, C and Rust |
| `smoke-tests/` | End-to-end tests that transpile the test programs and run them |
| `fuzz/` | Fuzz targets for the decoder, ELF reader, transpiler, loader and emulator |

## Prerequisites

- **Rust:** `rustup` installs the toolchain pinned in `rust-toolchain.toml`.
- **aarch64 and x86_64 binutils:** the unit tests compare the instruction encoders with the
  system assemblers `aarch64-linux-gnu-{as,ld,objcopy}` and `x86_64-linux-gnu-{as,objcopy}`.
- **RISC-V binutils:** the smoke tests assemble and link test programs with
  `riscv64-unknown-elf-{as,ld}`.
- **uv** with Python 3.12 or later, for the smoke tests.
- **Docker**, only to run the smoke tests for an architecture the host cannot run.

On macOS with Homebrew:

```bash
brew tap riscv-software-src/riscv
brew install riscv-tools
brew tap messense/macos-cross-toolchains
brew install aarch64-unknown-linux-gnu x86_64-linux-gnu-binutils
brew install uv
```

On Ubuntu:

```bash
sudo apt install -y binutils-riscv64-unknown-elf binutils-aarch64-linux-gnu binutils-x86-64-linux-gnu
curl -LsSf https://astral.sh/uv/install.sh | sh
```

## Build and test

```bash
cargo build --release
cargo test --workspace --release
(cd smoke-tests && uv run --locked transpiler_smoke_tests.py)
```

The encoder tests compare randomly generated instructions with the system assemblers. They use
a fixed seed and print it; set `TEST_SEED` (decimal or `0x` hex) to try other cases or to
replay a failure. Tests that run transpiled code are reported as ignored on hosts that cannot
run it: anything but aarch64 and x86_64 Linux.

[`fuzz/`](fuzz/README.md) has coverage-guided fuzz targets, which need a nightly toolchain and
`cargo-fuzz`.

## Preparing a program

The transpiler accepts a RISC-V ELF that meets these rules:

- It is a linked executable that uses only RV32I, M and A instructions: build for `rv32ima`
  with the `ilp32` ABI and without the compressed (`C`) extension.
- It is linked with [`data/linker_scripts/riscv_elf_mem_layout.x`](data/linker_scripts/riscv_elf_mem_layout.x),
  which fixes the memory layout and defines the symbols the transpiler reads.
- Each entry point is a global symbol in `.text`, defined once. In assembly, declare it with
  `.globl`.
- Optionally, keep the compiler off t4 to t6 (GCC: `-ffixed-t4 -ffixed-t5 -ffixed-t6`). Both
  backends keep t3 to t6 in memory rather than in host registers, so code that avoids them
  runs faster. The C example does this.
- It does not read its own `.init_array`. The host runs the init functions (see below), and
  the output does not include the section, so a C runtime that walks
  `__init_array_start`..`__init_array_end` itself, such as newlib's `__libc_init_array`, must
  not be used.

`data/rust_programs/math-test/` is a minimal `no_std` Rust program set up this way, and
`data/c_programs/artifacts/build_euler_project_464.sh` does the same for C.

## Transpiling

```bash
cargo run --release --bin transpiler -- \
  --object program.elf \
  --comma-separated-entry-points main,other \
  --output-elf program.native.elf
```

| Option | Meaning |
|---|---|
| `--platform arm64` or `x86_64` | Target architecture. The default is the host's. |
| `--trusted` | Omit gas metering and bounds checks. Only for code the host already trusts. |

The library API is described in the [crate README](crates/transpiler/README.md#library-api).

## Running transpiled code

A host needs five steps. The example in `crates/transpiler/examples/test_loader/` shows each
of them:

1. Load the native ELF's bytes with `transpiler::execute::TranspiledVm::load`, passing a
   `LoaderContext` and the entry-point mapping from `TranspilerOutput`. The VM owns the loaded program; dropping it unmaps the program.
2. Build a `TranspiledCodeRunner` from the VM. The runner borrows the VM exclusively.
3. Set the stack pointer to the layout's `stack_top()`, and set the starting gas.
4. Implement `user_code_runner::runner::EcallHandler` to serve the program's ecalls: the
   requests it makes to the host through the RISC-V `ECALL` instruction.
5. Call `run_init_array` once, then `call_entry_point` for each call.

To run a program where transpiled code cannot run, or without transpiling it, load the RISC-V
ELF into `riscv32_emulator::Emulator` instead of steps 1 and 2. It implements the same traits,
so the rest is unchanged, and it follows the same rules as untrusted transpiled code: memory
ranges, gas, the ecall ABI and exit values. It is slower than transpiled code.

Memory, the stack pointer and the gas register persist from one call to the next. A call that
ends in a panic, an illegal instruction or running out of gas leaves them wherever it stopped:

- Set the stack pointer back to `stack_top()`, since the failed call never popped its frames.
- Set the gas again; an out-of-gas call leaves it negative.
- Memory may be half-updated, for example an allocator in the middle of an update. A program
  cannot be trusted to recover from that, so load a fresh VM before calling an untrusted
  program again.

Only an ecall can be resumed. Resuming after any other exit takes the panic exit.

## Security model

The sandbox protects the host from the **RISC-V program**. The transpiler treats its input
ELF as untrusted: it rejects malformed input, and the code it emits confines every memory
access and jump and charges gas. Read [`SECURITY.md`](SECURITY.md) to report a way around it.

Keep these limits in mind when you build a host:

- **A native artifact is code, not data.** Loading one runs whatever native instructions it
  contains. The loader checks that an artifact is consistent, but it cannot tell whether its
  code is sandboxed. Load only artifacts the host transpiled itself, or whose integrity it
  verified, for example with a hash recorded at transpile time. `TranspiledVm::load` copies
  the bytes it is given, so check the same bytes you load.
- **The trusted flag is a label.** `LoaderContext::Untrusted` refuses artifacts built with
  `--trusted`, to catch mistakes. Anyone who can change the file can also clear the flag, so it
  is not a security boundary.
- **Unwinders see guest values.** While guest code runs on aarch64, x29 holds the guest's s0
  rather than a frame record, so an in-process unwinder, such as a crash reporter or a
  sampling profiler, would follow pointers the guest controls. Running untrusted programs in a
  separate process avoids this too.
- **Faults end the process.** The loader installs no signal handler. A fault in native code,
  such as an out-of-bounds access in a trusted artifact, terminates the host process. Run
  untrusted programs in a separate process with restricted privileges.
- **Gas bounds run time only if you set it.** A program runs until it exits, makes an ecall
  or runs out of the gas the host gave it.
- **Supported hosts:** Linux on x86_64 and aarch64 with pages of at most 16 KiB, and macOS on
  Apple Silicon. A macOS host with the hardened runtime needs the
  `com.apple.security.cs.allow-unsigned-executable-memory` entitlement, because the loader
  makes the copied code executable.

## Smoke tests

`smoke-tests/transpiler_smoke_tests.py` builds the transpiler and the example host, transpiles
each test program in both the default and trusted modes, checks the ELF layout, and runs every
entry point. It also runs each program in the emulator. It compares each return value and gas
count with the expected value.

```bash
cd smoke-tests
uv run --locked transpiler_smoke_tests.py                      # host architecture
uv run --locked transpiler_smoke_tests.py --platform x86_64    # other architecture, in Docker
uv run --locked transpiler_smoke_tests.py --tests asm --tests c-math
```

| Suite | Programs |
|---|---|
| `asm` | `data/rv32im_asm_files/` |
| `multi-init`, `failure`, `gas`, `jalr-alignment` | `data/rv32im_asm_files2/` |
| `c-math` | `data/c_programs/artifacts/` |
| `rust-math` | `data/rust_programs/math-test/artifacts/` |

## Contact

@fuzzerknowsbest

## License

Source-available under the PolyForm Small Business License 1.0.0. See
[`LICENSE.md`](LICENSE.md).
