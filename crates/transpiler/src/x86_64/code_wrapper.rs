//! Calls into loaded x86_64 transpiled code from the host, passing guest a0 to a7 in rcx, rdx,
//! r8, r9, r10, rax, rsi and rdi.
use user_code_runner::runner::RegistersInOut;

use crate::{
    execute::{LoaderError, SymbolHandle},
    traits::TranspiledCodeWrapper,
};

/// Calls the envelope's entry point with an inline-assembly `call`. On a host other than x86_64
/// Linux it returns `LoaderError::CompiledWithWrongArchitecture`.
pub(crate) struct X86CodeWrapper;

impl TranspiledCodeWrapper for X86CodeWrapper {
    fn transpiled_code_wrapper(
        start_transpiled_code_symbol: SymbolHandle,
        in_out: &mut RegistersInOut,
    ) -> Result<(), LoaderError> {
        #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
        {
            let _ = (start_transpiled_code_symbol.handle, in_out); // Prevent unused variable warnings
            Err(LoaderError::CompiledWithWrongArchitecture)
        }
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        {
            use std::arch::asm;

            // Guest a0 to a7, in the host registers the envelope reads them from. The order must
            // match `IN_OUT_ARGS_MAPPING` in `envelope.rs`.
            let [
                mut rcx,
                mut rdx,
                mut r8,
                mut r9,
                mut r10,
                mut rax,
                mut rsi,
                mut rdi,
            ] = *in_out.registers();
            // SAFETY:
            // - `start_transpiled_code_symbol.handle` is the entry point of loaded, executable
            //   x86_64 transpiled code and is valid to call.
            // - The envelope takes guest a0-a7 in rcx, rdx, r8, r9, r10, rax, rsi and rdi and
            //   returns them in the same registers. It pushes the callee-saved rbx, rbp and
            //   r12-r15 on entry and pops them before `ret`, and its other pushes and pops are
            //   balanced, so rsp is restored. `clobber_abi("system")` covers the caller-saved
            //   registers it overwrites, and the block omits `nostack`, so it may use the stack.
            // - `in_out` owns the guest register snapshot for the duration of this call, so
            //   reading it into these registers and writing the post-call values back is valid.
            // - This block is compiled only on `x86_64` Linux, so `call` and
            //   `clobber_abi("system")` match the executing architecture.
            #[expect(unsafe_code, reason = "See SAFETY comment")]
            unsafe {
                asm!(
                    "call {0}",
                    in(reg) start_transpiled_code_symbol.handle,
                    inout("rcx") rcx,
                    inout("rdx") rdx,
                    inout("r8") r8,
                    inout("r9") r9,
                    inout("r10") r10,
                    inout("rax") rax,
                    inout("rsi") rsi,
                    inout("rdi") rdi,
                    clobber_abi("system")
                )
            };
            in_out.set_registers([rcx, rdx, r8, r9, r10, rax, rsi, rdi]);
            Ok(())
        }
    }
}
