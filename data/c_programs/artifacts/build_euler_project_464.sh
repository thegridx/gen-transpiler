#!/usr/bin/env bash

# Rebuild the vendored `euler_project_464.test_elf` artifact consumed by
# the `c-math` suite in `transpiler_smoke_tests.py` (the suite
# renames it to `with_e_euler_project_464.o` at copy time; no separate
# unlinked `.o` is vendored).
#
# The test does not compile the C source on the fly, because gas usage is
# counted in dynamic RISC-V instructions (`DEFAULT_GAS_COST_SCHEME =
# Fixed(1)`), and even minor gcc version drift between hosts shifts a few
# one-shot codegen instructions, breaking the expected-gas assertion.
# Mirrors the pattern used by `data/rust_programs/math-test/artifacts/`.
#
# Rerun this script after intentionally editing the C source, then update
# `expected_gas_used` in `c_math_test_modes` to whatever the new artifact
# reports.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
C_DIR="${SCRIPT_DIR}/.."
LINKER_SCRIPTS_DIR="${SCRIPT_DIR}/../../linker_scripts"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

riscv64-unknown-elf-gcc \
    -O1 -DEP464_INT=int -nostdlib -nostartfiles \
    -march=rv32ima -mabi=ilp32 \
    -ffixed-t4 -ffixed-t5 -ffixed-t6 \
    -ffreestanding -fno-pic -fno-pie -fno-plt \
    -mcmodel=medany -r \
    -o "${TMP_DIR}/euler_project_464.test_obj" \
    "${C_DIR}/euler_project_464.c" \
    "${C_DIR}/euler_project_464_no_malloc.c"

# `-z max-page-size` aligns the PT_LOAD segments to the 16 KiB the linker script's
# `ALIGN(16K)` directives use, as the Rust build does (see math-test's build.rs).
riscv64-unknown-elf-ld \
    -T "${LINKER_SCRIPTS_DIR}/riscv_elf_mem_layout.x" \
    -m elf32lriscv \
    -z max-page-size=0x4000 \
    "${TMP_DIR}/euler_project_464.test_obj" \
    -o "${SCRIPT_DIR}/euler_project_464.test_elf"

echo "Wrote ${SCRIPT_DIR}/euler_project_464.test_elf"
