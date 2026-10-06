//! Calls into loaded aarch64 transpiled code from the host, passing guest a0 to a7 in w0 to w7.
use user_code_runner::runner::RegistersInOut;

use crate::{
    execute::{LoaderError, SymbolHandle},
    traits::TranspiledCodeWrapper,
};

/// Calls the envelope's entry point with an inline-assembly `BLR`. On a non-aarch64 host it
/// returns `LoaderError::CompiledWithWrongArchitecture`.
pub(crate) struct Arm64CodeWrapper;

impl TranspiledCodeWrapper for Arm64CodeWrapper {
    fn transpiled_code_wrapper(
        start_transpiled_code_symbol: SymbolHandle,
        in_out: &mut RegistersInOut,
    ) -> Result<(), LoaderError> {
        #[cfg(not(target_arch = "aarch64"))]
        {
            let _ = (start_transpiled_code_symbol, in_out); // Unused on this architecture.
            Err(LoaderError::CompiledWithWrongArchitecture)
        }
        #[cfg(target_arch = "aarch64")]
        {
            use std::arch::asm;

            let regs = in_out.registers();
            let (mut w0, mut w1, mut w2, mut w3, mut w4, mut w5, mut w6, mut w7) = (
                regs[0], regs[1], regs[2], regs[3], regs[4], regs[5], regs[6], regs[7],
            );

            // SAFETY:
            // - `start_transpiled_code_symbol.handle` is the entry point of loaded, executable
            //   aarch64 transpiled code and is valid to branch to.
            // - The envelope follows the AAPCS64 calling convention: it takes guest a0-a7 in
            //   w0-w7, saves and restores the callee-saved x19-x30, leaves sp as it found it,
            //   and returns with `RET`. It never touches x18 (the platform register on Apple
            //   hosts), so Linux and macOS behave the same. `clobber_abi("system")` covers the
            //   caller-saved registers it overwrites.
            // - `in_out` owns the guest register snapshot for the duration of this call, so
            //   reading it into w0-w7 and writing the post-call values back is valid.
            // - This block is compiled only on `aarch64`, so `BLR` and `clobber_abi("system")`
            //   match the executing architecture.
            #[expect(unsafe_code, reason = "See SAFETY comment")]
            unsafe {
                asm!(
                    "blr {0}",
                    in(reg) start_transpiled_code_symbol.handle,
                    inout("w0") w0,
                    inout("w1") w1,
                    inout("w2") w2,
                    inout("w3") w3,
                    inout("w4") w4,
                    inout("w5") w5,
                    inout("w6") w6,
                    inout("w7") w7,
                    clobber_abi("system")
                )
            }
            in_out.set_registers([w0, w1, w2, w3, w4, w5, w6, w7]);
            Ok(())
        }
    }
}
