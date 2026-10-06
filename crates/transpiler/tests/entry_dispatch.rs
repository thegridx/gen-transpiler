//! The entry stub validates the host-supplied state it dispatches on: the function selector in
//! a7, and the saved return address used to resume after an ecall. An out-of-range value takes
//! the panic exit instead of branching to an arbitrary address.

mod common;

use common::{load, native, transpile};
use googletest::prelude::{assert_that, eq, ok};
use riscv32::registers::RvPreservedRegister;
use riscv32_elf::types::RiscvAddress;
use transpiler::TranspilationMode;
use transpiler::execute::{LoaderContext, TranspiledCodeRunner, TranspiledVm};
use user_code_runner::gas::UserCodeGasAmount;
use user_code_runner::runner::{
    CodeRunner as _, EcallHandler, ILLEGAL_INSTRUCTION_EXIT_VALUE, MemoryAccessor as _,
    NORMAL_EXIT_VALUE, PANIC_EXIT_VALUE, RegistersInOut, RunOutcome, UserCodeRunnerError,
    call_entry_point,
};

const MODES: [TranspilationMode; 2] = [
    TranspilationMode::Trusted,
    TranspilationMode::Untrusted(riscv32::gas_cost::DEFAULT_GAS_COST_SCHEME),
];

/// Loads the math-test fixture.
fn load_vm(mode: &TranspilationMode) -> TranspiledVm {
    let output = transpile(native(), mode);
    load(&output.elf_bytes, LoaderContext::Trusted).expect("math-test should load")
}

fn runner(vm: &mut TranspiledVm) -> TranspiledCodeRunner<'_> {
    let mut runner = TranspiledCodeRunner::new(vm);
    let stack_top = runner.loaded_layout().stack_top();
    runner.set_sp(stack_top);
    runner.set_gas(UserCodeGasAmount::max());
    runner
}

/// Registers that select `a7`, with `a0 = 10` as the argument.
fn registers(a7: i32) -> RegistersInOut {
    RegistersInOut::new([10, 0, 0, 0, 0, 0, 0, a7])
}

/// Runs one burst and returns the exit value left in a7.
fn run_once(runner: &mut TranspiledCodeRunner<'_>, regs: &mut RegistersInOut) -> i32 {
    runner
        .run_till_ecall_or_return(regs)
        .expect("the burst should complete");
    regs.get_user_code_exit_value()
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn valid_selector_runs_the_entry_point() {
    for mode in MODES {
        let mut vm = load_vm(&mode);
        let mut runner = runner(&mut vm);
        let selector = *runner.entry_point_function_index_range().start();
        let mut regs = registers(i32::try_from(selector).expect("small index"));
        assert_that!(run_once(&mut runner, &mut regs), eq(NORMAL_EXIT_VALUE));
        // sum_1_to_x(10)
        assert_that!(regs.get_normal_return_value(), eq(55));
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
fn out_of_range_selector_takes_the_panic_exit() {
    for mode in MODES {
        let mut vm = load_vm(&mode);
        let mut runner = runner(&mut vm);
        let one_past_last = i32::try_from(*runner.entry_point_function_index_range().end())
            .expect("small index")
            + 1;
        for selector in [one_past_last, 9999, PANIC_EXIT_VALUE, -2, i32::MIN] {
            let mut regs = registers(selector);
            assert_that!(
                run_once(&mut runner, &mut regs),
                eq(PANIC_EXIT_VALUE),
                "selector {selector} in {mode:?}"
            );
        }
    }
}

/// Resuming at the end of the code, as after an `ecall` that is the last instruction, is like
/// falling off the end: an illegal-instruction exit.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn ecall_return_to_the_end_of_the_code_is_illegal() {
    for mode in MODES {
        let mut vm = load_vm(&mode);
        let mut runner = runner(&mut vm);
        let text_end = runner.loaded_layout().risc_code_end_addr();
        runner.set_ecall_return_address(text_end);
        let mut regs = RegistersInOut::new_return_from_ecall();
        assert_that!(
            run_once(&mut runner, &mut regs),
            eq(ILLEGAL_INSTRUCTION_EXIT_VALUE)
        );
    }
}

/// Only an ecall leaves a return address. Resuming a freshly loaded program, or one that has
/// exited, takes the panic exit in the entry stub, before any guest code runs: no gas is spent.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn resume_without_a_pending_ecall_panics() {
    const BUDGET: i64 = 1000;
    for mode in MODES {
        let mut vm = load_vm(&mode);
        let mut runner = runner(&mut vm);
        runner.set_gas(UserCodeGasAmount::new(BUDGET));
        let mut resume = RegistersInOut::new_return_from_ecall();
        assert_that!(
            run_once(&mut runner, &mut resume),
            eq(PANIC_EXIT_VALUE),
            "fresh program in {mode:?}"
        );
        assert_that!(
            runner.get_gas().as_i64(),
            eq(BUDGET),
            "fresh program in {mode:?}"
        );

        let selector = *runner.entry_point_function_index_range().start();
        let mut call = registers(i32::try_from(selector).expect("small index"));
        assert_that!(run_once(&mut runner, &mut call), eq(NORMAL_EXIT_VALUE));
        runner.set_gas(UserCodeGasAmount::new(BUDGET));
        let mut resume_after_exit = RegistersInOut::new_return_from_ecall();
        assert_that!(
            run_once(&mut runner, &mut resume_after_exit),
            eq(PANIC_EXIT_VALUE),
            "after a normal exit in {mode:?}"
        );
        assert_that!(
            runner.get_gas().as_i64(),
            eq(BUDGET),
            "after a normal exit in {mode:?}"
        );
    }
}

/// Gas far below zero is out of gas. Subtracting a block's cost from gas near `i64::MIN` would
/// wrap around to a large positive value, so the runner stores such gas as
/// `UserCodeGasAmount::min_stored()`.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn gas_near_the_minimum_is_out_of_gas() {
    let mut vm = load_vm(&TranspilationMode::default());
    let mut runner = runner(&mut vm);
    runner.set_gas(UserCodeGasAmount::new(i64::MIN));
    let selector = *runner.entry_point_function_index_range().start();
    let mut regs = registers(i32::try_from(selector).expect("small index"));
    assert_that!(run_once(&mut runner, &mut regs), eq(PANIC_EXIT_VALUE));
    assert!(runner.get_gas().is_negative(), "gas must stay negative");
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn bad_ecall_return_address_takes_the_panic_exit() {
    for mode in MODES {
        let mut vm = load_vm(&mode);
        let mut runner = runner(&mut vm);
        let text_end = runner.loaded_layout().risc_code_end_addr().as_u32();
        for return_address in [1, 2, 3, text_end + 4, 0x7FFF_FFFC, 0xFFFF_FFFC, 0x8000_0000] {
            runner.set_ecall_return_address(RiscvAddress::new(return_address));
            let mut regs = RegistersInOut::new_return_from_ecall();
            assert_that!(
                run_once(&mut runner, &mut regs),
                eq(PANIC_EXIT_VALUE),
                "return address {return_address:#x} in {mode:?}"
            );
        }
    }
}

/// A handler that accepts every exit and never asks to halt. Should the run loop keep
/// re-entering anyway, it fails the call after a few exits instead of spinning forever.
struct NeverHalts {
    exits: u32,
}

impl NeverHalts {
    fn record_exit(&mut self) -> Result<(), UserCodeRunnerError> {
        self.exits += 1;
        if self.exits > 3 {
            return Err(UserCodeRunnerError::InternalUserCodePanic);
        }
        Ok(())
    }
}

impl<'a> EcallHandler<TranspiledCodeRunner<'a>> for NeverHalts {
    fn handle_ecall(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        // math-test makes no ecalls.
        Err(UserCodeRunnerError::UnsupportedEcallCode {
            ecall_code: inout_registers.get_function_or_ecall_code(),
        })
    }
    fn handle_user_code_exited(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.record_exit()
    }
    fn handle_internal_user_code_panic(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.record_exit()
    }
    fn handle_internal_user_out_of_gas(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.record_exit()
    }
    fn handle_internal_user_illegal_opcode(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.record_exit()
    }
    fn should_halt(&self) -> bool {
        false
    }
    fn reset_should_halt(&mut self) {}
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn call_entry_point_stops_after_a_terminal_exit_without_halt() {
    let mode = TranspilationMode::default();
    let mut vm = load_vm(&mode);
    let mut runner = runner(&mut vm);
    let selector = *runner.entry_point_function_index_range().start();
    // Normal exit, then an out-of-gas panic.
    for (gas, outcome) in [
        (UserCodeGasAmount::max(), RunOutcome::Returned),
        (UserCodeGasAmount::zero(), RunOutcome::OutOfGas),
    ] {
        runner.set_gas(gas);
        let mut handler = NeverHalts { exits: 0 };
        let mut regs = registers(i32::try_from(selector).expect("small index"));
        assert_that!(
            call_entry_point(&mut runner, &mut handler, &mut regs),
            ok(eq(&outcome))
        );
        assert_that!(handler.exits, eq(1));
    }
}

/// The exits save the guest's registers. Entering them must not overwrite any: on aarch64 the
/// guest's `ra` lives in the link register, which a `BL` to the exit would replace with a host
/// code address.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn panic_exit_keeps_the_guest_return_address() {
    let mode = TranspilationMode::default();
    let mut vm = load_vm(&mode);
    let mut runner = runner(&mut vm);
    let text_end = runner.loaded_layout().risc_code_end_addr().as_i32();
    // No gas: the entry point's first block takes the panic exit.
    runner.set_gas(UserCodeGasAmount::zero());
    let selector = *runner.entry_point_function_index_range().start();
    let mut regs = registers(i32::try_from(selector).expect("small index"));
    assert_that!(run_once(&mut runner, &mut regs), eq(PANIC_EXIT_VALUE));
    // Each entry starts with `ra` at the end of the code.
    assert_that!(runner.read_register(RvPreservedRegister::Ra), eq(text_end));
}
