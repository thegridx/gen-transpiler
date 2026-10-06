# Prebuilt C test program

`euler_project_464.test_elf` is the input of the `c-math` smoke suite. It is built from
`../euler_project_464.c` and `../euler_project_464_no_malloc.c`.

The file is checked in so the tests give the same result on every host. The transpiler meters
gas per RISC-V instruction, so a different `riscv64-unknown-elf-gcc` version could change the
code and break the expected gas value in `c_math_test_modes` in
`smoke-tests/transpiler_smoke_tests.py`.

## Rebuilding

Rebuild only after you change the C sources, not after a toolchain update:

```sh
./build_euler_project_464.sh
```

The script needs `riscv64-unknown-elf-gcc` and `riscv64-unknown-elf-ld` on `PATH`, and links
with `data/linker_scripts/riscv_elf_mem_layout.x`. It overwrites `euler_project_464.test_elf`.
Then run the `c-math` suite and update `expected_gas_used` in `c_math_test_modes` to the value
the failure reports.
