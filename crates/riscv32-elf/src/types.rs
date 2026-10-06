//! Common types used by the ELF reader and the transpiler.

use riscv32::{
    N_BYTES_IN_WORD,
    common::{ValueError, checked_add_u32},
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RiscvAddress(u32);
impl RiscvAddress {
    pub fn new(addr: u32) -> Self {
        Self(addr)
    }
    pub fn index(&self) -> u32 {
        self.0 / N_BYTES_IN_WORD as u32
    }
    pub fn as_u32(&self) -> u32 {
        self.0
    }
    pub fn as_i32(&self) -> i32 {
        self.0 as i32
    }
    pub fn wrapping_add(&self, offset: u32) -> RiscvAddress {
        RiscvAddress(self.0.wrapping_add(offset))
    }
    pub fn checked_add(&self, v: u32, context: &str) -> Result<Self, ValueError> {
        checked_add_u32(self.0, v, context).map(RiscvAddress)
    }
}

/// Data address relative to the beginning of rodata.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct VirtualDataPointer(u32);
impl VirtualDataPointer {
    pub fn new(addr: u32) -> Self {
        Self(addr)
    }
    pub fn as_i32(&self) -> i32 {
        self.0 as i32
    }
    pub fn as_u32(&self) -> u32 {
        self.0
    }
    pub fn as_usize(&self) -> usize {
        self.0 as usize
    }
    pub fn checked_add(
        &self,
        addend: VirtualDataSize,
        context: &str,
    ) -> Result<VirtualDataPointer, ValueError> {
        checked_add_u32(self.0, addend.as_u32(), context).map(VirtualDataPointer)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VirtualDataRange {
    start: VirtualDataPointer,
    end: VirtualDataPointer,
}
impl VirtualDataRange {
    pub fn new(start: VirtualDataPointer, end: VirtualDataPointer) -> Self {
        Self { start, end }
    }
    pub fn start(&self) -> VirtualDataPointer {
        self.start
    }
    pub fn end(&self) -> VirtualDataPointer {
        self.end
    }
    pub fn size(&self) -> VirtualDataSize {
        VirtualDataSize::new(self.end.as_u32().saturating_sub(self.start.as_u32()))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VirtualDataSize(u32);
impl VirtualDataSize {
    pub fn new(size: u32) -> Self {
        Self(size)
    }
    pub fn as_u32(&self) -> u32 {
        self.0
    }
    pub fn as_usize(&self) -> usize {
        self.0 as usize
    }
}

pub type EntryPointMapping = HashMap<String, u32>;

/// The four key absolute symbol values found in a RISC-V ELF.
///
/// The linker script guarantees, and `read_riscv_elf` checks, that
/// `data_end >= stack_top >= data_start`. `data_end` is the end of the
/// R+W segment (`.ecall_data` is the last R+W section).
#[derive(Debug, Clone)]
pub struct RvSymbolValues {
    pub text_end: RiscvAddress,
    pub data_start: RiscvAddress,
    pub stack_top: RiscvAddress,
    pub data_end: RiscvAddress,
}
