# Fuzz targets

Coverage-guided fuzz targets for [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz). They
need a nightly toolchain, so this crate is outside the main workspace.

| Target | Input | Checks |
|---|---|---|
| `decode` | One instruction word | The decoder never panics, and re-encoding a decoded instruction decodes to the same instruction. |
| `read_riscv_elf` | Any bytes | The RISC-V ELF reader rejects what it cannot use with an error, never a panic. |
| `transpile` | Any bytes | The transpiler never panics, for either target and mode, and the loader loads whatever it outputs for this host. |
| `transpile_words` | Instruction words | A well-formed program transpiles for both targets and modes, whatever its instructions, and the output for this host loads. Fuzzes both encoders on any host. |
| `load` | Any bytes | The loader rejects a garbled artifact with an error, never a panic. It maps the artifact but runs nothing. |
| `differential` | Instruction words and a starting state | The program behaves the same in the emulator and transpiled for this host, over an optional init function and two calls, resumed after ecalls the host halts in (see `crates/transpiler/tests/differential/`). |

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
fuzz/seed_corpus.sh                       # optional: start from the test programs
cargo +nightly fuzz run transpile -- -max_total_time=600
```

`seed_corpus.sh` copies the RISC-V programs from `data/` and the smoke tests, and the
transpiled smoke-test ELFs, into `fuzz/corpus/`. Run the smoke tests first so they exist.
Without seeds the ELF targets start from random bytes and take much longer to reach the
interesting code. A crash is saved under `fuzz/artifacts/<target>/`; replay it with
`cargo +nightly fuzz run <target> <file>`.

`transpile`, `transpile_words`, `load` and `differential` load or run transpiled code only on
hosts that can run it: aarch64, or x86_64 Linux. CI builds every target, so they cannot
silently stop compiling.
