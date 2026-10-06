//! Layout of the `.register_store` and `.ecall_data` sections shared by the
//! transpiler (which emits code that reads and writes them) and any loader
//! (which marshals registers and gas through them).
//!
//! The writable region of every transpiled program is laid out
//! `.stack → .data → .bss → .heap → .ecall_data → .register_store`, with
//! `.stack` lowest so a stack-pointer underflow faults against the read-only
//! rodata segment rather than corrupting `.data`/`.bss`.

use riscv32::{N_BYTES_IN_WORD, registers::RvPreservedRegister};

/// Fixed `.ecall_data` section size (64 KB): transient scratch used to pass
/// ecall arguments and return values between guest and host.
pub const ECALL_DATA_SIZE_BYTES: u32 = 0x10000;

/// The return address stored when there is no ecall to return from: after loading, and after
/// a normal, panic or illegal-instruction exit. It is misaligned, so resuming from it (a7 = 0)
/// takes the panic exit instead of continuing at an arbitrary instruction.
pub const NO_ECALL_RETURN_ADDRESS: u32 = u32::MAX;

/// Size in bytes of the `.register_store` section (108):
/// - [`RvPreservedRegister::COUNT`] × 4 bytes for preserved RISC-V
///   registers (92 bytes)
/// - 4 bytes for the ecall return address
/// - 8 bytes for the 64-bit gas counter
/// - 4 bytes for the number of the ecall the program stopped at
pub const REGISTER_STORE_SIZE_BYTES: u32 =
    RvPreservedRegister::COUNT * N_BYTES_IN_WORD as u32 + N_BYTES_IN_WORD as u32 + 8 + 4;

/// One addressable slot inside the `.register_store` section. Maps a logical
/// piece of preserved VM state to its byte offset (via [`Self::get_offset`]),
/// so the transpiled code and the loader agree on the blob layout.
#[derive(Debug, Clone)]
pub enum StoredRegister {
    /// One of the preserved RISC-V registers, stored at `register_index * 4`
    /// bytes from the start of the section.
    Riscv32Reg(RvPreservedRegister),
    /// The RISC-V address to resume at after the yielding ecall, stored
    /// immediately after the preserved registers. Holds [`NO_ECALL_RETURN_ADDRESS`] when the
    /// program did not stop at an ecall.
    ReturnAddress,
    /// The 64-bit gas counter, stored immediately after the return address.
    Gas,
    /// The ecall number (a7) the program stopped at, stored after the gas counter. A resume
    /// puts it back in a7: the RISC-V ABI keeps a7 across an ecall, and compilers rely on it,
    /// while the host passes 0 in a7 to request the resume.
    EcallNumber,
}
impl StoredRegister {
    /// Byte offset of this slot from the start of the `.register_store`
    /// section. Offsets are contiguous and non-overlapping; the total is
    /// [`REGISTER_STORE_SIZE_BYTES`].
    pub fn get_offset(&self) -> u32 {
        match self {
            StoredRegister::Riscv32Reg(r) => (*r as u32) * N_BYTES_IN_WORD as u32,
            StoredRegister::ReturnAddress => RvPreservedRegister::COUNT * N_BYTES_IN_WORD as u32,
            StoredRegister::Gas => (RvPreservedRegister::COUNT + 1) * N_BYTES_IN_WORD as u32,
            StoredRegister::EcallNumber => {
                (RvPreservedRegister::COUNT + 1) * N_BYTES_IN_WORD as u32 + 8
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator as _;

    // ── StoredRegister::get_offset ────────────────────────────────────────────

    #[test]
    fn preserved_register_offsets_are_sequential_multiples_of_word_size() {
        for reg in RvPreservedRegister::iter() {
            let expected = (reg as u32) * N_BYTES_IN_WORD as u32;
            let actual = StoredRegister::Riscv32Reg(reg).get_offset();
            assert_eq!(actual, expected, "{reg:?} should be at offset {expected}");
        }
    }

    #[test]
    fn first_preserved_register_is_at_offset_zero() {
        assert_eq!(
            StoredRegister::Riscv32Reg(RvPreservedRegister::Ra).get_offset(),
            0
        );
    }

    #[test]
    fn last_preserved_register_is_at_correct_offset() {
        // T6 is variant 22 (0..=22), so offset = 22 * 4 = 88
        let expected = (RvPreservedRegister::COUNT - 1) * N_BYTES_IN_WORD as u32;
        assert_eq!(
            StoredRegister::Riscv32Reg(RvPreservedRegister::T6).get_offset(),
            expected
        );
        assert_eq!(expected, 88);
    }

    #[test]
    fn return_address_is_immediately_after_preserved_registers() {
        // COUNT preserved registers × 4 bytes each
        let expected = RvPreservedRegister::COUNT * N_BYTES_IN_WORD as u32;
        assert_eq!(StoredRegister::ReturnAddress.get_offset(), expected);
        assert_eq!(expected, 92);
    }

    #[test]
    fn gas_counter_is_after_return_address() {
        // (COUNT + 1) × 4 = 24 × 4 = 96
        let expected = (RvPreservedRegister::COUNT + 1) * N_BYTES_IN_WORD as u32;
        assert_eq!(StoredRegister::Gas.get_offset(), expected);
        assert_eq!(expected, 96);
    }

    #[test]
    fn ecall_number_is_after_gas_counter() {
        // Gas occupies 8 bytes (i64) from offset 96.
        assert_eq!(StoredRegister::EcallNumber.get_offset(), 104);
    }

    #[test]
    fn register_store_layout_consistent_with_size_constant() {
        // The ecall number, the last slot, occupies 4 bytes from offset 104: 108 in all.
        let last_offset = StoredRegister::EcallNumber.get_offset();
        assert_eq!(last_offset + 4, REGISTER_STORE_SIZE_BYTES);
        assert_eq!(REGISTER_STORE_SIZE_BYTES, 108);
    }

    #[test]
    fn preserved_register_count_matches_expected_layout() {
        // The comment says 23 registers; verify COUNT == 23
        assert_eq!(RvPreservedRegister::COUNT, 23);
    }
}
