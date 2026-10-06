#!/usr/bin/env bash
# Seeds the fuzz corpora with real inputs, so the fuzzers start from valid programs instead of
# random bytes. Run the smoke tests first: they produce the RISC-V objects and transpiled ELFs.
set -euo pipefail

FUZZ_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${FUZZ_DIR}/.."
CORPUS="${FUZZ_DIR}/corpus"
mkdir -p "${CORPUS}/read_riscv_elf" "${CORPUS}/transpile" "${CORPUS}/load"

riscv_inputs=(
    "${ROOT}/data/rust_programs/math-test/artifacts/math_test.o"
    "${ROOT}/data/c_programs/artifacts/euler_project_464.test_elf"
)
shopt -s nullglob
riscv_inputs+=("${ROOT}"/smoke-tests/temp_transpile/*.o)
for input in "${riscv_inputs[@]}"; do
    cp "${input}" "${CORPUS}/read_riscv_elf/"
    cp "${input}" "${CORPUS}/transpile/"
done

case "$(uname -m)" in
    arm64 | aarch64) native="aarch64-unknown-linux-gnu" ;;
    *) native="x86_64-unknown-linux-gnu" ;;
esac
for elf in "${ROOT}/smoke-tests/target/${native}/release/"*.elf; do
    cp "${elf}" "${CORPUS}/load/"
done

echo "Seeded ${#riscv_inputs[@]} RISC-V inputs and $(ls "${CORPUS}/load" | wc -l) transpiled ELFs."
