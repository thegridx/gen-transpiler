//! [`TranspiledVm`] is a loaded transpiled program and the memory it owns.
use riscv32_elf::data_layout::{NO_ECALL_RETURN_ADDRESS, StoredRegister};
use riscv32_elf::types::EntryPointMapping;
use std::collections::BTreeSet;

use crate::TranspilerForArchitecture;
use crate::execute::code_runner::write_register_i32;
use crate::execute::loader::{LoaderContext, MmapHandle, load_library};
use crate::execute::{LoadedLayout, LoaderError};

/// A transpiled library loaded into memory and ready to run.
///
/// It owns the memory the library is loaded into, so the code and data stay mapped for as long
/// as the VM is alive and are unmapped when it is dropped. Run it through a
/// [`TranspiledCodeRunner`](crate::execute::TranspiledCodeRunner), which borrows the VM
/// exclusively.
///
/// A VM can move to another thread and run there. On aarch64 the cache maintenance done at load
/// time reaches every core, and switching threads resynchronizes the instruction stream.
#[derive(Debug)]
pub struct TranspiledVm {
    /// Addresses and memory bounds computed at load time.
    pub(crate) loaded_layout: LoadedLayout,
    /// The host architecture, which the loaded code was built for.
    pub(crate) architecture: TranspilerForArchitecture,
    /// Keeps the library mapped; `loaded_layout` points into it.
    _mapping: MmapHandle,
}

/// The architecture of this host, if transpiled code can run on it: aarch64, or x86_64 on
/// Linux. The x86_64 entry wrapper exists only for Linux.
fn runnable_host_architecture() -> Option<TranspilerForArchitecture> {
    if cfg!(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )) {
        TranspilerForArchitecture::get_native()
    } else {
        None
    }
}

/// Checks that every index in `entry_point_mapping` is in the library's entry-point range.
fn validate_entry_point_mapping(
    loaded_layout: &LoadedLayout,
    entry_point_mapping: &EntryPointMapping,
) -> Result<(), LoaderError> {
    let range = loaded_layout.entry_points_range();
    if entry_point_mapping
        .values()
        .all(|&i| range.contains(&(i as usize)))
    {
        return Ok(());
    }
    let indices = entry_point_mapping
        .values()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Err(LoaderError::EntryPointsDoNotComplyWithRange {
        indices,
        first: *range.start(),
        last: *range.end(),
    })
}

impl TranspiledVm {
    /// Loads a transpiled library from the bytes of the native ELF the transpiler produced.
    ///
    /// The bytes are copied into fresh memory, so later changes to their source cannot change
    /// the loaded code. `loader_ctx` must be [`LoaderContext::Trusted`] to load an artifact
    /// built with `TranspilationMode::Trusted`. `entry_point_mapping` is the mapping the
    /// transpiler returned with the ELF; it is checked against the library's entry points.
    ///
    /// Loading only checks that the ELF is internally consistent. It cannot tell whether the
    /// native code in it is sandboxed: load only artifacts this host transpiled itself or
    /// whose integrity it has verified.
    ///
    /// # Errors
    /// Returns an error if the ELF is malformed or unsupported on this host, if it is trusted
    /// and `loader_ctx` is untrusted, or if the entry point mapping doesn't match the library.
    pub fn load(
        elf_bytes: &[u8],
        loader_ctx: LoaderContext,
        entry_point_mapping: &EntryPointMapping,
    ) -> Result<Self, LoaderError> {
        let architecture = runnable_host_architecture().ok_or(LoaderError::UnsupportedHost)?;
        let (mapping, loaded_layout) = load_library(elf_bytes, loader_ctx)?;
        validate_entry_point_mapping(&loaded_layout, entry_point_mapping)?;
        // Nothing has stopped at an ecall yet, so a resume must not continue anywhere.
        write_register_i32(
            &loaded_layout,
            StoredRegister::ReturnAddress,
            NO_ECALL_RETURN_ADDRESS as i32,
        );
        Ok(TranspiledVm {
            loaded_layout,
            architecture,
            _mapping: mapping,
        })
    }
}
