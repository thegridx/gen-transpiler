use std::{env, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../linker_scripts/riscv_elf_mem_layout.x");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let script_path =
        Path::new(&manifest_dir).join("../../linker_scripts/riscv_elf_mem_layout.x");
    assert!(
        script_path.exists(),
        "riscv_elf_mem_layout.x not found at `{}`",
        script_path.display()
    );
    println!("cargo:rustc-link-arg=-T{}", script_path.display());

    // Align PT_LOAD segments to the same 16 KiB the `ALIGN(16K)` directives in
    // `riscv_elf_mem_layout.x` use. Must match `SEGMENT_BYTE_ALIGN` in
    // `crates/riscv32/src/common.rs`. `-z max-page-size` is a GNU ld misnomer:
    // it controls segment alignment in the output ELF, not a runtime page size.
    println!("cargo:rustc-link-arg=-z");
    println!("cargo:rustc-link-arg=max-page-size=0x4000");
}
