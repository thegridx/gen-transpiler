//! Addresses and bounds of a loaded program, computed once at load time.
use log::debug;
use riscv32_elf::types::{RiscvAddress, VirtualDataPointer};
use std::{ops::RangeInclusive, os::raw::c_void};

/// A host address in the loaded program, as a plain native-sized integer.
pub(crate) type RawPtr = usize;

/// Addresses and bounds of a loaded program, derived and validated at load time.
///
/// Guest (virtual) addresses are offsets from the start of `.original_risc_prog`.
///
/// Only the loader creates one. The raw addresses it reports are plain numbers for inspection:
/// they are valid only while the [`TranspiledVm`](crate::execute::TranspiledVm) it came from
/// is alive, and no public API dereferences them.
#[derive(Debug, Clone)]
pub struct LoadedLayout {
    pub(crate) n_init_array_functions: usize,
    pub(crate) n_entry_point_functions: usize,
    pub(crate) start_transpiled_code: RawPtr,
    pub(crate) virtual_data_start: VirtualDataPointer,
    pub(crate) virtual_stack_bottom: VirtualDataPointer,
    pub(crate) virtual_stack_top: VirtualDataPointer,
    pub(crate) virtual_data_end: VirtualDataPointer,
    pub(crate) derived_rodata_start: RawPtr,
    pub(crate) derived_register_store_start: RawPtr,
    pub(crate) risc_code_end_addr: RiscvAddress,
}

impl LoadedLayout {
    /// Logs all layout fields at debug level, under `label`.
    pub(crate) fn log_summary(&self, label: &str) {
        debug!(
            "{label}: n_init={} n_entry={} start_code={:#x} rodata={:#x} \
             data_start={:#x} stack_bottom={:#x} stack_top={:#x} \
             data_end={:#x} register_store={:#x}",
            self.n_init_array_functions,
            self.n_entry_point_functions,
            self.start_transpiled_code,
            self.derived_rodata_start,
            self.virtual_data_start.as_u32(),
            self.virtual_stack_bottom.as_u32(),
            self.virtual_stack_top.as_u32(),
            self.virtual_data_end.as_u32(),
            self.derived_register_store_start,
        );
    }

    pub(crate) fn entry_points_range(&self) -> RangeInclusive<usize> {
        // Entry points come after the init functions. Index 0 means "return from ecall", so
        // init functions start at 1.
        self.n_init_array_functions.saturating_add(1)
            ..=self
                .n_init_array_functions
                .saturating_add(self.n_entry_point_functions)
    }
    pub(crate) fn start_transpiled_code_ptr(&self) -> *mut c_void {
        self.start_transpiled_code as *mut c_void
    }
    pub(crate) fn register_store_ptr(&self) -> *mut u8 {
        self.derived_register_store_start as *mut u8
    }
    pub(crate) fn rodata_start_ptr(&self) -> *mut u8 {
        self.derived_rodata_start as *mut u8
    }
    /// Guest address just past the end of `.stack`: the initial stack pointer.
    pub fn stack_top(&self) -> VirtualDataPointer {
        self.virtual_stack_top
    }
    /// Guest address `_data_end_value`, the exclusive end of readable and writable guest memory.
    pub(crate) fn max_readable_virtual_address(&self) -> VirtualDataPointer {
        self.virtual_data_end
    }
    /// Guest address `_text_end_value`, the end of the original RISC-V code.
    pub fn risc_code_end_addr(&self) -> RiscvAddress {
        self.risc_code_end_addr
    }
}
