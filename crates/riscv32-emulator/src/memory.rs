//! Guest memory: one flat image of `[0, _data_end_value)`, with the transpiler's access rules.
//!
//! Loads may read `[0, _data_end_value)` and stores may write
//! `[_data_start_value, _data_end_value)`, the same ranges the transpiled bounds checks allow.
use riscv32_elf::reader::RiscvElf;

use crate::error::EmulatorError;

/// The bounds of the guest's code and memory.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GuestMemoryRanges {
    /// `_text_end_value`: the end of the code, and the address an entry point returns to.
    pub(crate) code_end: u32,
    /// `_data_start_value`: the first writable address.
    pub(crate) data_start: u32,
    /// `_data_end_value`: the end of readable and writable memory.
    pub(crate) data_end: u32,
}

impl GuestMemoryRanges {
    /// The last address a load of `size` bytes may start at, if any fits.
    pub(crate) fn last_readable(&self, size: u32) -> Option<u32> {
        self.data_end.checked_sub(size)
    }

    /// The last address a store of `size` bytes may start at, if any fits.
    pub(crate) fn last_writable(&self, size: u32) -> Option<u32> {
        self.data_end
            .checked_sub(size)
            .filter(|&last| last >= self.data_start)
    }

    /// Whether a guest load of `size` bytes at `address` is allowed.
    pub(crate) fn can_load(&self, address: u32, size: u32) -> bool {
        self.last_readable(size).is_some_and(|last| address <= last)
    }

    /// Whether a guest store of `size` bytes at `address` is allowed.
    pub(crate) fn can_store(&self, address: u32, size: u32) -> bool {
        address >= self.data_start && self.last_writable(size).is_some_and(|last| address <= last)
    }
}

/// The guest's memory image.
#[derive(Debug)]
pub(crate) struct GuestMemory {
    bytes: Vec<u8>,
}

impl GuestMemory {
    /// Lays out the program's code and data sections in a zeroed image of `data_end` bytes,
    /// as the loader does for transpiled code. `.init_array` is not included.
    pub(crate) fn new(riscv_elf: &RiscvElf, data_end: u32) -> Result<Self, EmulatorError> {
        let mut bytes = vec![0; data_end as usize];
        let sections = std::iter::once(riscv_elf.code_section()).chain(riscv_elf.data_sections());
        for section in sections {
            let data = section.data();
            let address = section.address().as_u32();
            let destination = (address as usize)
                .checked_add(data.len())
                .and_then(|end| bytes.get_mut(address as usize..end))
                .ok_or_else(|| EmulatorError::SectionOutsideMemory {
                    section: section.name().to_string(),
                    address,
                    size: data.len(),
                })?;
            destination.copy_from_slice(data);
        }
        Ok(Self { bytes })
    }

    /// The bytes at `[address, address + len)`, if they are all in the image.
    pub(crate) fn get(&self, address: u32, len: usize) -> Option<&[u8]> {
        let start = address as usize;
        self.bytes.get(start..start.checked_add(len)?)
    }

    /// The bytes at `[address, address + len)` for writing, if they are all in the image.
    pub(crate) fn get_mut(&mut self, address: u32, len: usize) -> Option<&mut [u8]> {
        let start = address as usize;
        self.bytes.get_mut(start..start.checked_add(len)?)
    }

    /// Reads a little-endian value of `size` bytes (1, 2 or 4), zero-extended.
    pub(crate) fn read(&self, address: u32, size: u32) -> Option<u32> {
        let bytes = self.get(address, size as usize)?;
        Some(
            bytes
                .iter()
                .rev()
                .fold(0_u32, |value, &byte| (value << 8) | u32::from(byte)),
        )
    }

    /// Writes the low `size` bytes (1, 2 or 4) of `value`, little-endian.
    pub(crate) fn write(&mut self, address: u32, size: u32, value: u32) -> Option<()> {
        let bytes = self.get_mut(address, size as usize)?;
        let le = value.to_le_bytes();
        bytes.copy_from_slice(le.get(..size as usize)?);
        Some(())
    }
}
