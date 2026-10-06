//! Converts RV32IMA RISC-V ELF programs into sandboxed, gas-metered native aarch64 or x86_64
//! ELF files, and loads and runs them (see [`execute`]).
//!
//! Start with [`TranspilerForArchitecture::transpile_to_native_elf`]. The crate README
//! describes the design: its "Pipeline", "Output layout", "Sandboxing", "Gas metering" and
//! "Calling into transpiled code" sections explain how the output works, and "Library API"
//! shows a short example.
use log::debug;
use riscv32::gas_cost::{DEFAULT_GAS_COST_SCHEME, RvGasCostScheme};
use riscv32_elf::reader::read_riscv_elf;
use riscv32_elf::types::EntryPointMapping;
use std::collections::HashSet;
use std::str::FromStr;
use strum::Display;
use thiserror::Error;

use crate::arch64::Arm64Transpiler;
pub use crate::elf_builder::ElfBuilderError;
pub use crate::error::TranspilerError;
pub use crate::layout::{EF_TRUSTED_TRANSPILED, LayoutValidationError};
use crate::traits::TranspilerImplementation;
use crate::transpile_rv_prog::transpile_rv_prog_to_elf;
use crate::x86_64::X86Transpiler;

pub(crate) mod arch64;
pub(crate) mod elf_builder;
pub(crate) mod error;
pub mod execute;
pub(crate) mod layout;
#[cfg(test)]
mod test_support;
pub(crate) mod traits;
pub(crate) mod transpile_rv32ima_inst;
pub(crate) mod transpile_rv_prog;
pub(crate) mod transpiled_address;
pub(crate) mod x86_64;

/// Input to a transpilation pass.
///
/// Bundles the raw RISC-V ELF binary with the set of public entry-point names
/// that must be reachable in the output ELF file.
pub struct TranspilerInput<'a> {
    /// Raw bytes of the RISC-V 32-bit ELF to transpile.
    pub riscv_elf_bytes: &'a [u8],
    /// Names of the functions the host can call. Each must be a global function symbol in the
    /// input ELF.
    pub entry_points: HashSet<String>,
}

/// Output from a successful transpilation pass.
///
/// Contains the native ELF bytes and the index of each entry point, which the loader passes
/// in a7 to call it.
pub struct TranspilerOutput {
    /// Bytes of the native ELF file.
    pub elf_bytes: Vec<u8>,
    /// Maps each entry-point name to its call index. Indices start at 1: init functions come
    /// first, in `.init_array` order, then the entry points in byte order of their names.
    pub entry_point_mapping: EntryPointMapping,
}

/// Selects between the gas-metered, sandboxed (untrusted) and the bare-bones (trusted) output.
///
/// `Trusted` output omits gas metering and all bounds checks except the `JALR` target
/// alignment check, and its jump-table entries hold only a 4-byte offset. It is meant only for
/// trusted code run in-process. Its ELF header carries
/// [`EF_TRUSTED_TRANSPILED`], so the loader can refuse it
/// in an untrusted context. The flag is a label, not a security boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranspilationMode {
    /// No gas metering and no bounds checks (the `JALR` target alignment check stays);
    /// 4-byte jump-table entries. Only for trusted code run in-process.
    Trusted,
    /// The default: gas-metered, fully bounds-checked output with 8-byte jump-table entries
    /// (gas cost and offset), using the given gas cost scheme. For untrusted code.
    Untrusted(RvGasCostScheme),
}
impl Default for TranspilationMode {
    fn default() -> Self {
        TranspilationMode::Untrusted(DEFAULT_GAS_COST_SCHEME)
    }
}
impl TranspilationMode {
    /// Returns `true` for [`TranspilationMode::Trusted`].
    pub(crate) fn is_trusted(&self) -> bool {
        matches!(self, TranspilationMode::Trusted)
    }
}

/// Transpiles a RISC-V ELF to a native ELF for the target `TI`.
///
/// Reads and validates the input ELF, transpiles it, and writes the native ELF directly, with
/// no external assembler or linker.
///
/// The returned entry-point mapping gives each entry point's call index. Indices start at 1:
/// init functions come first, in `.init_array` order, then the entry points in byte order of
/// their names, so the mapping is the same on every host.
pub(crate) fn transpile_to_elf<TI: TranspilerImplementation>(
    input: TranspilerInput<'_>,
    mode: &TranspilationMode,
) -> Result<TranspilerOutput, TranspilerError> {
    // With a zero cost, gas never runs out: the output would be unmetered.
    if let TranspilationMode::Untrusted(RvGasCostScheme::Fixed(cost)) = mode
        && cost.value() == 0
    {
        return Err(TranspilerError::ZeroGasCost);
    }
    let riscv_elf = read_riscv_elf(input.riscv_elf_bytes, &input.entry_points)?;
    let elf_bytes = transpile_rv_prog_to_elf::<TI>(&riscv_elf, mode)?;
    Ok(TranspilerOutput {
        elf_bytes,
        entry_point_mapping: riscv_elf.entry_points().name_to_index().clone(),
    })
}

/// Target architecture of the transpiler output.
///
/// Parses from `arm64` (or `aarch64`) and `x86_64`.
#[derive(Debug, Display, Clone, Copy)]
pub enum TranspilerForArchitecture {
    Arm64,
    X86_64,
}

/// A string that names no supported architecture.
#[derive(Debug, Error)]
#[error("unknown architecture {0:?}: expected arm64, aarch64 or x86_64")]
pub struct UnknownArchitecture(String);

impl FromStr for TranspilerForArchitecture {
    type Err = UnknownArchitecture;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "arm64" | "aarch64" => Ok(Self::Arm64),
            "x86_64" => Ok(Self::X86_64),
            _ => Err(UnknownArchitecture(name.to_string())),
        }
    }
}

impl TranspilerForArchitecture {
    /// Returns the architecture of the host the transpiler runs on, or `None` if it is
    /// neither aarch64 nor x86_64.
    ///
    /// Only `target_arch` matters: the OS does not change the output. Segments are aligned to
    /// 16 KiB, so the same ELF loads on hosts with 4 KiB and 16 KiB pages.
    pub fn get_native() -> Option<Self> {
        #[cfg(target_arch = "aarch64")]
        {
            debug!("Native architecture detected: ARM64");
            Some(TranspilerForArchitecture::Arm64)
        }
        #[cfg(target_arch = "x86_64")]
        {
            debug!("Native architecture detected: x86_64");
            Some(TranspilerForArchitecture::X86_64)
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        {
            debug!("Unsupported architecture detected - no native transpiler available");
            None
        }
    }

    /// Transpiles a RISC-V ELF to a native ELF for this architecture.
    ///
    /// `mode` selects the default gas-metered, sandboxed output ([`TranspilationMode::Untrusted`])
    /// or the bare-bones output for trusted code ([`TranspilationMode::Trusted`]).
    ///
    /// [`TranspilerOutput::entry_point_mapping`] gives each entry point's call index: init
    /// functions are indexed first, from 1, then the entry points in byte order of their names.
    pub fn transpile_to_native_elf(
        &self,
        input: TranspilerInput<'_>,
        mode: &TranspilationMode,
    ) -> Result<TranspilerOutput, TranspilerError> {
        match self {
            TranspilerForArchitecture::Arm64 => transpile_to_elf::<Arm64Transpiler>(input, mode),
            TranspilerForArchitecture::X86_64 => transpile_to_elf::<X86Transpiler>(input, mode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TranspilerForArchitecture;

    #[test]
    fn architecture_names_parse() {
        for (name, expected) in [
            ("arm64", "Arm64"),
            ("aarch64", "Arm64"),
            ("x86_64", "X86_64"),
        ] {
            let parsed: TranspilerForArchitecture = name.parse().expect("a known name");
            assert_eq!(parsed.to_string(), expected, "{name}");
        }
        assert!("mips".parse::<TranspilerForArchitecture>().is_err());
        assert!("ARM64".parse::<TranspilerForArchitecture>().is_err());
    }
}
