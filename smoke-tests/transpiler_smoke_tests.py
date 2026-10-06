#!/usr/bin/env python3

import argparse
import json
import os
import platform as _platform
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TypedDict

from elftools.elf.constants import SH_FLAGS
from elftools.elf.elffile import ELFFile

# ──────────────────────────────────────────────────────────────────────────────
# Platform configuration
# ──────────────────────────────────────────────────────────────────────────────

PLATFORM_CONFIG = {
    "arm64": {
        "target_triple": "aarch64-unknown-linux-gnu",
        "obj_suffix": "arm64",
        "docker_platform": "linux/arm64",
        "native_arch": "aarch64",
    },
    "x86_64": {
        "target_triple": "x86_64-unknown-linux-gnu",
        "obj_suffix": "x86_64",
        "docker_platform": "linux/amd64",
        "native_arch": "x86_64",
    },
}

# Number of writable (WA) allocatable sections in the transpiled ELF output per asm test:
# `.stack`, `.heap`, `.ecall_data`, `.register_store` (4), plus `.data` / `.bss` when the program
# has them. Used to build the expected per-section flag pattern for both the regular and trusted
# variants (which share the same section structure).
ELF_WA_COUNTS: dict[str, int] = {
    "addi_tests": 4,
    "andi_tests": 4,
    "arithmetic_tests": 4,
    "atomic_tests": 5,
    "auipc_lui_tests": 4,
    "branches2_tests": 4,
    "branches_tests": 4,
    "ecall_tests": 4,
    "guest_readable_range_tests": 4,
    "invalid_branch_target_tests": 4,
    "jumps_tests": 4,
    "load_store_tests": 6,
    "loader_mem_ops_tests": 5,
    "logical_tests": 4,
    "memory_ordering_tests": 5,
    "mul_tests": 4,
    "ori_tests": 4,
    "register_store_read_write_tests": 4,
    "t3456_arithmetic_tests": 4,
    "t3456_branches_tests": 4,
    "t3456_jal_jalr_tests": 4,
    "t3456_load_store_tests": 4,
    "t3456_logical_tests": 4,
    "t3456_lui_auipc_tests": 4,
    "t3456_mul_div_tests": 4,
    "t3456_shifts_tests": 4,
    "verify_register_store_tests": 4,
    "xori_tests": 4,
    "zero_operand_tests": 5,
}

# Gas each asm test's `_test_start` uses in the regular (metered) variant. Gas is charged per
# RISC-V instruction, so it is the same on every target.
ASM_EXPECTED_GAS: dict[str, int] = {
    "addi_tests": 17,
    "andi_tests": 43,
    "arithmetic_tests": 131,
    "atomic_tests": 718,
    "auipc_lui_tests": 144,
    "branches2_tests": 83,
    "branches_tests": 135,
    "ecall_tests": 1435,
    "guest_readable_range_tests": 183,
    "invalid_branch_target_tests": 14,
    "jumps_tests": 115,
    "load_store_tests": 188,
    "loader_mem_ops_tests": 456,
    "logical_tests": 174,
    "memory_ordering_tests": 56,
    "mul_tests": 116,
    "ori_tests": 27,
    "register_store_read_write_tests": 73,
    "t3456_arithmetic_tests": 105,
    "t3456_branches_tests": 63,
    "t3456_jal_jalr_tests": 70,
    "t3456_load_store_tests": 90,
    "t3456_logical_tests": 88,
    "t3456_lui_auipc_tests": 60,
    "t3456_mul_div_tests": 180,
    "t3456_shifts_tests": 86,
    "verify_register_store_tests": 77,
    "xori_tests": 43,
    "zero_operand_tests": 128,
}

# Asm tests that also carry a `.rodata` section, which adds one read-only (A) section.
ELF_RODATA_TESTS: set[str] = {"guest_readable_range_tests"}

# ──────────────────────────────────────────────────────────────────────────────
# Test context — shared state threaded once instead of repeated across every call
# ──────────────────────────────────────────────────────────────────────────────


@dataclass
class TestContext:
    script_dir: Path
    docker_cmd: list[str]
    temp_dir: Path
    loader_rel: Path
    libs_path: Path
    platform: str  # "arm64" or "x86_64"
    obj_suffix: str  # "arm64" or "x86_64"


# ──────────────────────────────────────────────────────────────────────────────
# Platform helpers
# ──────────────────────────────────────────────────────────────────────────────


def _darwin_host_arch() -> str:
    """Return the host CPU architecture on Darwin, ignoring Rosetta translation.

    `platform.machine()` reports the *interpreter's* arch, which is `x86_64`
    when Python itself runs under Rosetta on Apple Silicon. `sysctl -n
    hw.optional.arm64 == 1` reflects the actual CPU, which is what we care
    about when deciding whether to skip the Linux cross-build of test_loader.
    """
    try:
        out = subprocess.run(
            ["sysctl", "-n", "hw.optional.arm64"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        return "arm64" if out == "1" else "x86_64"
    except (subprocess.CalledProcessError, FileNotFoundError):
        return _platform.machine()


def detect_platform() -> str:
    arch = _platform.machine()
    os_name = _platform.system()
    if os_name == "Linux":
        with open("/etc/os-release", encoding="utf-8") as f:
            content = f.read()
        if re.search(r"^ID=ubuntu\b", content, re.M):
            if arch == "aarch64":
                return "arm64"
            if arch == "x86_64":
                return "x86_64"
    if os_name == "Darwin" and _darwin_host_arch() == "arm64":
        return "arm64"
    raise NotImplementedError(f"Unsupported platform: {arch} on {os_name}")


def host_is_native(platform_key: str) -> bool:
    """True when the requested target matches the host OS+arch closely enough
    to run `test_loader` directly (no docker). Linux requires arch match; on
    macOS we only support the arm64 target."""
    cfg = PLATFORM_CONFIG[platform_key]
    if _platform.system() == "Linux" and _platform.machine() == cfg["native_arch"]:
        return True
    if (
        _platform.system() == "Darwin"
        and _darwin_host_arch() == "arm64"
        and platform_key == "arm64"
    ):
        return True
    return False


def make_docker_cmd(workdir: Path, platform_key: str) -> list[str]:
    cfg = PLATFORM_CONFIG[platform_key]
    if host_is_native(platform_key):
        return []
    # Mount the repository root (1 level up) so that ../target paths work
    repo_root = workdir.parent
    return [
        "docker",
        "run",
        "--rm",
        "-v",
        f"{repo_root}:/workspace",
        "-w",
        "/workspace/smoke-tests",
        "--platform",
        cfg["docker_platform"],
        "ubuntu:24.04",
    ]


# ──────────────────────────────────────────────────────────────────────────────
# Build helpers
# ──────────────────────────────────────────────────────────────────────────────


# Rust image used to build `test_loader` for a platform the host cannot run natively.
# Must match the channel in the repository's `rust-toolchain.toml`.
DOCKER_RUST_IMAGE = "rust:1.98.1"
DOCKER_RUST_TOOLCHAIN = "1.98.1"
# Named volume that caches the crates.io registry across Docker builds.
DOCKER_CARGO_REGISTRY_VOLUME = "transpiler-smoke-cargo-registry"


def docker_loader_rel(platform_key: str) -> Path:
    """Path of the Docker-built `test_loader`, relative to `script_dir`."""
    return Path(f"../target-docker/{platform_key}/release/examples/test_loader")


def build_loader_in_docker(*, script_dir: Path, platform_key: str) -> None:
    """Build `test_loader` inside a container of the requested platform.

    Used when the host cannot run the target platform natively, so no host
    cross-toolchain is needed. The build writes to `target-docker/<platform>/`
    to keep its output apart from the host `target/` directory.
    """
    cfg = PLATFORM_CONFIG[platform_key]
    repo_root = script_dir.parent
    cmd = [
        "docker",
        "run",
        "--rm",
        "--platform",
        cfg["docker_platform"],
        "-v",
        f"{repo_root}:/workspace",
        "-v",
        f"{DOCKER_CARGO_REGISTRY_VOLUME}:/usr/local/cargo/registry",
        "-w",
        "/workspace",
        # Use the image's toolchain as-is instead of letting rustup install
        # the extra components `rust-toolchain.toml` lists.
        "-e",
        f"RUSTUP_TOOLCHAIN={DOCKER_RUST_TOOLCHAIN}",
        "-e",
        f"CARGO_TARGET_DIR=/workspace/target-docker/{platform_key}",
        DOCKER_RUST_IMAGE,
        "cargo",
        "build",
        "-p",
        "transpiler",
        "--example",
        "test_loader",
        "--release",
    ]
    print(f"Building loader in Docker: {' '.join(cmd)}", file=sys.stderr)
    subprocess.run(cmd, cwd=script_dir, check=True, text=True)


def run_build_steps(
    *, script_dir: Path, target: str | None, docker_platform_key: str | None
) -> None:
    # Pin cargo to `script_dir` so the script works when invoked by path
    # from outside the workspace; cargo walks up from there to the
    # repository's workspace root and picks up its rust-toolchain.
    subprocess.run(
        ["cargo", "build", "-p", "transpiler", "--release"],
        cwd=script_dir,
        check=True,
        text=True,
    )
    # When `target` is None we build `test_loader` for the host triple; on
    # macOS the Linux cross-build fails (undefined `statx` against the
    # cross-toolchain glibc) and is unnecessary because we run the loader
    # natively against the Linux ELF artifacts.
    if docker_platform_key is not None:
        build_loader_in_docker(script_dir=script_dir, platform_key=docker_platform_key)
        return
    loader_cmd = ["cargo", "build", "-p", "transpiler", "--example", "test_loader", "--release"]
    if target is not None:
        loader_cmd += ["--target", target]
    subprocess.run(loader_cmd, cwd=script_dir, check=True, text=True)


# ──────────────────────────────────────────────────────────────────────────────
# ELF analysis helpers
# ──────────────────────────────────────────────────────────────────────────────


def _section_flags(sh_flags: int) -> str:
    flag_letters = [
        (SH_FLAGS.SHF_WRITE, "W"),
        (SH_FLAGS.SHF_ALLOC, "A"),
        (SH_FLAGS.SHF_EXECINSTR, "X"),
        (SH_FLAGS.SHF_MERGE, "M"),
        (SH_FLAGS.SHF_STRINGS, "S"),
        (SH_FLAGS.SHF_INFO_LINK, "I"),
        (SH_FLAGS.SHF_LINK_ORDER, "L"),
        (SH_FLAGS.SHF_GROUP, "G"),
        (SH_FLAGS.SHF_TLS, "T"),
        (SH_FLAGS.SHF_EXCLUDE, "E"),
    ]
    return "".join(letter for bit, letter in flag_letters if sh_flags & bit)


def get_section_struct(lib_path: Path, *, cwd: Path) -> list[str]:
    """
    Get the section flags from the ELF file and return them as a list of strings,
    where each string is a section flag. Only sections marked as allocatable (A) are considered.
    """
    lib_abs = lib_path if lib_path.is_absolute() else cwd / lib_path
    section_flags: list[str] = []
    with open(lib_abs, "rb") as lib_file:
        elf = ELFFile(lib_file)
        for section in elf.iter_sections():
            flags = _section_flags(int(section.header["sh_flags"]))
            if "A" not in flags:
                continue
            section_flags.append(flags)
    return section_flags


def expected_match(section_struct: list[str], pattern: list[str]) -> bool:
    if len(section_struct) != len(pattern):
        print(
            f"section_struct and pattern have different lengths: expected "
            f"{len(pattern)} but got {len(section_struct)}",
            file=sys.stderr,
        )
        return False
    for section_flag, pattern_flag in zip(section_struct, pattern, strict=True):
        if section_flag != pattern_flag:
            print(
                f"section_struct and pattern have different flags: expected "
                f"{pattern_flag} but got {section_flag}",
                file=sys.stderr,
            )
            return False
    return True


class SectionInfo(TypedDict):
    """Address, size and flag string of one ELF section."""

    vaddr: int
    size: int
    flags: str


def get_section_info(so_path: Path) -> dict[str, SectionInfo]:
    """Get section info from an ELF file: {name -> {vaddr, size, flags}}."""
    sections: dict[str, SectionInfo] = {}
    with open(so_path, "rb") as f:
        elf = ELFFile(f)
        for section in elf.iter_sections():
            name = section.name
            if name:
                sections[name] = {
                    "vaddr": section["sh_addr"],
                    "size": section["sh_size"],
                    "flags": _section_flags(section["sh_flags"]),
                }
    return sections


def get_elf_flags(elf_path: Path) -> int:
    """Return the ELF header `e_flags` field."""
    with open(elf_path, "rb") as f:
        return int(ELFFile(f).header["e_flags"])


def section_bytes(elf_path: Path, name: str) -> bytes:
    """Extract a named section's raw bytes from an ELF file (empty if absent or NOBITS)."""
    with open(elf_path, "rb") as f:
        elf = ELFFile(f)
        section = elf.get_section_by_name(name)
        return section.data() if section is not None else b""


# ──────────────────────────────────────────────────────────────────────────────
# Regular vs trusted comparison constants
# ──────────────────────────────────────────────────────────────────────────────

# ELF `e_flags` value stamped into the trusted (bare-bones) variant; the regular (untrusted)
# variant leaves `e_flags` at 0. Mirrors `EF_TRUSTED_TRANSPILED` in `layout.rs`.
EF_TRUSTED_TRANSPILED: int = int.from_bytes(b"TRST", "little")  # 0x54535254

# Allocatable sections present in every transpiled ELF (regular and trusted alike).
COMMON_SECTIONS: set[str] = {
    ".text",
    ".jump_table",
    ".transpiled.symbols",
    ".original_risc_prog",
    ".ecall_data",
    ".stack",
    ".heap",
    ".register_store",
}

# Optional sections that depend on the program's content.
OPTIONAL_SECTIONS: set[str] = {".rodata", ".data", ".bss"}

# Dynamic-linking / external-linker sections that the pure-Rust ELF builder must never emit
# (the loader maps the segments itself, without the dynamic linker).
FORBIDDEN_SECTIONS: set[str] = {
    ".gnu.hash",
    ".got",
    ".plt",
    ".symtab",
    ".strtab",
    ".dynsym",
    ".dynstr",
    ".hash",
    ".dynamic",
    ".rela.dyn",
}

# Sections whose raw bytes must be byte-identical between the two variants: they are copied
# verbatim from the same input RISC-V program and are independent of the gas/bounds codegen.
IDENTICAL_CONTENT_SECTIONS: tuple[str, ...] = (".original_risc_prog", ".rodata", ".data")

# Expected ascending-vaddr order of allocatable sections. `.stack` is its own R+W segment placed
# below `.data` (the lowest writable address) so a stack underflow faults against rodata.
EXPECTED_SECTION_ORDER: list[str] = [
    ".text",
    ".jump_table",
    ".transpiled.symbols",
    ".original_risc_prog",
    ".rodata",
    ".stack",
    ".data",
    ".bss",
    ".heap",
    ".ecall_data",
    ".register_store",
]

# Symbol names in .transpiled.symbols order. The order matches the .transpiled.symbols binary layout
# (8 consecutive LE u32 values).
TRANSPILED_SYMBOL_NAMES: tuple[str, ...] = (
    "_start_transpiled_code",
    "__entry_points_hooks_begin",
    "__init_array_end",
    "__entry_points_hooks_end",
    "_text_end_value",
    "_data_start_value",
    "_stack_top_value",
    "_data_end_value",
)


# ──────────────────────────────────────────────────────────────────────────────
# Regular vs trusted comparison
# ──────────────────────────────────────────────────────────────────────────────


def validate_transpiled_symbols(elf_so: Path, test_name: str) -> list[str]:
    """
    Validate the .transpiled.symbols section in a transpiled ELF.
    Checks: exists, exactly 32 bytes, 8 non-zero LE u32 values,
    and _start_transpiled_code falls within the .text section.
    """
    import struct

    errors = []
    with open(elf_so, "rb") as f:
        elf = ELFFile(f)
        transpiled_sym = elf.get_section_by_name(".transpiled.symbols")
        if transpiled_sym is None:
            errors.append(f"[{test_name}] .transpiled.symbols section missing")
            return errors
        data = transpiled_sym.data()
        if len(data) != 32:
            errors.append(
                f"[{test_name}] .transpiled.symbols has wrong size: expected 32, got {len(data)}"
            )
            return errors
        values = struct.unpack("<8I", data)
        for name, val in zip(TRANSPILED_SYMBOL_NAMES, values, strict=True):
            if val == 0:
                errors.append(f"[{test_name}] .transpiled.symbols: {name} is zero")
        # _start_transpiled_code should be within .text
        text = elf.get_section_by_name(".text")
        if text is not None:
            text_start = text.header["sh_addr"]
            text_end = text_start + text.header["sh_size"]
            start_tc = values[0]
            if not (text_start <= start_tc < text_end):
                errors.append(
                    f"[{test_name}] .transpiled.symbols: _start_transpiled_code "
                    f"(0x{start_tc:x}) not within .text [0x{text_start:x}..0x{text_end:x})"
                )
    return errors


def compare_regular_vs_trusted(regular_elf: Path, trusted_elf: Path, test_name: str) -> int:
    """
    Compare the regular (untrusted) and trusted transpiled ELF outputs of the same program.

    Both are produced by the same pure-Rust ELF builder and share the same allocatable section
    structure; they differ by design in the ways asserted here: the trusted variant omits gas
    metering and bounds checks (smaller `.text`), uses a 4-byte (offset-only) jump table instead of
    8-byte entries (half the `.jump_table` size), and is stamped with `EF_TRUSTED_TRANSPILED` in
    its ELF header. Returns the number of comparison failures.
    """
    errors: list[str] = []
    reg = get_section_info(regular_elf)
    tru = get_section_info(trusted_elf)
    reg_names = set(reg)
    tru_names = set(tru)
    allowed = COMMON_SECTIONS | OPTIONAL_SECTIONS | {".shstrtab"}

    for variant, names in (("regular", reg_names), ("trusted", tru_names)):
        for section in COMMON_SECTIONS:
            if section not in names:
                errors.append(f"[{test_name}] {variant}: missing section '{section}'")
        for section in FORBIDDEN_SECTIONS:
            if section in names:
                errors.append(
                    f"[{test_name}] {variant}: unexpected dynamic/linker section '{section}'"
                )
        unexpected = names - allowed
        if unexpected:
            errors.append(f"[{test_name}] {variant}: unexpected sections {sorted(unexpected)}")

    # Both variants expose exactly the same section set.
    if reg_names != tru_names:
        errors.append(
            f"[{test_name}] section sets differ: "
            f"regular-only={sorted(reg_names - tru_names)}, trusted-only={sorted(tru_names - reg_names)}"
        )

    # ELF header flags: regular leaves `e_flags` at 0; trusted is stamped EF_TRUSTED_TRANSPILED.
    reg_flags = get_elf_flags(regular_elf)
    tru_flags = get_elf_flags(trusted_elf)
    if reg_flags != 0:
        errors.append(f"[{test_name}] regular e_flags should be 0, got {reg_flags:#x}")
    if tru_flags != EF_TRUSTED_TRANSPILED:
        errors.append(
            f"[{test_name}] trusted e_flags should be {EF_TRUSTED_TRANSPILED:#x}, got {tru_flags:#x}"
        )

    # Trusted jump table has the same entry count but 4-byte entries vs the regular 8-byte entries,
    # so it is exactly half the size.
    if ".jump_table" in reg and ".jump_table" in tru:
        rj, tj = reg[".jump_table"]["size"], tru[".jump_table"]["size"]
        if rj != tj * 2:
            errors.append(f"[{test_name}] jump_table size: regular {rj} should be 2x trusted {tj}")

    # Trusted omits gas-metering and bounds-check code, so its `.text` is strictly smaller.
    if ".text" in reg and ".text" in tru and tru[".text"]["size"] >= reg[".text"]["size"]:
        errors.append(
            f"[{test_name}] trusted .text ({tru['.text']['size']}) should be smaller than "
            f"regular ({reg['.text']['size']})"
        )

    # Sections copied verbatim from the input program must be byte-identical across variants.
    for section in IDENTICAL_CONTENT_SECTIONS:
        if section in reg_names and section in tru_names:
            rb, tb = section_bytes(regular_elf, section), section_bytes(trusted_elf, section)
            if rb != tb:
                errors.append(
                    f"[{test_name}] section '{section}' content differs between variants "
                    f"({len(rb)} vs {len(tb)} bytes)"
                )

    errors += validate_transpiled_symbols(regular_elf, f"{test_name} (regular)")
    errors += validate_transpiled_symbols(trusted_elf, f"{test_name} (trusted)")

    # Per-variant layout invariants: ascending-vaddr section order, `.stack` as the lowest writable
    # section, and the `.heap` -> `.ecall_data` -> `.register_store` contiguous tail.
    order_index = {name: idx for idx, name in enumerate(EXPECTED_SECTION_ORDER)}
    for variant, info in (("regular", reg), ("trusted", tru)):
        present = sorted(
            ((name, d["vaddr"]) for name, d in info.items() if name in order_index),
            key=lambda x: x[1],
        )
        prev_idx = -1
        for name, _vaddr in present:
            idx = order_index[name]
            if idx < prev_idx:
                errors.append(f"[{test_name}] {variant}: section order violation at '{name}'")
            prev_idx = idx

        if (
            ".stack" in info
            and ".data" in info
            and info[".stack"]["vaddr"] >= info[".data"]["vaddr"]
        ):
            errors.append(
                f"[{test_name}] {variant}: .stack (0x{info['.stack']['vaddr']:x}) should be below "
                f".data (0x{info['.data']['vaddr']:x})"
            )

        if ".heap" in info and ".ecall_data" in info:
            heap_end = info[".heap"]["vaddr"] + info[".heap"]["size"]
            if heap_end != info[".ecall_data"]["vaddr"]:
                errors.append(
                    f"[{test_name}] {variant}: .ecall_data NOT immediately after .heap: "
                    f"heap ends at 0x{heap_end:x}, ecall_data starts at 0x{info['.ecall_data']['vaddr']:x}"
                )
        if ".ecall_data" in info and ".register_store" in info:
            ecall_end = info[".ecall_data"]["vaddr"] + info[".ecall_data"]["size"]
            if ecall_end != info[".register_store"]["vaddr"]:
                errors.append(
                    f"[{test_name}] {variant}: .register_store NOT immediately after .ecall_data: "
                    f"ecall_data ends at 0x{ecall_end:x}, register_store starts at 0x{info['.register_store']['vaddr']:x}"
                )

    for error in errors:
        print(error, file=sys.stderr)
    return len(errors)


# ──────────────────────────────────────────────────────────────────────────────
# Test runner helpers
# ──────────────────────────────────────────────────────────────────────────────


def get_cargo_target_dir(script_dir: Path) -> Path:
    """Get the Cargo target directory."""
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=script_dir,
        check=True,
        text=True,
        capture_output=True,
    )
    metadata = json.loads(result.stdout)
    return Path(metadata["target_directory"])


def pre_link_riscv(*, script_dir: Path, temp_dir: Path, lib_name: str) -> None:
    """
    Pre-link a RISC-V object file using riscv_elf_mem_layout.x linker script.
    This adds the required symbols (_text_end_value, _data_start_value, _stack_top_value, _data_end_value)
    that the transpiler expects.
    """
    linker_script = script_dir / ".." / "data" / "linker_scripts" / "riscv_elf_mem_layout.x"
    subprocess.run(
        [
            "riscv64-unknown-elf-ld",
            "-T",
            str(linker_script),
            "-march=rv32ima",
            "-m",
            "elf32lriscv",
            str(temp_dir / f"{lib_name}.o"),
            "-o",
            str(temp_dir / f"with_e_{lib_name}.o"),
        ],
        cwd=script_dir,
        check=True,
        text=True,
    )


def transpile_to_elf(
    *,
    script_dir: Path,
    riscv_obj: Path,
    output_elf: Path,
    entry_points: str,
    platform: str,
    trusted: bool,
) -> None:
    """
    Transpile a RISC-V object to a native ELF using the transpiler. `trusted` selects the
    bare-bones variant (no gas metering / bounds checks, 4-byte jump table) via `--trusted`;
    otherwise the default gas-metered & sandboxed variant is produced.
    """
    transpiler_bin = get_cargo_target_dir(script_dir) / "release" / "transpiler"
    cmd = [
        str(transpiler_bin),
        "--object",
        str(riscv_obj),
        "--comma-separated-entry-points",
        entry_points,
        "--output-elf",
        str(output_elf),
        "--platform",
        platform,
    ]
    if trusted:
        cmd.append("--trusted")
    subprocess.run(cmd, cwd=script_dir, check=True, text=True)


def run_loader(
    ctx: TestContext,
    *,
    lib_rel: str,
    function_codes: str,
    inputs: str,
    expected_outputs: str,
    expected_gas_used: str,
    trusted: bool,
    extra_args: str = "",
) -> bool:
    # Trusted artifacts load only in the loader's trusted context; everything else is loaded
    # the way a host loads untrusted code.
    trusted_flag = "--trusted " if trusted else ""
    bash_line = ctx.docker_cmd + [
        "bash",
        "-lc",
        (
            f'"{ctx.loader_rel}" '
            f"{trusted_flag}"
            f"{extra_args}"
            f'--lib-path "{lib_rel}" '
            f"--comma-separated-function-codes {function_codes} "
            f"--comma-separated-inputs {inputs} "
            f"--comma-separated-expected-outputs {expected_outputs} "
            f"--comma-separated-expected-gas-used {expected_gas_used} "
        ),
    ]
    print(f"Running loader with command: {' '.join(bash_line)}", file=sys.stderr)
    proc = subprocess.run(bash_line, cwd=str(ctx.script_dir), text=True, check=False)
    return proc.returncode == 0


def run_and_check(
    ctx: TestContext,
    *,
    test_name: str,
    lib_path: Path,
    function_codes: str,
    inputs: str,
    expected_outputs: str,
    expected_pattern: list[str],
    expected_gas_used: str,
    trusted: bool,
) -> int:
    ok = run_loader(
        ctx,
        lib_rel=str(lib_path),
        function_codes=function_codes,
        inputs=inputs,
        expected_outputs=expected_outputs,
        expected_gas_used=expected_gas_used,
        trusted=trusted,
    )
    if not ok:
        print(f"{test_name} loader run failed", file=sys.stderr)
        return 1
    section_struct = get_section_struct(lib_path, cwd=ctx.script_dir)
    return 0 if expected_match(section_struct, expected_pattern) else 1


# ──────────────────────────────────────────────────────────────────────────────
# Regular vs trusted test infrastructure
# ──────────────────────────────────────────────────────────────────────────────


def run_modes_test(
    ctx: TestContext,
    *,
    riscv_obj: Path,
    name: str,
    entry_points: str,
    expected_pattern: list[str],
    function_codes: str,
    inputs: str,
    expected_outputs: str,
    expected_gas_used: str,
    run_trusted_behavioral: bool = True,
) -> tuple[int, int]:
    """
    Transpile a RISC-V object in both the regular (untrusted) and trusted variants, compare their
    structure, and run each through the loader.

    The regular run asserts the metered `expected_gas_used`; the trusted variant has no gas metering
    (gas is never deducted), so its behavioral run asserts the same outputs with gas checks disabled
    (all-zero expected gas → the loader skips the gas assertion). Suites that exercise gas/bounds
    guardrails (e.g. out-of-gas, illegal access) pass `run_trusted_behavioral=False`, since the
    trusted variant deliberately lacks those guardrails; they are still structurally compared.

    Returns (loader_failures, structural_comparison_failures).
    """
    libs_path_abs = ctx.script_dir / ctx.libs_path
    libs_path_abs.mkdir(parents=True, exist_ok=True)

    regular_elf = libs_path_abs / f"lib{name}_regular_{ctx.obj_suffix}.elf"
    trusted_elf = libs_path_abs / f"lib{name}_trusted_{ctx.obj_suffix}.elf"
    transpile_to_elf(
        script_dir=ctx.script_dir,
        riscv_obj=riscv_obj,
        output_elf=regular_elf,
        entry_points=entry_points,
        platform=ctx.platform,
        trusted=False,
    )
    transpile_to_elf(
        script_dir=ctx.script_dir,
        riscv_obj=riscv_obj,
        output_elf=trusted_elf,
        entry_points=entry_points,
        platform=ctx.platform,
        trusted=True,
    )

    # Compare the two variants' structure before running the loader.
    compare_fail = compare_regular_vs_trusted(regular_elf, trusted_elf, name)

    # Regular variant: gas-metered, fully checked.
    failures = run_and_check(
        ctx,
        test_name=f"{name} (regular)",
        lib_path=ctx.libs_path / f"lib{name}_regular_{ctx.obj_suffix}.elf",
        function_codes=function_codes,
        inputs=inputs,
        expected_outputs=expected_outputs,
        expected_pattern=expected_pattern,
        expected_gas_used=expected_gas_used,
        trusted=False,
    )

    # Emulator: the RISC-V ELF itself, with the regular variant's outputs and gas. It follows the
    # same rules as untrusted transpiled code, so any difference is a bug in one of them.
    emulated = run_loader(
        ctx,
        lib_rel=os.path.relpath(riscv_obj, ctx.script_dir),
        function_codes=function_codes,
        inputs=inputs,
        expected_outputs=expected_outputs,
        expected_gas_used=expected_gas_used,
        trusted=False,
        extra_args=f"--emulate --comma-separated-entry-points {entry_points} ",
    )
    if not emulated:
        print(f"{name} (emulator) loader run failed", file=sys.stderr)
        failures += 1

    # Trusted variant: same outputs, no gas metering (skip the gas assertion via all-zero gas).
    if run_trusted_behavioral:
        zero_gas = ",".join("0" for _ in function_codes.split(","))
        failures += run_and_check(
            ctx,
            test_name=f"{name} (trusted)",
            lib_path=ctx.libs_path / f"lib{name}_trusted_{ctx.obj_suffix}.elf",
            function_codes=function_codes,
            inputs=inputs,
            expected_outputs=expected_outputs,
            expected_pattern=expected_pattern,
            expected_gas_used=zero_gas,
            trusted=True,
        )

    return failures, compare_fail


# ──────────────────────────────────────────────────────────────────────────────
# Individual test suites
# ──────────────────────────────────────────────────────────────────────────────


def asm_test_modes(s_file: Path, ctx: TestContext, wa_count: int) -> tuple[int, int]:
    """Run a small RISC-V assembly test in both the regular and trusted variants."""
    name = s_file.stem
    subprocess.run(
        [
            "riscv64-unknown-elf-as",
            "-march=rv32ima",
            "-mabi=ilp32",
            str(s_file),
            "-o",
            str(ctx.temp_dir / f"{name}.o"),
        ],
        cwd=ctx.script_dir,
        check=True,
        text=True,
    )
    pre_link_riscv(script_dir=ctx.script_dir, temp_dir=ctx.temp_dir, lib_name=name)
    ro_count = 4 if name in ELF_RODATA_TESTS else 3
    if name not in ASM_EXPECTED_GAS:
        raise KeyError(f"Test file '{name}' not found in ASM_EXPECTED_GAS. Add its gas.")

    # Allocatable sections: .text(AX); .jump_table(A), .transpiled.symbols(A), .original_risc_prog(A),
    # plus .rodata(A) when present; then `wa_count` writable (WA) sections (.stack, .heap, .ecall_data, .register_store, plus
    # .data/.bss when present). Regular and trusted share this structure.
    return run_modes_test(
        ctx,
        riscv_obj=ctx.temp_dir / f"with_e_{name}.o",
        name=name,
        entry_points="_test_start",
        expected_pattern=1 * ["AX"] + ro_count * ["A"] + wa_count * ["WA"],
        function_codes="1",
        inputs="0",
        expected_outputs="0",
        expected_gas_used=str(ASM_EXPECTED_GAS[name]),
    )


def multi_init_test_modes(ctx: TestContext) -> tuple[int, int]:
    """Run multi init test in both the regular and trusted variants."""
    multi_init_dir = ctx.script_dir / ".." / "data" / "rv32im_asm_files2"
    name = "multi_init_functions"
    subprocess.run(
        [
            "riscv64-unknown-elf-as",
            "-march=rv32ima",
            "-mabi=ilp32",
            str(multi_init_dir / f"{name}.s"),
            "-o",
            str(ctx.temp_dir / f"{name}.o"),
        ],
        cwd=ctx.script_dir,
        check=True,
        text=True,
    )
    pre_link_riscv(script_dir=ctx.script_dir, temp_dir=ctx.temp_dir, lib_name=name)
    # `move_sp_to_a0` returns the stack pointer, which equals `_stack_top_value` from
    # `riscv_elf_mem_layout.x`. `.stack` is now the first writable section, segment-aligned right
    # after rodata, so with the test's text fitting inside one 16K segment the layout reduces to:
    #   ALIGN(16K)(text) + 64K(stack) = 0x4000 + 0x10000 = 0x14000 = 81920.
    # `.data`/`.bss` follow the stack and no longer contribute to the stack-top offset.
    stack_top = 81920
    return run_modes_test(
        ctx,
        riscv_obj=ctx.temp_dir / f"with_e_{name}.o",
        name=name,
        entry_points="increment_bss,product_bss,sum_bss,move_sp_to_a0",
        expected_pattern=1 * ["AX"] + 3 * ["A"] + 5 * ["WA"],
        function_codes="6,8,7,5,7,8,6",
        inputs="0,0,0,0,0,0,0",
        expected_outputs=f"{stack_top},17,210,0,576,21,{stack_top}",
        expected_gas_used="2,10,10,16,10,10,2",
    )


# Failure codes the test loader reports in place of an entry point's return value.
PANIC = 1
ILLEGAL = 2
OUT_OF_GAS = 3


@dataclass(frozen=True)
class EntryCase:
    """One entry point of a test program and what calling it must produce."""

    name: str
    # The return value, or a failure code (PANIC, ILLEGAL, OUT_OF_GAS).
    expected_output: int
    # The gas the call uses. 0 skips the check; a negative value starts the call with no gas.
    expected_gas: int = 0


def entry_case_args(cases: list[EntryCase]) -> dict[str, str]:
    """Build the loader arguments for `cases`, for a program without init functions.

    The transpiler numbers entry points from 1 in byte order of their names, so the cases are
    sorted the same way to get each one's function code.
    """
    ordered = sorted(cases, key=lambda case: case.name.encode())
    return {
        "entry_points": ",".join(case.name for case in ordered),
        "function_codes": ",".join(str(i + 1) for i in range(len(ordered))),
        "inputs": ",".join("0" for _ in ordered),
        "expected_outputs": ",".join(str(case.expected_output) for case in ordered),
        "expected_gas_used": ",".join(str(case.expected_gas) for case in ordered),
    }


def failure_test_modes(ctx: TestContext) -> tuple[int, int]:
    """Run the failure suite. Behavioral checks run on the regular variant only — the trusted
    variant deliberately omits the gas/bounds guardrails these cases exercise (out-of-gas, illegal
    read/jump), so it is structurally compared but not executed here."""
    failure_dir = ctx.script_dir / ".." / "data" / "rv32im_asm_files2"
    name = "failure_tests"
    subprocess.run(
        [
            "riscv64-unknown-elf-as",
            "-march=rv32ima",
            "-mabi=ilp32",
            str(failure_dir / f"{name}.s"),
            "-o",
            str(ctx.temp_dir / f"{name}.o"),
        ],
        cwd=ctx.script_dir,
        check=True,
        text=True,
    )
    pre_link_riscv(script_dir=ctx.script_dir, temp_dir=ctx.temp_dir, lib_name=name)
    return run_modes_test(
        ctx,
        riscv_obj=ctx.temp_dir / f"with_e_{name}.o",
        name=name,
        expected_pattern=1 * ["AX"] + 3 * ["A"] + 4 * ["WA"],
        **entry_case_args(FAILURE_CASES),
        run_trusted_behavioral=False,
    )


# The gas of a failing call covers the whole block it fails in: blocks are charged up front.
FAILURE_CASES = [
    EntryCase("illegal_opcode", ILLEGAL, expected_gas=2),
    EntryCase("out_of_gas", OUT_OF_GAS, expected_gas=-1),
    EntryCase("illegal_read", PANIC, expected_gas=4),
    EntryCase("illegal_jump", PANIC, expected_gas=3),
    EntryCase("load_past_end", PANIC, expected_gas=4),
    EntryCase("store_past_end", PANIC, expected_gas=4),
    EntryCase("amo_past_end", PANIC, expected_gas=4),
    EntryCase("taken_branch_misaligned", PANIC, expected_gas=1),
    EntryCase("taken_branch_past_end", PANIC, expected_gas=1),
    EntryCase("jal_misaligned", PANIC, expected_gas=1),
    EntryCase("lr_misaligned", PANIC, expected_gas=3),
    EntryCase("sc_misaligned", PANIC, expected_gas=3),
    EntryCase("store_to_code", PANIC, expected_gas=2),
    EntryCase("amo_to_code", PANIC, expected_gas=2),
    EntryCase("load_wraparound", PANIC, expected_gas=2),
    EntryCase("jalr_to_data", PANIC, expected_gas=2),
    EntryCase("branch_to_end", ILLEGAL, expected_gas=1),
    EntryCase("div_at_end", ILLEGAL, expected_gas=2),
]


def gas_test_modes(ctx: TestContext) -> tuple[int, int]:
    """Run the gas edge-case suite on the regular variant; the trusted variant has no gas."""
    gas_dir = ctx.script_dir / ".." / "data" / "rv32im_asm_files2"
    name = "gas_tests"
    subprocess.run(
        [
            "riscv64-unknown-elf-as",
            "-march=rv32ima",
            "-mabi=ilp32",
            str(gas_dir / f"{name}.s"),
            "-o",
            str(ctx.temp_dir / f"{name}.o"),
        ],
        cwd=ctx.script_dir,
        check=True,
        text=True,
    )
    pre_link_riscv(script_dir=ctx.script_dir, temp_dir=ctx.temp_dir, lib_name=name)
    return run_modes_test(
        ctx,
        riscv_obj=ctx.temp_dir / f"with_e_{name}.o",
        name=name,
        expected_pattern=1 * ["AX"] + 3 * ["A"] + 4 * ["WA"],
        # Each case sets its own gas, so the gas used is not checked.
        **entry_case_args(
            [
                EntryCase("block_exact", 7),
                EntryCase("block_one_short", OUT_OF_GAS),
                EntryCase("mid_block_exact", 9),
                EntryCase("mid_block_one_short", OUT_OF_GAS),
                EntryCase("loop_runs_out", OUT_OF_GAS),
            ]
        ),
        run_trusted_behavioral=False,
    )


def jalr_alignment_test_modes(ctx: TestContext) -> tuple[int, int]:
    """Run the `jalr` target alignment suite in both the regular and trusted variants. The
    misaligned-target trap is the one guardrail the trusted variant keeps, so unlike the failure
    suite this one is executed on both variants."""
    alignment_dir = ctx.script_dir / ".." / "data" / "rv32im_asm_files2"
    name = "jalr_alignment_tests"
    subprocess.run(
        [
            "riscv64-unknown-elf-as",
            "-march=rv32ima",
            "-mabi=ilp32",
            str(alignment_dir / f"{name}.s"),
            "-o",
            str(ctx.temp_dir / f"{name}.o"),
        ],
        cwd=ctx.script_dir,
        check=True,
        text=True,
    )
    pre_link_riscv(script_dir=ctx.script_dir, temp_dir=ctx.temp_dir, lib_name=name)
    # `misaligned_jump` must fail with code 1 (panic); `tagged_jump` returns 7.
    return run_modes_test(
        ctx,
        riscv_obj=ctx.temp_dir / f"with_e_{name}.o",
        name=name,
        entry_points="misaligned_jump,tagged_jump",
        expected_pattern=1 * ["AX"] + 3 * ["A"] + 4 * ["WA"],
        function_codes="1,2",
        inputs="0,0",
        expected_outputs="1,7",
        expected_gas_used="4,6",
    )


def c_math_test_modes(ctx: TestContext) -> tuple[int, int]:
    """Run C math (euler 464) test in both the regular and trusted variants.

    Uses a pre-built, pre-linked `.o` shipped under
    `data/c_programs/artifacts/` — gas usage is counted as dynamic RISC-V
    instructions, so even minor host gcc version drift would shift one-shot
    codegen and break the expected-gas assertion. Rebuild the artifact via
    `data/c_programs/artifacts/build_euler_project_464.sh` when the C source
    changes, then update `expected_gas_used` to the new value.
    """
    artifacts_dir = ctx.script_dir / ".." / "data" / "c_programs" / "artifacts"
    name = "euler_project_464"
    dst_obj = ctx.temp_dir / f"with_e_{name}.o"
    shutil.copy(artifacts_dir / f"{name}.test_elf", dst_obj)
    dst_obj.chmod(dst_obj.stat().st_mode & ~0o111)  # Remove execute permissions
    return run_modes_test(
        ctx,
        riscv_obj=dst_obj,
        name=name,
        entry_points="entry_point",
        expected_pattern=1 * ["AX"] + 3 * ["A"] + 5 * ["WA"],
        function_codes="1",
        inputs="10000",
        expected_outputs="20155319",
        expected_gas_used="27700474",
    )


def rust_math_test_modes(ctx: TestContext) -> tuple[int, int]:
    """Run Rust math test in both the regular and trusted variants."""
    compiled_rust_dir = ctx.script_dir / ".." / "data" / "rust_programs" / "math-test" / "artifacts"
    name = "math_test"
    dst_obj = ctx.temp_dir / f"with_e_{name}.o"
    shutil.copy(compiled_rust_dir / f"{name}.o", dst_obj)
    dst_obj.chmod(dst_obj.stat().st_mode & ~0o111)  # Remove execute permissions
    return run_modes_test(
        ctx,
        riscv_obj=dst_obj,
        name=name,
        entry_points="sum_1_to_x,sum_squares_1_to_x,sum_cubes_1_to_x,factorial_x",
        # No `.data`/`.bss`: the program is dependency-free and has no statics beyond rodata.
        expected_pattern=1 * ["AX"] + 4 * ["A"] + 4 * ["WA"],
        function_codes="4,3,2,1,2",
        inputs="10,10,10,9,9",
        expected_outputs="385,3025,55,362880,45",
        expected_gas_used="62,72,52,47,47",
    )


# ──────────────────────────────────────────────────────────────────────────────
# Main
# ──────────────────────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run transpiler regular-vs-trusted loader test suites"
    )
    parser.add_argument(
        "--tests",
        action="append",
        choices=["asm", "multi-init", "failure", "gas", "jalr-alignment", "c-math", "rust-math"],
        help="Test suites to run. Can be provided multiple times. Defaults to all suites.",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help=(
            "Skip building the transpiler and the example loader. "
            "Also skipped when TRANSPILER_SKIP_BUILD is set to a truthy value."
        ),
    )
    parser.add_argument(
        "--platform",
        choices=list(PLATFORM_CONFIG),
        default=None,
        help="Target platform: arm64 or x86_64. Auto-detected from the host if not specified.",
    )
    args = parser.parse_args()

    platform_key = args.platform or detect_platform()
    cfg = PLATFORM_CONFIG[platform_key]
    target = cfg["target_triple"]

    script_dir = Path(__file__).resolve().parent

    # On macOS we build `test_loader` for the native host (no --target). The
    # loader is run directly against the Linux ELF artifacts, so we don't
    # need the Linux cross-build (which fails on macOS anyway).
    is_macos_native = (
        _platform.system() == "Darwin"
        and _darwin_host_arch() == "arm64"
        and platform_key == "arm64"
    )
    loader_build_target = None if is_macos_native else target
    # When the loader is a host binary it lives in `target/release/`; when it
    # was built with `--target <triple>` it lives in `target/<triple>/release/`.
    # A platform the host cannot run natively gets its loader built in Docker.
    docker_platform_key = None if host_is_native(platform_key) else platform_key
    if docker_platform_key is not None:
        loader_rel = docker_loader_rel(docker_platform_key)
    elif loader_build_target is None:
        loader_rel = Path("../target/release/examples/test_loader")
    else:
        loader_rel = Path(f"../target/{loader_build_target}/release/examples/test_loader")

    skip_build = args.skip_build or os.environ.get("TRANSPILER_SKIP_BUILD", "").lower() in {
        "1",
        "true",
        "yes",
    }
    if skip_build:
        print("Skipping build steps (flag/env).")
    else:
        run_build_steps(
            script_dir=script_dir,
            target=loader_build_target,
            docker_platform_key=docker_platform_key,
        )

    temp_dir = script_dir / "temp_transpile"
    temp_dir.mkdir(parents=True, exist_ok=True)

    ctx = TestContext(
        script_dir=script_dir,
        docker_cmd=make_docker_cmd(script_dir, platform_key),
        temp_dir=temp_dir,
        loader_rel=loader_rel,
        libs_path=Path(f"./target/{target}/release"),
        platform=platform_key,
        obj_suffix=cfg["obj_suffix"],
    )

    selected_tests = (
        set(args.tests)
        if args.tests
        else {"asm", "multi-init", "failure", "gas", "jalr-alignment", "c-math", "rust-math"}
    )
    total_failures = total_compare_fails = 0

    if "asm" in selected_tests:
        f = c = 0
        for s_file in sorted((script_dir / ".." / "data" / "rv32im_asm_files").glob("*_tests.s")):
            if s_file.stem not in ELF_WA_COUNTS:
                raise KeyError(
                    f"Test file '{s_file.stem}' not found in ELF_WA_COUNTS. "
                    "Please add an entry for this test."
                )
            df, dc = asm_test_modes(s_file, ctx, ELF_WA_COUNTS[s_file.stem])
            f += df
            c += dc
        print(f"{f} asm tests failed, {c} regular/trusted comparisons failed")
        total_failures += f
        total_compare_fails += c

    for suite_name, suite_fn in [
        ("multi-init", multi_init_test_modes),
        ("failure", failure_test_modes),
        ("gas", gas_test_modes),
        ("jalr-alignment", jalr_alignment_test_modes),
        ("c-math", c_math_test_modes),
        ("rust-math", rust_math_test_modes),
    ]:
        if suite_name not in selected_tests:
            continue
        f, c = suite_fn(ctx)
        label = suite_name.replace("-", " ")
        print(f"{f} {label} tests failed, {c} regular/trusted comparisons failed")
        total_failures += f
        total_compare_fails += c

    print("\n=== SUMMARY ===")
    print(f"Total test failures: {total_failures}")
    print(f"Total regular/trusted comparison failures: {total_compare_fails}")

    if total_compare_fails == 0:
        print("SUCCESS: All regular/trusted section and symbol comparisons passed")
    else:
        print("ERROR: Some regular/trusted section or symbol comparisons failed!", file=sys.stderr)

    return total_failures + total_compare_fails


if __name__ == "__main__":
    sys.exit(main())
