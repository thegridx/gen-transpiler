//! The loader rejects artifacts it cannot run safely: ones built for another architecture or
//! carrying unknown ELF flags, and ones whose PT_LOAD segments overlap, leave code outside an
//! executable segment, or leave part of the guest-accessible range unmapped, where a
//! bounds-checked guest access would fault the host.

mod common;

use common::{load, native, transpile};
use googletest::prelude::{assert_that, contains_substring, eq, matches_pattern, none, some};
use transpiler::execute::{LoaderContext, LoaderError};
use transpiler::{TranspilationMode, TranspilerForArchitecture};

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_R: u32 = 4;

fn read_u16(elf: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes(
        elf[at..at + 2].try_into().expect("2 bytes"),
    ))
}

fn read_u32(elf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(elf[at..at + 4].try_into().expect("4 bytes"))
}

fn read_u64(elf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(elf[at..at + 8].try_into().expect("8 bytes"))
}

/// Returns the offset of the first PT_LOAD program header whose `p_flags` are `flags`.
fn load_segment_header(elf: &[u8], flags: u32) -> usize {
    let e_phoff = usize::try_from(read_u64(elf, 32)).expect("e_phoff fits usize");
    let e_phentsize = read_u16(elf, 54);
    let e_phnum = read_u16(elf, 56);
    (0..e_phnum)
        .map(|i| e_phoff + i * e_phentsize)
        .find(|&phdr| read_u32(elf, phdr) == PT_LOAD && read_u32(elf, phdr + 4) == flags)
        .unwrap_or_else(|| panic!("no PT_LOAD segment with flags {flags:#x}"))
}

/// Shrinks the read-only segment's `p_memsz` to its `p_filesz`, as the ELF builder used to emit
/// it. That leaves the tail between the end of `.rodata` and the stack segment unmapped.
fn shrink_rodata_segment_to_file_size(elf: &mut [u8]) {
    let e_phoff = usize::try_from(read_u64(elf, 32)).expect("e_phoff fits usize");
    let e_phentsize = read_u16(elf, 54);
    let e_phnum = read_u16(elf, 56);
    for i in 0..e_phnum {
        let phdr = e_phoff + i * e_phentsize;
        if read_u32(elf, phdr) == PT_LOAD && read_u32(elf, phdr + 4) == PF_R {
            let p_filesz = read_u64(elf, phdr + 32);
            assert!(
                p_filesz < read_u64(elf, phdr + 40),
                "the fixture's rodata segment must end short of the next segment"
            );
            elf[phdr + 40..phdr + 48].copy_from_slice(&p_filesz.to_le_bytes());
            return;
        }
    }
    panic!("no read-only PT_LOAD segment");
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn transpiled_output_maps_the_whole_guest_range() {
    let transpiler = native();
    for mode in [TranspilationMode::default(), TranspilationMode::Trusted] {
        let elf = transpile(transpiler, &mode).elf_bytes;
        assert_that!(load(&elf, LoaderContext::Trusted).err(), none());
    }
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn unmapped_gap_in_guest_readable_range_is_rejected() {
    let transpiler = native();
    let mut elf = transpile(transpiler, &TranspilationMode::default()).elf_bytes;
    shrink_rodata_segment_to_file_size(&mut elf);
    assert_that!(
        load(&elf, LoaderContext::Untrusted).err(),
        some(matches_pattern!(LoaderError::RangeNotMapped {
            access: eq("readable"),
            ..
        }))
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
fn artifact_for_another_architecture_is_rejected() {
    let native = native();
    let other = match native {
        TranspilerForArchitecture::Arm64 => TranspilerForArchitecture::X86_64,
        TranspilerForArchitecture::X86_64 => TranspilerForArchitecture::Arm64,
    };
    let elf = transpile(other, &TranspilationMode::default()).elf_bytes;
    assert_that!(
        load(&elf, LoaderContext::Trusted).err(),
        some(matches_pattern!(LoaderError::CompiledWithWrongArchitecture))
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
fn unknown_elf_flags_are_rejected() {
    // ELF64 `e_flags` is the 32-bit field at offset 0x30.
    const E_FLAGS_OFFSET: usize = 0x30;
    let transpiler = native();
    let mut elf = transpile(transpiler, &TranspilationMode::default()).elf_bytes;
    elf[E_FLAGS_OFFSET..E_FLAGS_OFFSET + 4].copy_from_slice(&0x1234_u32.to_le_bytes());
    assert_that!(
        load(&elf, LoaderContext::Trusted).err(),
        some(matches_pattern!(LoaderError::UnknownElfFlags {
            e_flags: eq(&0x1234)
        }))
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
fn overlapping_segments_are_rejected() {
    let transpiler = native();
    let mut elf = transpile(transpiler, &TranspilationMode::default()).elf_bytes;
    // Move the read-only segment onto the code segment's address.
    let code_vaddr = read_u64(&elf, load_segment_header(&elf, PF_R | PF_X) + 16);
    let rodata = load_segment_header(&elf, PF_R);
    elf[rodata + 16..rodata + 24].copy_from_slice(&code_vaddr.to_le_bytes());
    assert_that!(
        load(&elf, LoaderContext::Trusted).err(),
        some(matches_pattern!(LoaderError::OverlappingSegments { .. }))
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
fn code_outside_an_executable_segment_is_rejected() {
    let transpiler = native();
    let mut elf = transpile(transpiler, &TranspilationMode::default()).elf_bytes;
    let code = load_segment_header(&elf, PF_R | PF_X);
    elf[code + 4..code + 8].copy_from_slice(&PF_R.to_le_bytes());
    assert_that!(
        load(&elf, LoaderContext::Trusted).err(),
        some(matches_pattern!(LoaderError::RangeNotMapped {
            access: eq("executable"),
            ..
        }))
    );
}

/// On a host that cannot run transpiled code, such as x86_64 macOS, loading fails up front
/// instead of every run failing later.
#[cfg_attr(
    any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    ),
    ignore = "this host can run transpiled code"
)]
#[test]
fn host_that_cannot_run_transpiled_code_rejects_it_at_load() {
    let architecture =
        TranspilerForArchitecture::get_native().unwrap_or(TranspilerForArchitecture::X86_64);
    let output = transpile(architecture, &TranspilationMode::default());
    assert_that!(
        load(&output.elf_bytes, LoaderContext::Untrusted).err(),
        some(matches_pattern!(LoaderError::UnsupportedHost))
    );
}

/// The code jumps through the jump table, so the table must not be writable.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn writable_jump_table_is_rejected() {
    const PF_W: u32 = 2;
    let mut elf = transpile(native(), &TranspilationMode::default()).elf_bytes;
    // The jump table is in the read-only segment.
    let rodata = load_segment_header(&elf, PF_R);
    elf[rodata + 4..rodata + 8].copy_from_slice(&(PF_R | PF_W).to_le_bytes());
    assert_that!(
        load(&elf, LoaderContext::Trusted).err(),
        some(matches_pattern!(LoaderError::WritableJumpTable { .. }))
    );
}

/// The code entry must be an instruction inside `.text`, not the address just past it.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn code_entry_at_the_end_of_text_is_rejected() {
    use object::{Object as _, ObjectSection as _};
    let mut elf = transpile(native(), &TranspilationMode::default()).elf_bytes;
    let (symbols_offset, text_end) = {
        let file = object::File::parse(&*elf).expect("the output should parse");
        let text = file.section_by_name(".text").expect(".text");
        let symbols = file
            .section_by_name(".transpiled.symbols")
            .expect(".transpiled.symbols");
        let (offset, _) = symbols.file_range().expect("has file bytes");
        (
            usize::try_from(offset).expect("offset fits usize"),
            u32::try_from(text.address() + text.size()).expect("address fits u32"),
        )
    };
    // `start_transpiled_code` is the section's first field.
    elf[symbols_offset..symbols_offset + 4].copy_from_slice(&text_end.to_le_bytes());
    assert_that!(
        load(&elf, LoaderContext::Trusted).err(),
        some(matches_pattern!(LoaderError::ElfParsingError {
            message: contains_substring("start_transpiled_code vaddr")
        }))
    );
}
