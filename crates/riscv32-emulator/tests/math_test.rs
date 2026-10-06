//! The emulator runs the vendored math-test program on any host, with the same results and gas
//! the smoke tests expect from transpiled code.

use std::collections::HashSet;
use std::path::PathBuf;

use riscv32::gas_cost::{RvGasCost, RvGasCostScheme};
use riscv32_emulator::{Emulator, EmulatorError, GasMetering};
use user_code_runner::gas::UserCodeGasAmount;
use user_code_runner::runner::{
    CodeRunner, EcallHandler, MemoryAccessor, RegistersInOut, RunOutcome, UserCodeRunnerError,
    VirtualDataPointer, call_entry_point,
};

const ENTRY_POINTS: [&str; 4] = [
    "factorial_x",
    "sum_1_to_x",
    "sum_cubes_1_to_x",
    "sum_squares_1_to_x",
];

fn math_test_elf() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/rust_programs/math-test/artifacts/math_test.o");
    #[expect(
        clippy::disallowed_methods,
        reason = "Test reads a fixture synchronously."
    )]
    std::fs::read(&path).expect("math-test fixture should be readable")
}

fn load(metering: GasMetering) -> Emulator {
    let entry_points: HashSet<String> = ENTRY_POINTS.iter().map(ToString::to_string).collect();
    Emulator::load(&math_test_elf(), &entry_points, metering).expect("math-test should load")
}

/// The program makes no ecalls; every exit is accepted.
struct NoEcalls;

impl<R: CodeRunner> EcallHandler<R> for NoEcalls {
    fn handle_ecall(
        &mut self,
        _runner: &mut R,
        registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Err(UserCodeRunnerError::UnsupportedEcallCode {
            ecall_code: registers.get_function_or_ecall_code(),
        })
    }
    fn handle_user_code_exited(
        &mut self,
        _runner: &mut R,
        _registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn handle_internal_user_code_panic(
        &mut self,
        _runner: &mut R,
        _registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn handle_internal_user_out_of_gas(
        &mut self,
        _runner: &mut R,
        _registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn handle_internal_user_illegal_opcode(
        &mut self,
        _runner: &mut R,
        _registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn should_halt(&self) -> bool {
        false
    }
    fn reset_should_halt(&mut self) {}
}

/// Calls `name` with `x` and returns the outcome, the value in a0 and the gas used.
fn call(emulator: &mut Emulator, name: &str, x: i32, gas: i64) -> (RunOutcome, i32, i64) {
    let index = emulator.entry_point_mapping()[name];
    let stack_top = emulator.virtual_stack_range().end();
    emulator.set_sp(stack_top);
    emulator.set_gas(UserCodeGasAmount::new(gas));
    let mut registers = RegistersInOut::new([x, 0, 0, 0, 0, 0, 0, index as i32]);
    let outcome =
        call_entry_point(emulator, &mut NoEcalls, &mut registers).expect("the call should run");
    let used = gas - emulator.get_gas().as_i64();
    (outcome, registers.get_normal_return_value(), used)
}

#[test]
fn math_test_runs_with_the_transpiled_results_and_gas() {
    let mut emulator = load(GasMetering::default());
    // The smoke tests' expectations for transpiled code.
    for (name, x, value, gas) in [
        ("sum_squares_1_to_x", 10, 385, 62),
        ("sum_cubes_1_to_x", 10, 3025, 72),
        ("sum_1_to_x", 10, 55, 52),
        ("factorial_x", 9, 362_880, 47),
        ("sum_1_to_x", 9, 45, 47),
    ] {
        assert_eq!(
            call(&mut emulator, name, x, 1_000_000),
            (RunOutcome::Returned, value, gas),
            "{name}({x})"
        );
    }
}

#[test]
fn running_out_of_gas_stops_the_call() {
    let mut emulator = load(GasMetering::default());
    let (outcome, _, _) = call(&mut emulator, "sum_1_to_x", 10, 20);
    assert_eq!(outcome, RunOutcome::OutOfGas);
    assert!(emulator.get_gas().is_negative());
}

#[test]
fn unmetered_runs_use_no_gas() {
    let mut emulator = load(GasMetering::Unmetered);
    assert_eq!(
        call(&mut emulator, "factorial_x", 5, 0),
        (RunOutcome::Returned, 120, 0)
    );
}

#[test]
fn zero_gas_cost_is_rejected() {
    let entry_points = HashSet::from(["sum_1_to_x".to_string()]);
    let metering = GasMetering::Metered(RvGasCostScheme::Fixed(RvGasCost::zero()));
    assert!(matches!(
        Emulator::load(&math_test_elf(), &entry_points, metering),
        Err(EmulatorError::ZeroGasCost)
    ));
}

#[test]
fn resume_without_a_pending_ecall_panics() {
    let mut emulator = load(GasMetering::default());
    emulator.set_gas(UserCodeGasAmount::new(1000));
    let mut registers = RegistersInOut::new_return_from_ecall();
    emulator
        .run_till_ecall_or_return(&mut registers)
        .expect("the run should complete");
    assert_eq!(
        registers.get_user_code_exit_value(),
        user_code_runner::runner::PANIC_EXIT_VALUE
    );
}

#[test]
fn host_memory_access_is_bounded_like_transpiled_code() {
    let mut emulator = load(GasMetering::default());
    let mut buf = [0_u8; 4];
    // Code is readable but not writable; the stack is writable.
    assert!(
        emulator
            .read_memory(VirtualDataPointer::new(0), &mut buf)
            .is_ok()
    );
    assert!(matches!(
        emulator.write_memory(VirtualDataPointer::new(0), &buf),
        Err(UserCodeRunnerError::GuestMemoryOutOfBounds { addr: 0, len: 4 })
    ));
    let stack_bottom = emulator.virtual_stack_range().start();
    assert!(emulator.write_memory(stack_bottom, &buf).is_ok());
    assert!(matches!(
        emulator.read_memory(VirtualDataPointer::new(u32::MAX - 1), &mut buf),
        Err(UserCodeRunnerError::GuestMemoryOutOfBounds { .. })
    ));
}
