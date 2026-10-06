//! Example host for transpiled RISC-V programs.
//!
//! Loads a native ELF produced by the `transpiler` CLI, serves its ecalls with a small test
//! handler, runs the requested entry points, and checks each return value and the gas used.
//! `main` shows the steps every host takes:
//!
//! 1. Read the ELF bytes and call `TranspiledVm::load` with a `LoaderContext` and the
//!    entry-point mapping. The VM owns the loaded program.
//! 2. Build a `TranspiledCodeRunner`, which borrows the VM exclusively.
//! 3. Set the stack pointer to `loaded_layout().stack_top()` and set the starting gas.
//! 4. Call `run_init_array` once.
//! 5. For each call, put the arguments in a0 to a6 and the entry-point index in a7, then call
//!    `call_entry_point`. It runs the program and hands each ecall to the `EcallHandler`.
//!
//! A real host takes the mapping from `TranspilerOutput::entry_point_mapping` and looks up
//! indices by name. This example receives the indices directly on the command line instead.
//!
//! Build it with `cargo build --release --example test_loader`. The smoke tests in
//! `smoke-tests/` run it.

use clap::Parser;
use log::{debug, error, info};
use riscv32::registers::RvPreservedRegister;
use riscv32_elf::types::VirtualDataPointer;
use riscv32_emulator::{Emulator, GasMetering};
use std::collections::{HashMap, HashSet};
use strum::IntoEnumIterator;
use strum::{EnumIter, FromRepr};
use transpiler::execute::{LoaderContext, TranspiledCodeRunner, TranspiledVm};
use user_code_runner::gas::UserCodeGasAmount;
use user_code_runner::runner::{CodeRunner, run_init_array};
use user_code_runner::runner::{
    EcallHandler, RegistersInOut, RunOutcome, UserCodeRunnerError, call_entry_point,
    validate_ecall_input_size,
};

/// Values of a7 when the program stops. Positive values are the ecall numbers the test programs
/// use. `Return`, `Panic` and `IllegalOpcode` are the exit values, which `call_entry_point`
/// handles without calling `handle_ecall`. No test program uses `Halt`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, EnumIter, FromRepr)]
enum TestEcallCode {
    Halt = -3,
    IllegalOpcode = -2,
    Panic = -1,
    Return = 0,
    /// a0 = a0 + a1.
    I32Add = 1,
    /// a0 = a0 * a1.
    I32Mul = 2,
    /// Checks that each saved register except sp and ra holds its own `RvPreservedRegister`
    /// value, adds 100 to each, and returns sp in a1 and ra in a2.
    ReadWriteRegisterStore = 3,
    /// Copies a1 bytes from guest address a0 to a2, then copies them to the top of the stack
    /// and points sp at them.
    ReadWriteMem = 4,
    /// Sets the remaining gas to a0.
    SetGasToW0 = 5,
}

/// Ecall handler for the test programs. It records the return value of a normal exit.
struct TestEcallHandler {
    should_halt: bool,
    return_value: Option<i32>,
    /// The gas the current call started with, reported if it runs out.
    initial_gas: UserCodeGasAmount,
}

impl TestEcallHandler {
    pub(crate) fn new() -> Self {
        Self {
            should_halt: false,
            return_value: None,
            initial_gas: UserCodeGasAmount::zero(),
        }
    }
}

impl<R: CodeRunner> EcallHandler<R> for TestEcallHandler {
    // The ecalls are specific to the test programs, but a real handler has the same shape:
    // read the ecall number and arguments from `inout_registers`, use the runner to access
    // guest memory and registers, write results to a0 to a6 and return.
    fn handle_ecall(
        &mut self,
        code_runner: &mut R,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        let ecall_code = inout_registers.get_function_or_ecall_code();
        debug!("TestEcallHandler::handle_ecall: {ecall_code}");
        match TestEcallCode::from_repr(ecall_code as i32)
            .ok_or(UserCodeRunnerError::UnsupportedEcallCode { ecall_code })?
        {
            TestEcallCode::I32Add => {
                let [a0, a1, ..] = *inout_registers.registers();
                inout_registers.set_register(0, a0.wrapping_add(a1));
                inout_registers.set_return_from_ecall();
                Ok(())
            }
            TestEcallCode::I32Mul => {
                let [a0, a1, ..] = *inout_registers.registers();
                inout_registers.set_register(0, a0.wrapping_mul(a1));
                inout_registers.set_return_from_ecall();
                Ok(())
            }
            TestEcallCode::ReadWriteRegisterStore => {
                for risc_reg in RvPreservedRegister::iter()
                    .filter(|r| !matches!(r, RvPreservedRegister::Sp | RvPreservedRegister::Ra))
                {
                    let cur_val = code_runner.read_register(risc_reg);
                    if cur_val != risc_reg as i32 {
                        return Err(UserCodeRunnerError::EcallFailed {
                            ecall_code,
                            reason: format!(
                                "register {risc_reg:?} holds {cur_val}, expected {}",
                                risc_reg as i32
                            ),
                        });
                    }
                    code_runner.write_register(risc_reg, cur_val + 100);
                }
                inout_registers.set_register(1, code_runner.read_register(RvPreservedRegister::Sp));
                inout_registers.set_register(2, code_runner.read_register(RvPreservedRegister::Ra));
                inout_registers.set_return_from_ecall();
                Ok(())
            }
            TestEcallCode::ReadWriteMem => {
                debug!(
                    "TestEcallCode::ReadWriteMem: {}",
                    format!("{inout_registers:?}").as_str()
                );
                // The size comes from the guest: bound it before sizing a host buffer from it.
                let size_in_bytes = inout_registers.registers()[1] as u32;
                validate_ecall_input_size(size_in_bytes)?;
                let mut v = vec![0; size_in_bytes as usize];
                code_runner.read_memory(
                    VirtualDataPointer::new(inout_registers.registers()[0] as u32),
                    &mut v,
                )?;
                debug!("TestEcallCode::ReadWriteMem: v.len()={}", v.len());
                code_runner.write_memory(
                    VirtualDataPointer::new(inout_registers.registers()[2] as u32),
                    &v,
                )?;
                // Also push the bytes onto the guest stack, 16-byte aligned, and point sp at them:
                // a host can pass data to the guest this way.
                let stack = code_runner.virtual_stack_range();
                let sp = stack
                    .end()
                    .as_u32()
                    .checked_sub(size_in_bytes)
                    .filter(|&sp| sp & !0xF >= stack.start().as_u32())
                    .ok_or(UserCodeRunnerError::StackOverflow)?
                    & !0xF;
                code_runner.write_memory(VirtualDataPointer::new(sp), &v)?;
                code_runner.set_sp(VirtualDataPointer::new(sp));
                inout_registers.set_return_from_ecall();
                Ok(())
            }
            TestEcallCode::SetGasToW0 => {
                debug!(
                    "TestEcallCode::SetGasToW0: {}",
                    format!("{inout_registers:?}").as_str()
                );
                let w0 = inout_registers.registers()[0];
                code_runner.set_gas(UserCodeGasAmount::new(i64::from(w0)));
                inout_registers.set_return_from_ecall();
                Ok(())
            }
            _ => Err(UserCodeRunnerError::UnsupportedEcallCode { ecall_code }),
        }
    }
    fn handle_user_code_exited(
        &mut self,
        _memory_accessor: &mut R,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.should_halt = true;
        self.return_value = Some(inout_registers.registers()[0]);
        Ok(())
    }
    fn handle_internal_user_code_panic(
        &mut self,
        _memory_accessor: &mut R,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.should_halt = true;
        Err(UserCodeRunnerError::InternalUserCodePanic)
    }
    fn handle_internal_user_out_of_gas(
        &mut self,
        _memory_accessor: &mut R,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.should_halt = true;
        Err(UserCodeRunnerError::OutOfGas {
            limit: self.initial_gas,
        })
    }
    fn handle_internal_user_illegal_opcode(
        &mut self,
        _memory_accessor: &mut R,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.should_halt = true;
        Err(UserCodeRunnerError::IllegalOpcode)
    }
    fn should_halt(&self) -> bool {
        self.should_halt
    }
    fn reset_should_halt(&mut self) {
        self.should_halt = false;
    }
}

/// Command-line arguments. The four lists describe one call per position.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path of the native ELF to load.
    #[arg(long)]
    lib_path: String,
    /// Entry-point index of each call (the value passed in a7).
    #[arg(long, value_delimiter = ',')]
    comma_separated_function_codes: Vec<String>,
    /// Value passed in a0 for each call.
    #[arg(long, value_delimiter = ',')]
    comma_separated_inputs: Vec<String>,
    /// Expected return value of each call, or the expected failure: 1 panic, 2 illegal
    /// instruction, 3 out of gas.
    #[arg(long, value_delimiter = ',')]
    comma_separated_expected_outputs: Vec<String>,
    /// Expected gas used by each call. 0 skips the check; a negative value starts the call
    /// with no gas.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    comma_separated_expected_gas_used: Vec<String>,
    /// Load the library as a trusted artifact. Required for output built with `--trusted`;
    /// without it the library is loaded in the untrusted context.
    #[arg(long, default_value_t = false)]
    trusted: bool,
    /// Run the RISC-V ELF at `--lib-path` in the emulator instead of loading a native ELF.
    #[arg(
        long,
        default_value_t = false,
        requires = "comma_separated_entry_points"
    )]
    emulate: bool,
    /// With `--emulate`: the entry-point names, as given to the transpiler.
    #[arg(long, value_delimiter = ',')]
    comma_separated_entry_points: Vec<String>,
}

fn parse_comma_separated_i32(input: &[String]) -> Vec<i32> {
    input
        .iter()
        .map(|s| s.parse::<i32>().expect("Failed to parse integer"))
        .collect()
}

fn main() {
    env_logger::init();
    let args = Args::parse();
    let function_codes = parse_comma_separated_i32(&args.comma_separated_function_codes);
    let inputs = parse_comma_separated_i32(&args.comma_separated_inputs);
    let expected_outputs = parse_comma_separated_i32(&args.comma_separated_expected_outputs);
    let expected_gas_used = parse_comma_separated_i32(&args.comma_separated_expected_gas_used);
    assert_eq!(
        function_codes.len(),
        inputs.len(),
        "function_codes and inputs must have the same length"
    );
    assert_eq!(
        inputs.len(),
        expected_outputs.len(),
        "inputs and expected_outputs must have the same length"
    );
    assert_eq!(
        expected_outputs.len(),
        expected_gas_used.len(),
        "expected_outputs and expected_gas_used must have the same length"
    );

    #[expect(
        clippy::disallowed_methods,
        reason = "Example binary reads the library synchronously before starting any work."
    )]
    let elf_bytes = std::fs::read(&args.lib_path).expect("Failed to read library file");
    let calls = Calls {
        function_codes,
        inputs,
        expected_outputs,
        expected_gas_used,
    };
    if args.emulate {
        let entry_points: HashSet<String> = args.comma_separated_entry_points.into_iter().collect();
        let mut emulator = Emulator::load(&elf_bytes, &entry_points, GasMetering::default())
            .expect("Failed to load the RISC-V ELF into the emulator");
        run_calls(&mut emulator, &calls);
    } else {
        // The command line gives indices, not names, so name each index `function_<index>`.
        let faux_entry_points = calls
            .function_codes
            .iter()
            .map(|&code| (format!("function_{code}"), code as u32))
            .collect::<HashMap<_, _>>();
        let loader_ctx = if args.trusted {
            LoaderContext::Trusted
        } else {
            LoaderContext::Untrusted
        };
        let mut transpiled_vm = TranspiledVm::load(&elf_bytes, loader_ctx, &faux_entry_points)
            .expect("Failed to load the library");
        run_calls(&mut TranspiledCodeRunner::new(&mut transpiled_vm), &calls);
    }
}

/// The calls to make and what each should produce.
struct Calls {
    function_codes: Vec<i32>,
    inputs: Vec<i32>,
    expected_outputs: Vec<i32>,
    expected_gas_used: Vec<i32>,
}

/// Runs the init functions, then each call, and checks its result and gas. The same code runs
/// transpiled code and the emulator.
fn run_calls<R: CodeRunner>(runner: &mut R, calls: &Calls) {
    // One handler serves every call.
    let mut ecall_handler = TestEcallHandler::new();

    // The runner keeps sp and gas between runs. Set them before the init functions run.
    let stack_top = runner.virtual_stack_range().end();
    runner.set_sp(stack_top);
    runner.set_gas(UserCodeGasAmount::max());

    run_init_array(runner, &mut ecall_handler).expect("Failed to run init array functions");

    for (((function_code, input), expected_output), expected_gas) in calls
        .function_codes
        .iter()
        .zip(&calls.inputs)
        .zip(&calls.expected_outputs)
        .zip(&calls.expected_gas_used)
    {
        // A negative expected gas marks a call that must run out of gas, so start it with none.
        let initial_gas = if *expected_gas >= 0 {
            UserCodeGasAmount::max()
        } else {
            UserCodeGasAmount::zero()
        };
        runner.set_gas(initial_gas);
        ecall_handler.initial_gas = initial_gas;
        // A result from an earlier call must not pass for this one.
        ecall_handler.return_value = None;
        let mut input_regs = RegistersInOut::new([*input, 0, 0, 0, 0, 0, 0, *function_code]);
        let res = call_entry_point(runner, &mut ecall_handler, &mut input_regs);
        match res {
            Err(e) => {
                error!("{e}");
                // The expected output doubles as a failure code: 1 panic, 2 illegal opcode,
                // 3 out of gas. Match the variant, not the message, which holds run-specific
                // details.
                let matches_expectation = match *expected_output {
                    1 => matches!(e, UserCodeRunnerError::InternalUserCodePanic),
                    2 => matches!(e, UserCodeRunnerError::IllegalOpcode),
                    3 => matches!(e, UserCodeRunnerError::OutOfGas { .. }),
                    // A call that should have returned a value failed: report the error.
                    other => panic!(
                        "expected output {other} but the run failed with {e:?}; \
                         failure codes are 1 panic, 2 illegal opcode, 3 out of gas"
                    ),
                };
                assert!(
                    matches_expectation,
                    "expected failure code {expected_output} but got {e:?}"
                );
            }
            Ok(RunOutcome::Returned) => {
                info!("test res = {:?}", ecall_handler.return_value);
                assert_eq!(
                    ecall_handler.return_value,
                    Some(*expected_output),
                    "Expected output {}, but got {:?}",
                    expected_output,
                    ecall_handler.return_value
                )
            }
            // Panics, illegal instructions and running out of gas are errors here, so any other
            // outcome is a halt the test programs never ask for.
            Ok(outcome) => panic!("expected a normal return, but the call ended with {outcome:?}"),
        }
        if *expected_gas > 0 {
            let remaining_gas = runner.get_gas();
            assert!(!remaining_gas.is_negative(), "Remaining gas is negative");
            let gas_used = UserCodeGasAmount::max()
                .sub(&remaining_gas)
                .expect("Failed to compute gas used")
                .as_i64();
            assert_eq!(
                gas_used, *expected_gas as i64,
                "Expected gas used {expected_gas}, but got {gas_used}",
            );
        }
    }
}
