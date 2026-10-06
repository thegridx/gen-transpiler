//! [`TranspiledCodeRunner`] runs a loaded program and gives the host access to its saved
//! registers, gas and guest memory between runs.
use log::{debug, trace};
use riscv32::common::checked_usize_to_u32;
use riscv32::registers::RvPreservedRegister;
use riscv32_elf::{data_layout::StoredRegister, types::RiscvAddress};
use user_code_runner::{
    gas::UserCodeGasAmount,
    runner::{
        CodeRunner, MemoryAccessor, RegistersInOut, UserCodeRunnerError, VirtualDataPointer,
        VirtualDataRange, VirtualDataSize,
    },
};

use crate::{
    TranspilerForArchitecture,
    arch64::code_wrapper::Arm64CodeWrapper,
    execute::{LoadedLayout, SymbolHandle, transpiled_vm::TranspiledVm},
    traits::TranspiledCodeWrapper,
    x86_64::code_wrapper::X86CodeWrapper,
};

fn stored_register_to_ptr_i32(loaded_layout: &LoadedLayout, sr: StoredRegister) -> *mut i32 {
    let register_store_ptr = loaded_layout.register_store_ptr();
    let offset = sr.get_offset();
    trace!("register_store_ptr: {register_store_ptr:?} offset={offset}");
    (register_store_ptr.wrapping_add(offset as usize)) as *mut i32
}

fn stored_register_to_ptr_i64(loaded_layout: &LoadedLayout, sr: StoredRegister) -> *mut i64 {
    let register_store_ptr = loaded_layout.register_store_ptr();
    let offset = sr.get_offset();
    trace!("register_store_ptr: {register_store_ptr:?} offset={offset}");
    (register_store_ptr.wrapping_add(offset as usize)) as *mut i64
}

// The register-store and guest-memory accessors below dereference raw addresses taken from a
// `LoadedLayout`. They are sound because a `LoadedLayout` comes only from the loader, which
// checked that the register store is mapped read-write and 8-byte aligned and that guest
// memory is mapped, and because callers reach them only through a `TranspiledCodeRunner`,
// which exclusively borrows the `TranspiledVm` that owns the mapping. Native code does not run
// while the runner is being used to access memory.

/// Reads an i32 value from the register store.
#[expect(unsafe_code, reason = "See SAFETY comment")]
pub(crate) fn read_register_i32(loaded_layout: &LoadedLayout, sr: StoredRegister) -> i32 {
    let location = stored_register_to_ptr_i32(loaded_layout, sr);
    // SAFETY: the register store is mapped and aligned (see the comment above), and `sr` is
    // a 4-byte slot inside it.
    unsafe { std::ptr::read(location) }
}

/// Reads an i64 value from the register store.
#[expect(unsafe_code, reason = "See SAFETY comment")]
fn read_register_i64(loaded_layout: &LoadedLayout, sr: StoredRegister) -> i64 {
    let location = stored_register_to_ptr_i64(loaded_layout, sr);
    // SAFETY: the register store is mapped and 8-byte aligned (see the comment above), and
    // `sr` is an 8-byte-aligned 8-byte slot inside it.
    unsafe { std::ptr::read(location) }
}

/// Writes an i32 value to the register store.
#[expect(unsafe_code, reason = "See SAFETY comment")]
pub(crate) fn write_register_i32(loaded_layout: &LoadedLayout, sr: StoredRegister, value: i32) {
    let location = stored_register_to_ptr_i32(loaded_layout, sr);
    // SAFETY: the register store is mapped writable and aligned (see the comment above), and
    // `sr` is a 4-byte slot inside it.
    unsafe { std::ptr::write(location, value) };
}

/// Writes an i64 value to the register store.
#[expect(unsafe_code, reason = "See SAFETY comment")]
fn write_register_i64(loaded_layout: &LoadedLayout, sr: StoredRegister, value: i64) {
    let location = stored_register_to_ptr_i64(loaded_layout, sr);
    // SAFETY: the register store is mapped writable and 8-byte aligned (see the comment
    // above), and `sr` is an 8-byte-aligned 8-byte slot inside it.
    unsafe { std::ptr::write(location, value) };
}

// Guest loads may read `[0, _data_end_value)`: from the start of `.original_risc_prog` to the
// end of `.ecall_data`.
fn validate_read_memory_pointer(
    loaded_layout: &LoadedLayout,
    ptr: VirtualDataPointer,
    len: VirtualDataSize,
) -> Result<*mut u8, UserCodeRunnerError> {
    let out_of_bounds = UserCodeRunnerError::GuestMemoryOutOfBounds {
        addr: ptr.as_u32(),
        len: len.as_u32(),
    };
    let Ok(read_end_addr) = ptr.checked_add(len, "read_end_addr") else {
        return Err(out_of_bounds);
    };
    if read_end_addr > loaded_layout.max_readable_virtual_address() {
        return Err(out_of_bounds);
    }
    let read_ptr = loaded_layout
        .rodata_start_ptr()
        .wrapping_add(ptr.as_usize());
    Ok(read_ptr)
}

fn validate_write_memory_pointer(
    loaded_layout: &LoadedLayout,
    ptr: VirtualDataPointer,
    len: VirtualDataSize,
) -> Result<*mut u8, UserCodeRunnerError> {
    if ptr < loaded_layout.virtual_data_start {
        return Err(UserCodeRunnerError::GuestMemoryOutOfBounds {
            addr: ptr.as_u32(),
            len: len.as_u32(),
        });
    }
    validate_read_memory_pointer(loaded_layout, ptr, len)
}

/// Copies guest memory starting at `ptr` into `buf`. The range must lie in
/// `[0, _data_end_value)`.
#[expect(unsafe_code, reason = "See SAFETY comment")]
fn mem_bytes_to_slice(
    loaded_layout: &LoadedLayout,
    ptr: VirtualDataPointer,
    buf: &mut [u8],
) -> Result<(), UserCodeRunnerError> {
    let len_in_bytes = checked_usize_to_u32(buf.len(), "read_len").map_err(|_| {
        UserCodeRunnerError::GuestMemoryOutOfBounds {
            addr: ptr.as_u32(),
            len: u32::MAX,
        }
    })?;
    let read_ptr =
        validate_read_memory_pointer(loaded_layout, ptr, VirtualDataSize::new(len_in_bytes))?;
    debug!("reading from: {read_ptr:?} {len_in_bytes:?}");
    // SAFETY: the range was bounds-checked above against guest memory, which the loader
    // checked is mapped readable (see the comment above `read_register_i32`). `buf` is a
    // host buffer, so it cannot overlap guest memory.
    unsafe {
        std::ptr::copy_nonoverlapping(read_ptr, buf.as_mut_ptr(), buf.len());
    }
    Ok(())
}

/// Copies bytes into guest memory. The range must lie in the writable region,
/// `[_data_start_value, _data_end_value)`: from the start of `.stack` to the end of `.ecall_data`.
#[expect(unsafe_code, reason = "See SAFETY comment")]
pub(crate) fn vec_to_mem_bytes(
    loaded_layout: &LoadedLayout,
    bytes: &[u8],
    ptr: VirtualDataPointer,
) -> Result<(), UserCodeRunnerError> {
    let len_in_bytes = checked_usize_to_u32(bytes.len(), "write_len").map_err(|_| {
        UserCodeRunnerError::GuestMemoryOutOfBounds {
            addr: ptr.as_u32(),
            len: u32::MAX,
        }
    })?;
    let write_ptr =
        validate_write_memory_pointer(loaded_layout, ptr, VirtualDataSize::new(len_in_bytes))?;
    debug!("writing to: {write_ptr:?} {len_in_bytes:?}");
    // SAFETY: the range was bounds-checked above against the writable guest region, which the
    // loader checked is mapped writable (see the comment above `read_register_i32`).
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), write_ptr, len_in_bytes as usize);
    }
    Ok(())
}

/// Runs a loaded [`TranspiledVm`].
///
/// Implements [`user_code_runner::runner::CodeRunner`] and
/// [`user_code_runner::runner::MemoryAccessor`], so a host drives it with
/// `user_code_runner::runner::run_init_array` and `call_entry_point`.
///
/// The runner borrows the [`TranspiledVm`] exclusively: native code writes the VM's memory, so
/// only one runner may drive a VM at a time.
pub struct TranspiledCodeRunner<'a> {
    transpiled_vm: &'a mut TranspiledVm,
}

impl<'a> TranspiledCodeRunner<'a> {
    /// Creates a runner for a loaded program. The program was built for this host's
    /// architecture, which [`TranspiledVm::load`] checked.
    pub fn new(transpiled_vm: &'a mut TranspiledVm) -> Self {
        Self { transpiled_vm }
    }

    /// The layout of the VM this runner drives.
    pub fn loaded_layout(&self) -> &LoadedLayout {
        &self.transpiled_vm.loaded_layout
    }

    /// Sets the address a return from an ecall (a7 = 0) resumes at. The transpiled code takes
    /// the panic exit if it is not a 4-aligned address within the guest's code.
    pub fn set_ecall_return_address(&mut self, address: RiscvAddress) {
        write_register_i32(
            &self.transpiled_vm.loaded_layout,
            StoredRegister::ReturnAddress,
            address.as_i32(),
        )
    }
}

impl MemoryAccessor for TranspiledCodeRunner<'_> {
    fn read_register(&self, register: RvPreservedRegister) -> i32 {
        read_register_i32(
            &self.transpiled_vm.loaded_layout,
            StoredRegister::Riscv32Reg(register),
        )
    }

    fn write_register(&mut self, register: RvPreservedRegister, value: i32) {
        write_register_i32(
            &self.transpiled_vm.loaded_layout,
            StoredRegister::Riscv32Reg(register),
            value,
        )
    }

    fn read_memory(
        &self,
        addr: VirtualDataPointer,
        buf: &mut [u8],
    ) -> Result<(), UserCodeRunnerError> {
        mem_bytes_to_slice(&self.transpiled_vm.loaded_layout, addr, buf)
    }

    fn write_memory(
        &mut self,
        addr: VirtualDataPointer,
        data: &[u8],
    ) -> Result<(), UserCodeRunnerError> {
        vec_to_mem_bytes(&self.transpiled_vm.loaded_layout, data, addr)?;
        Ok(())
    }

    fn get_sp(&self) -> VirtualDataPointer {
        VirtualDataPointer::new(read_register_i32(
            &self.transpiled_vm.loaded_layout,
            StoredRegister::Riscv32Reg(RvPreservedRegister::Sp),
        ) as u32)
    }

    fn set_sp(&mut self, sp: VirtualDataPointer) {
        write_register_i32(
            &self.transpiled_vm.loaded_layout,
            StoredRegister::Riscv32Reg(RvPreservedRegister::Sp),
            sp.as_i32(),
        )
    }

    fn get_gas(&self) -> UserCodeGasAmount {
        UserCodeGasAmount::new(read_register_i64(
            &self.transpiled_vm.loaded_layout,
            StoredRegister::Gas,
        ))
    }

    fn set_gas(&mut self, gas: UserCodeGasAmount) {
        write_register_i64(
            &self.transpiled_vm.loaded_layout,
            StoredRegister::Gas,
            gas.clamped_to_min_stored().as_i64(),
        )
    }
}

impl CodeRunner for TranspiledCodeRunner<'_> {
    fn run_till_ecall_or_return(
        &mut self,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        let start_transpiled_code_ptr = SymbolHandle {
            handle: self.transpiled_vm.loaded_layout.start_transpiled_code_ptr(),
        };
        let run_result = match self.transpiled_vm.architecture {
            TranspilerForArchitecture::Arm64 => Arm64CodeWrapper::transpiled_code_wrapper(
                start_transpiled_code_ptr,
                inout_registers,
            ),
            TranspilerForArchitecture::X86_64 => {
                X86CodeWrapper::transpiled_code_wrapper(start_transpiled_code_ptr, inout_registers)
            }
        };
        run_result.map_err(UserCodeRunnerError::from)
    }

    fn n_init_array_functions(&self) -> usize {
        self.transpiled_vm.loaded_layout.n_init_array_functions
    }

    fn n_entry_point_functions(&self) -> usize {
        self.transpiled_vm.loaded_layout.n_entry_point_functions
    }

    fn virtual_stack_range(&self) -> VirtualDataRange {
        VirtualDataRange::new(
            self.transpiled_vm.loaded_layout.virtual_stack_bottom,
            self.transpiled_vm.loaded_layout.virtual_stack_top,
        )
    }
}
