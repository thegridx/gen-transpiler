//! The trusted-mode ELF flag lets a host reject a trusted artifact before running any of it.

mod common;

use common::{load, native, transpile};
use googletest::prelude::{assert_that, eq, matches_pattern, none, some};
use transpiler::EF_TRUSTED_TRANSPILED;
use transpiler::TranspilationMode;
use transpiler::execute::{LoaderContext, LoaderError};

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn only_trusted_output_carries_the_flag() {
    let transpiler = native();
    let e_flags = |elf: &[u8]| {
        let file = object::File::parse(elf).expect("output should be an ELF");
        match object::Object::flags(&file) {
            object::FileFlags::Elf { e_flags, .. } => e_flags,
            other => panic!("unexpected flags {other:?}"),
        }
    };
    assert_that!(
        e_flags(&transpile(transpiler, &TranspilationMode::Trusted).elf_bytes),
        eq(EF_TRUSTED_TRANSPILED)
    );
    assert_that!(
        e_flags(&transpile(transpiler, &TranspilationMode::default()).elf_bytes),
        eq(0)
    );
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn untrusted_context_rejects_trusted_output() {
    let transpiler = native();
    let elf = transpile(transpiler, &TranspilationMode::Trusted).elf_bytes;
    assert_that!(
        load(&elf, LoaderContext::Untrusted).err(),
        some(matches_pattern!(
            LoaderError::TrustedArtifactInUntrustedContext
        ))
    );
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn trusted_context_accepts_trusted_output() {
    let transpiler = native();
    let elf = transpile(transpiler, &TranspilationMode::Trusted).elf_bytes;
    assert_that!(load(&elf, LoaderContext::Trusted).err(), none());
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn untrusted_output_loads_in_either_context() {
    let transpiler = native();
    let elf = transpile(transpiler, &TranspilationMode::default()).elf_bytes;
    assert_that!(load(&elf, LoaderContext::Untrusted).err(), none());
    assert_that!(load(&elf, LoaderContext::Trusted).err(), none());
}
