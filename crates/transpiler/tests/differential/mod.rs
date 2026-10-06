//! Runs one program in the emulator and transpiled for this host, and checks that a host
//! observes the same in both. Shared by the differential test and the differential fuzz target.
//!
//! A run is a sequence of steps, all on one loaded program: the init function, if the program
//! has one, then two calls to `main`, each resumed after every ecall the handler halts in. After
//! each step the host's view is recorded: the result, a0 to a7, every saved register, the gas
//! left and the ecalls served so far. At the end the whole writable memory is compared too.
//!
//! Both runs share the decoder, the gas-block rules (`riscv32::blocks`) and the ELF reader, so
//! this checks the two backends against the emulator, not that shared code.

use std::collections::HashSet;

use riscv32::registers::RvPreservedRegister;
use riscv32_elf::reader::read_riscv_elf;
use riscv32_emulator::{Emulator, GasMetering};
use strum::IntoEnumIterator as _;
use transpiler::execute::{LoaderContext, TranspiledCodeRunner, TranspiledVm};
use transpiler::{TranspilationMode, TranspilerForArchitecture, TranspilerInput};
use user_code_runner::gas::UserCodeGasAmount;
use user_code_runner::runner::{
    CodeRunner, EcallHandler, RegistersInOut, UserCodeRunnerError, VirtualDataPointer,
    call_entry_point, run_init_array,
};

/// The most times one call is resumed after halting in an ecall.
const MAX_RESUMES: usize = 32;

/// The state a run starts from.
pub(crate) struct Start {
    /// a0 to a6 for each call to `main`.
    pub(crate) arguments: [i32; 7],
    /// The saved registers, in `RvPreservedRegister` order. sp, s0 and s1 are then set by the
    /// host: sp to the top of the stack, s0 to the first writable address and s1 to the end of
    /// guest memory, so programs can address both ends of the writable range.
    pub(crate) saved_registers: Vec<i32>,
    /// The gas for the whole run; it carries over from one step to the next.
    pub(crate) gas: i64,
    /// Whether the handler halts in every ecall, so the host has to resume the call.
    pub(crate) halt_on_ecall: bool,
}

/// Serves every ecall: records it and returns a0 + number in a0. Halts in it if asked to.
#[derive(Debug, Default)]
struct Recorder {
    ecalls: Vec<(u32, i32)>,
    halt_on_ecall: bool,
    halted: bool,
}

impl<R: CodeRunner> EcallHandler<R> for Recorder {
    fn handle_ecall(
        &mut self,
        _runner: &mut R,
        registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        let number = registers.get_function_or_ecall_code();
        let a0 = registers.registers()[0];
        self.ecalls.push((number, a0));
        registers.set_register(0, a0.wrapping_add(number as i32));
        self.halted = self.halt_on_ecall;
        Ok(())
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
        self.halted
    }
    fn reset_should_halt(&mut self) {
        self.halted = false;
    }
}

/// What a host observes after one step.
#[derive(Debug, PartialEq)]
struct Snapshot {
    step: String,
    result: String,
    registers_out: Option<RegistersInOut>,
    saved_registers: Vec<i32>,
    gas: i64,
    ecalls: Vec<(u32, i32)>,
}

/// Everything a host observes over a run.
#[derive(Debug, PartialEq)]
struct Observation {
    snapshots: Vec<Snapshot>,
    writable_memory: Vec<u8>,
}

fn snapshot<R: CodeRunner>(
    runner: &R,
    step: String,
    result: String,
    registers_out: Option<RegistersInOut>,
    recorder: &Recorder,
) -> Snapshot {
    Snapshot {
        step,
        result,
        registers_out,
        saved_registers: RvPreservedRegister::iter()
            .map(|register| runner.read_register(register))
            .collect(),
        gas: runner.get_gas().as_i64(),
        ecalls: recorder.ecalls.clone(),
    }
}

/// Runs the init function, if any, and two calls to `main` on `runner` from `start`.
fn observe<R: CodeRunner>(
    runner: &mut R,
    main: u32,
    start: &Start,
    writable: (u32, u32),
) -> Observation {
    for (register, &value) in RvPreservedRegister::iter().zip(&start.saved_registers) {
        runner.write_register(register, value);
    }
    let stack_top = runner.virtual_stack_range().end();
    runner.set_sp(stack_top);
    runner.write_register(RvPreservedRegister::S0, writable.0 as i32);
    runner.write_register(RvPreservedRegister::S1, writable.1 as i32);
    runner.set_gas(UserCodeGasAmount::new(start.gas));
    let mut recorder = Recorder {
        halt_on_ecall: start.halt_on_ecall,
        ..Recorder::default()
    };
    let mut snapshots = Vec::new();

    let initialized = if runner.n_init_array_functions() > 0 {
        let result = run_init_array(runner, &mut recorder);
        let ok = result.is_ok();
        snapshots.push(snapshot(
            runner,
            "init".to_string(),
            format!("{result:?}"),
            None,
            &recorder,
        ));
        ok
    } else {
        true
    };
    if initialized {
        for call in 0..2 {
            let [a0, a1, a2, a3, a4, a5, a6] = start.arguments;
            let mut registers = RegistersInOut::new([a0, a1, a2, a3, a4, a5, a6, main as i32]);
            for resume in 0..=MAX_RESUMES {
                let result = call_entry_point(runner, &mut recorder, &mut registers);
                let halted = matches!(
                    result,
                    Ok(user_code_runner::runner::RunOutcome::HaltedInEcall)
                );
                snapshots.push(snapshot(
                    runner,
                    format!("call {call}, resume {resume}"),
                    format!("{result:?}"),
                    Some(registers.clone()),
                    &recorder,
                ));
                if !halted {
                    break;
                }
            }
        }
    }

    let mut writable_memory = vec![0; (writable.1 - writable.0) as usize];
    runner
        .read_memory(VirtualDataPointer::new(writable.0), &mut writable_memory)
        .unwrap_or_else(|e| panic!("writable memory should be readable: {e}"));
    Observation {
        snapshots,
        writable_memory,
    }
}

/// Runs `elf` from `start` in the emulator and transpiled for this host, and panics if a host
/// would observe any difference. Returns the result of the first call to `main`, or of the
/// init function if it failed.
///
/// # Panics
///
/// Panics if the runs differ, or if the program loads in one but not the other.
pub(crate) fn assert_same_behavior(elf: &[u8], start: &Start) -> String {
    let native = TranspilerForArchitecture::get_native().expect("a native host");
    let entry_points = HashSet::from(["main".to_string()]);
    let symbols = read_riscv_elf(elf, &entry_points)
        .unwrap_or_else(|e| panic!("the program should read: {e}"))
        .symbol_values()
        .clone();
    let writable = (symbols.data_start.as_u32(), symbols.data_end.as_u32());

    let mut emulator = Emulator::load(elf, &entry_points, GasMetering::default())
        .unwrap_or_else(|e| panic!("the program should load in the emulator: {e}"));
    let main = emulator.entry_point_mapping()["main"];
    let emulated = observe(&mut emulator, main, start, writable);

    let output = native
        .transpile_to_native_elf(
            TranspilerInput {
                riscv_elf_bytes: elf,
                entry_points,
            },
            &TranspilationMode::default(),
        )
        .unwrap_or_else(|e| panic!("the program should transpile: {e}"));
    let mut vm = TranspiledVm::load(
        &output.elf_bytes,
        LoaderContext::Untrusted,
        &output.entry_point_mapping,
    )
    .unwrap_or_else(|e| panic!("the transpiled program should load: {e}"));
    assert_eq!(output.entry_point_mapping["main"], main);
    let transpiled = observe(
        &mut TranspiledCodeRunner::new(&mut vm),
        main,
        start,
        writable,
    );

    assert_eq!(
        emulated.snapshots, transpiled.snapshots,
        "the emulator and transpiled code differ"
    );
    if let Some(offset) = emulated
        .writable_memory
        .iter()
        .zip(&transpiled.writable_memory)
        .position(|(a, b)| a != b)
    {
        panic!(
            "writable memory differs at {:#x}",
            writable.0 as usize + offset
        );
    }
    emulated
        .snapshots
        .iter()
        .find(|snapshot| snapshot.step != "init" || !snapshot.result.starts_with("Ok"))
        .map_or_else(String::new, |snapshot| snapshot.result.clone())
}
