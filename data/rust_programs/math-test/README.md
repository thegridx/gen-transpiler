# `math-test`: Rust test program for the `rust-math` smoke suite

A minimal `no_std` Rust program with four entry points:

| Entry point | Returns |
|---|---|
| `sum_1_to_x(x)` | 1 + 2 + … + x |
| `sum_squares_1_to_x(x)` | 1² + 2² + … + x² |
| `sum_cubes_1_to_x(x)` | 1³ + 2³ + … + x³ |
| `factorial_x(x)` | x! |

Each function takes one `u32` in `a0` and returns one `u32` in `a0`. The
program has no dependencies, no allocator and no init-array constructors, so
the loader numbers the entry points 1 to 4 in alphabetical order:
`factorial_x`, `sum_1_to_x`, `sum_cubes_1_to_x`, `sum_squares_1_to_x`.

## Vendored artifact

`artifacts/math_test.o` is checked in so the smoke tests are
byte-deterministic across hosts. The transpiler meters gas per RISC-V
instruction, so a different compiler version could change the code and break
the `expected_gas_used` values in `rust_math_test_modes` in
`smoke-tests/transpiler_smoke_tests.py`.

## Rebuilding

```sh
./build.sh
```

Requires the `riscv32im-unknown-none-elf` rustup target (installed from
`rust-toolchain.toml`) and `riscv64-unknown-elf-ld` on `PATH`. The build links
with `data/linker_scripts/riscv_elf_mem_layout.x`.

After a rebuild, run the `rust-math` suite. If the gas values change, update
`expected_gas_used` in `rust_math_test_modes`.
