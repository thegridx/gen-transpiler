# transpiler

This crate converts RISC-V 32-bit (RV32IMA) ELF binaries into native aarch64 or x86_64 ELF
files. The output runs directly on the host CPU, but it stays sandboxed and gas-metered, so a
host can run code it does not trust at close to native speed.

This document describes how the transpiled code works. For build instructions, the CLI and a
quick start, read the [repository README](../../README.md).

- [Goals](#goals)
- [Pipeline](#pipeline)
- [Output layout](#output-layout)
- [Sandboxing](#sandboxing)
- [Gas metering](#gas-metering)
- [Calling into transpiled code](#calling-into-transpiled-code)
- [Ecalls](#ecalls)
- [Trusted mode](#trusted-mode)
- [Library API](#library-api)
- [Source map](#source-map)

## Goals

The transpiler must do three things:

- **Translate** every RISC-V instruction into an exact native equivalent.
- **Confine** the program: bounds-check every load and store, and allow jumps only to valid
  targets.
- **Meter** execution: charge gas per RISC-V instruction and stop the program when gas runs out.

The output also has these properties:

- **Linkage-free:** it has no relocations, PLT, GOT or dynamic-linker tables. All addresses
  are resolved at transpile time, and all jumps and memory accesses are PC-relative.
- **Deterministic:** the output depends only on the input ELF, never on the host.
- **Self-describing:** a small `.transpiled.symbols` section tells the loader
  where the entry points and data regions are.

The transpiler writes the ELF itself with the `object` crate, so it needs no external
assembler or linker.

## Pipeline

```text
RISC-V ELF ──► ELF reader ──► instruction transpiler ──► program transpiler ──► ELF builder ──► native ELF
              (riscv32-elf)  (transpile_rv32ima_inst)    (transpile_rv_prog)   (elf_builder)
```

1. **Read the input.** The `riscv32-elf` crate checks that the file is a little-endian
   32-bit RISC-V ELF. It reads `.text`, `.rodata`, `.data`, `.bss` and `.init_array`, checks
   that the named entry points exist, and reads the layout symbols that the linker script
   defines (`_text_end_value`, `_data_start_value`, `_stack_top_value`, `_data_end_value`).
2. **Translate instructions.** Each 32-bit instruction becomes a short sequence of native
   operations with a gas cost. The transpiler supports RV32I, M (multiply and divide) and A
   (atomics). It rejects compressed (`C`) instructions, so compile the input without the C
   extension. Atomics compile to plain loads and stores, because transpiled code is
   single-threaded. For the same reason `FENCE` compiles to nothing, and the `aq`/`rl` ordering
   bits of atomics are ignored. `SC.W` always succeeds, even without a matching `LR.W` and
   across an ecall: with one thread, no other write can break the reservation. `FENCE.I`
   (Zifencei) is not supported.
3. **Form pseudo basic blocks.** Branch targets split the code into blocks. Each block starts
   with one gas deduction for all its instructions.
4. **Build the jump table.** The table maps every RISC-V instruction to its native address. A
   jump through a register (`JALR`) and a return from an ecall both go through this table.
5. **Add the envelope.** Stubs around the program save and restore host state, dispatch to
   entry points, and provide the exits: normal, panic and illegal instruction.
6. **Resolve branches.** Branches use symbolic targets until all code sizes are known. Two
   passes then replace each one with a PC-relative offset.
7. **Write the ELF.** The builder lays out the sections in four loadable segments, checks the
   layout against the input's linker-script symbols, and writes `.transpiled.symbols`.

## Output layout

The input must be linked with [`data/linker_scripts/riscv_elf_mem_layout.x`](../../data/linker_scripts/riscv_elf_mem_layout.x)
or an equivalent script. The output keeps the same guest address space:

```text
Segment 0 (R+X)       .text                 transpiled code and envelope
Segment 1 (R)         .jump_table           one entry per RISC-V instruction, plus an exit sentinel
                      .transpiled.symbols   entry-point and region addresses for the loader
                      .original_risc_prog   the original RISC-V code, readable as data
                      .rodata               original read-only data
Segment 2 (R+W)       .stack                guest stack (size from the linker script)
Segment 3 (R+W)       .data, .bss           original writable data
                      .heap                 guest heap (size from the linker script)
                      .ecall_data           ecall return buffer (64 KiB)
                      .register_store       saved guest registers and gas (104 bytes)
```

The linker script sets the stack and heap sizes, 64 KiB each by default; the stack size must be
a multiple of 16 KiB. `.stack` is the lowest writable address, so a stack underflow faults against read-only memory
instead of corrupting `.data`. Segments are aligned to 16 KiB, which is at least the page size
of every supported host.

The native code section may be at most 4 MiB, and the whole image up to the register store at
most 8 MiB.

## Sandboxing

- **Loads** may read from the start of `.original_risc_prog` to the end of `.ecall_data`.
- **Stores** may write from the start of `.stack` to the end of `.ecall_data`.
- **Direct branches** (`B*`, `JAL`) are checked at transpile time. One whose target is
  misaligned or past `.text` takes the panic exit, but only if it is taken, as the RISC-V spec
  requires. One whose target is exactly the end of `.text` takes the illegal-instruction exit,
  like falling off the last instruction.
- **Indirect jumps** (`JALR`) must target a RISC-V instruction in the jump table and be 4-byte
  aligned.
- **Atomics** (AMOs, `LR` and `SC`) use the same checks as loads and stores, and must be
  4-byte aligned.

Any failed check jumps to the panic exit. Invalid code in `.text`, such as embedded data,
jumps to the illegal-instruction exit. The program can reach the outside world only through
ecalls.

## Gas metering

Gas lives in a dedicated 64-bit host register. At the start of each pseudo basic block, and at
each jump through the jump table, the code subtracts the block's cost and checks for underflow.
When gas runs out, the code jumps to the panic exit with a negative gas value, which the loader
reports as out-of-gas.

A jump through the jump table can land in the middle of a block. Each jump-table entry
therefore holds the cost of the rest of the block from that instruction, next to the native
offset:

```text
┌─────────────────────┬───────────────────────────┐
│ gas cost (4 bytes)  │ relative offset (4 bytes) │
└─────────────────────┴───────────────────────────┘
```

`RvGasCostScheme` in the `riscv32` crate sets the cost. `Fixed(n)` charges `n` per
instruction, and `PerInstruction` looks the cost up per opcode; today every opcode costs 1 there
too. The default is `Fixed(1)`.

Gas is saved in `.register_store` whenever the program stops, and loaded again when it resumes.
The loader can read or change it between runs.

## Calling into transpiled code

The loader enters the program through the envelope with eight argument registers, a0 to a7:

- **a7** selects what to run. A value above 0 is the index of an init function or an entry
  point, and the function sees it in a7. A value of 0 means "return from the last ecall".
- **a0 to a6** are passed to the program unchanged.

Init functions come first, in `.init_array` order, then the entry points in byte order of
their names. `TranspilerOutput::entry_point_mapping` gives each entry point's index. If the
input has an `.init_array`, the loader must run each init function once, before the first
entry point. `user_code_runner::runner::run_init_array` does this.

The program runs until it issues an ecall, exits normally, or takes the panic or
illegal-instruction exit. The envelope then saves the guest state in `.register_store`,
restores the host state, and returns to the loader. The value in the returned a7 tells the
loader which of these happened.

## Ecalls

An `ECALL` instruction stops the program and hands its a0 to a7 registers to the loader, with
the ecall number in a7. The numbers 0, -1 and -2 are the exit values, so an ecall with one of
them takes the illegal-instruction exit instead. The loader handles the request and then
re-enters the program with a7 = 0 and any results in a0 to a6. The envelope restores the guest
state, puts the ecall number back in a7, as the RISC-V ABI requires, and continues at the
instruction after the `ECALL`.

Larger results go in the `.ecall_data` buffer, which the guest can read. Between entries,
the loader can also read and write guest memory and the saved registers, including gas. The
helpers for this are in the `execute` module.

## Trusted mode

`TranspilationMode::Trusted` emits a bare-bones variant for code that is already trusted. It drops
gas metering and all bounds checks except the `JALR` alignment check, and its jump-table
entries hold only the 4-byte offset. Run trusted output only in-process, and only with
trusted input.

Without bounds checks, an access outside the guest's memory is undefined, and the targets
compute such an address differently: aarch64 adds the sign-extended register and the offset
to the guest base, while x86_64 adds them in 32 bits and zero-extends the sum. Accesses within
the guest's memory behave the same on both.

Trusted output is marked in the ELF header: its `e_flags` field holds `EF_TRUSTED_TRANSPILED`
(the ASCII tag `TRST`), and untrusted output leaves it at 0. The loader reads the flag before it
maps any code, and rejects any other value. Loading with `LoaderContext::Untrusted` rejects a
trusted artifact with `LoaderError::TrustedArtifactInUntrustedContext`.

The flag guards against loading the wrong artifact by mistake. It is not a security boundary:
anyone who can change an artifact can clear the flag or replace its code. The
[security model](../../README.md#security-model) explains which artifacts a host may load.

## Library API

```rust
use std::collections::HashSet;
use transpiler::{TranspilerInput, TranspilationMode, TranspilerForArchitecture};

let transpiler = TranspilerForArchitecture::get_native().ok_or("unsupported host")?;
let output = transpiler.transpile_to_native_elf(
    TranspilerInput {
        riscv_elf_bytes: &riscv_elf_bytes,
        entry_points: HashSet::from(["main".to_string()]),
    },
    &TranspilationMode::default(),
)?;
// output.elf_bytes: the native ELF
// output.entry_point_mapping: entry-point name -> index to pass in a7

// Load it and build a runner (see the repository README for the remaining steps).
use transpiler::execute::{LoaderContext, TranspiledCodeRunner, TranspiledVm};
let mut vm = TranspiledVm::load(
    &output.elf_bytes,
    LoaderContext::Untrusted,
    &output.entry_point_mapping,
)?;
let mut runner = TranspiledCodeRunner::new(&mut vm);
```

The transpiler runs on any host and can target either architecture. The output runs only on
the architecture it was built for.

The default `cli` feature builds the `transpiler` command. A library user can depend on the
crate with `default-features = false` to leave out the CLI's dependencies (`clap` and
`env_logger`).

## Source map

| Path | Contents |
|---|---|
| `src/lib.rs` | Public API: `TranspilerInput`, `TranspilerOutput`, `TranspilationMode`, `TranspilerForArchitecture` |
| `src/main.rs` | The `transpiler` CLI |
| `src/transpile_rv32ima_inst.rs` | Per-instruction decoding and dispatch to the target architecture |
| `src/transpile_rv_prog.rs` | Blocks, gas, jump table, envelope and label resolution |
| `src/layout.rs` | Section identifiers, labels and layout validation |
| `src/elf_builder.rs` | ELF64 writer |
| `src/traits.rs` | Traits each target architecture implements |
| `src/arch64/`, `src/x86_64/` | Instruction encoders, register maps and envelopes for each target |
| `src/execute/` | Loader: validates an output ELF, loads it and runs it |
| `examples/test_loader/` | Example loader that maps and runs transpiled programs |
