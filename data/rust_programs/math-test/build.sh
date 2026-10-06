#!/usr/bin/env bash
# Rebuilds artifacts/math_test.o from src/main.rs.
#
# Requires the `riscv32im-unknown-none-elf` rustup target (installed by
# rust-toolchain.toml) and `riscv64-unknown-elf-ld` on PATH.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if ! command -v riscv64-unknown-elf-ld >/dev/null 2>&1; then
    echo "Error: riscv64-unknown-elf-ld not found in PATH" >&2
    exit 1
fi

cargo build --release
cp target/riscv32im-unknown-none-elf/release/math_test artifacts/math_test.o
chmod -x artifacts/math_test.o
echo "Wrote $SCRIPT_DIR/artifacts/math_test.o"
