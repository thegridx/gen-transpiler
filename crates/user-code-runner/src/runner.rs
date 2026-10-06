use std::ops::RangeInclusive;

use log::debug;
use riscv32::registers::RvPreservedRegister;
use riscv32_elf::data_layout::ECALL_DATA_SIZE_BYTES;
pub use riscv32_elf::types::{VirtualDataPointer, VirtualDataRange, VirtualDataSize};
use thiserror::Error;

use crate::gas::UserCodeGasAmount;

/// The value in a7 after the user code returns from the called function.
pub const NORMAL_EXIT_VALUE: i32 = 0;
/// The value in a7 after the user code panics or runs out of gas.
pub const PANIC_EXIT_VALUE: i32 = -1;
/// The value in a7 after the user code reaches an illegal instruction.
pub const ILLEGAL_INSTRUCTION_EXIT_VALUE: i32 = -2;

/// The largest ecall input a host should copy out of guest memory: the size of the `.ecall_data`
/// buffer. The size comes from a guest register, so without a bound a single ecall could make
/// the host allocate up to 4 GiB. See [`validate_ecall_input_size`].
pub const MAX_ECALL_INPUT_SIZE_IN_BYTES: u32 = ECALL_DATA_SIZE_BYTES;

/// Why running user code failed. The runner functions return the code runner's errors and the
/// handler's; a handler picks the variant that fits, such as [`Self::UnsupportedEcallCode`] for
/// an ecall it does not know.
#[derive(Error, Debug)]
pub enum UserCodeRunnerError {
    /// The handler does not support this ecall number.
    #[error("unsupported ecall code {ecall_code:?}")]
    UnsupportedEcallCode { ecall_code: u32 },
    /// The handler supports the ecall but could not complete it.
    #[error("ecall {ecall_code} failed: {reason}")]
    EcallFailed { ecall_code: u32, reason: String },
    /// The index in a7 names no entry point.
    #[error("bad function index {function_index}")]
    BadFunctionIndex { function_index: u32 },
    /// The user code panicked, and the handler treats that as an error.
    #[error("internal user code panic")]
    InternalUserCodePanic,
    /// The user code ran out of gas, and the handler treats that as an error.
    #[error("out of gas: limit {limit:?} exhausted")]
    OutOfGas {
        /// The gas the call started with.
        limit: UserCodeGasAmount,
    },
    /// The user code reached an illegal instruction, and the handler treats that as an error.
    #[error("illegal code")]
    IllegalOpcode,
    /// Data the host wanted to put on the guest stack does not fit.
    #[error("stack overflow")]
    StackOverflow,
    /// The code runner failed. The source is the runner's own error type, which a host can
    /// recover with `downcast_ref`.
    #[error("code runner error: {0}")]
    Runner(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Gas arithmetic overflowed.
    #[error("Overflow error: {0}")]
    OverflowError(String),
    /// A host read or write of guest memory falls outside the range it may access.
    #[error("guest memory access of {len} bytes at {addr:#x} is out of bounds")]
    GuestMemoryOutOfBounds { addr: u32, len: u32 },
    /// An ecall's input is larger than [`MAX_ECALL_INPUT_SIZE_IN_BYTES`].
    #[error(
        "ecall input size {input_size_in_bytes} exceeds the maximum of {max_input_size_in_bytes} bytes"
    )]
    EcallInputTooLarge {
        input_size_in_bytes: u32,
        max_input_size_in_bytes: u32,
    },
    /// An init function did not return normally; see [`run_init_array`].
    #[error("init function {function_index} did not return normally: {outcome:?}")]
    InitFunctionFailed {
        function_index: u32,
        /// How the function stopped instead.
        outcome: RunOutcome,
    },
}

/// How a call to user code ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The function returned; its results are in a0 to a6.
    Returned,
    /// The code panicked, for example on an out-of-bounds access.
    Panicked,
    /// The code ran out of gas.
    OutOfGas,
    /// The code reached an illegal instruction.
    IllegalInstruction,
    /// The handler asked to halt while serving an ecall. Calling [`call_entry_point`] again
    /// with the same registers resumes after that ecall.
    HaltedInEcall,
}

/// Rejects a guest-supplied ecall input size that exceeds
/// [`MAX_ECALL_INPUT_SIZE_IN_BYTES`].
///
/// Call this *before* allocating any host buffer sized from the guest's input
/// size register, typically in the host's ecall handler.
///
/// # Errors
///
/// Returns [`UserCodeRunnerError::EcallInputTooLarge`] when
/// `input_size_in_bytes` is above [`MAX_ECALL_INPUT_SIZE_IN_BYTES`].
pub fn validate_ecall_input_size(input_size_in_bytes: u32) -> Result<(), UserCodeRunnerError> {
    if input_size_in_bytes > MAX_ECALL_INPUT_SIZE_IN_BYTES {
        return Err(UserCodeRunnerError::EcallInputTooLarge {
            input_size_in_bytes,
            max_input_size_in_bytes: MAX_ECALL_INPUT_SIZE_IN_BYTES,
        });
    }
    Ok(())
}

/// Trait for accessing memory during ecall processing.
///
/// This trait provides an abstraction layer for reading from and writing to user code memory,
/// allowing ecall handlers to exchange data with executing user code regardless of the
/// backend that runs it.
pub trait MemoryAccessor {
    /// Retrieves the current stack pointer value.
    ///
    /// # Returns
    ///
    /// * `VirtualDataPointer` - The current stack pointer address.
    fn get_sp(&self) -> VirtualDataPointer;
    /// Sets the stack pointer value.
    ///
    /// # Arguments
    ///
    /// * `sp` - The new stack pointer address.
    fn set_sp(&mut self, sp: VirtualDataPointer);
    /// Reads a saved guest register. Between runs, it holds the guest's value as of its last
    /// exit.
    fn read_register(&self, register: RvPreservedRegister) -> i32;
    /// Writes a saved guest register; the guest sees the value when it next resumes.
    fn write_register(&mut self, register: RvPreservedRegister, value: i32);
    /// Retrieves the current gas value.
    ///
    /// # Returns
    ///
    /// * `UserCodeGasAmount` - The current gas value.
    fn get_gas(&self) -> UserCodeGasAmount;
    /// Sets the value of the gas register. Values below [`UserCodeGasAmount::min_stored`] are
    /// stored as that value; they are out of gas either way.
    ///
    /// # Arguments
    ///
    /// * `gas` - The new gas value.
    fn set_gas(&mut self, gas: UserCodeGasAmount);
    /// Reads `buf.len()` bytes of guest memory, starting at `addr`, into `buf`.
    ///
    /// # Errors
    ///
    /// Returns [`UserCodeRunnerError::GuestMemoryOutOfBounds`] if the range is outside the
    /// memory the host may read.
    fn read_memory(
        &self,
        addr: VirtualDataPointer,
        buf: &mut [u8],
    ) -> Result<(), UserCodeRunnerError>;

    /// Writes a block of data to the user code address space.
    ///
    /// # Arguments
    ///
    /// * `addr` - The starting address in user code memory to write to.
    /// * `data` - The data to write to memory.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The data was successfully written to memory.
    /// * `Err(UserCodeRunnerError)` - If the memory write fails due to invalid address or access violation.
    fn write_memory(
        &mut self,
        addr: VirtualDataPointer,
        data: &[u8],
    ) -> Result<(), UserCodeRunnerError>;
}

/// Trait for processing system calls (ecalls) from user code.
///
/// This trait defines the interface for handling ecalls, which are the primary mechanism
/// for user code to interact with the runtime. Implementers decode ecall codes, manage
/// user code lifecycle events (exit, panic), and control execution flow through halt semantics.
///
/// # Type Parameters
///
/// * `MA` - The memory accessor type used to read/write user code memory during ecall processing.
pub trait EcallHandler<MA: MemoryAccessor> {
    /// Processes a system call (ecall) from user code.
    ///
    /// This method is invoked when user code executes an ecall instruction. The handler
    /// decodes the ecall code from registers, reads arguments from memory, executes the
    /// requested operation, and writes results back.
    ///
    /// Do not call [`call_entry_point`] or [`run_init_array`] on the same runner from here. The
    /// program is suspended at the ecall, and a nested call overwrites its saved registers and
    /// the address it resumes at. That is not memory-unsafe, but the program then resumes in the
    /// wrong place.
    ///
    /// # Arguments
    ///
    /// * `memory_accessor` - Provides access to user code memory for reading arguments and writing results.
    /// * `inout_registers` - The register state containing ecall code, arguments, and return values.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The ecall was successfully processed.
    /// * `Err(UserCodeRunnerError)` - If the ecall fails or the ecall code is unsupported.
    fn handle_ecall(
        &mut self,
        memory_accessor: &mut MA,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError>;

    /// Handles user code exit scenarios.
    ///
    /// This method is called when user code terminates normally (e.g., by returning from main
    /// or calling an exit function). The handler may log the exit, save state, or perform cleanup.
    ///
    /// # Arguments
    ///
    /// * `memory_accessor` - Provides access to user code memory.
    /// * `inout_registers` - The register state at the time of exit, including return values.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The exit was successfully handled.
    /// * `Err(UserCodeRunnerError)` - If handling the exit fails or the exit was unexpected.
    fn handle_user_code_exited(
        &mut self,
        memory_accessor: &mut MA,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError>;

    /// Handles internal panics that occur within user code, resulting from out-of-sandbox memory access
    ///
    /// This method is invoked when user code violates the sandbox's safety rules. It is issued by the loader or
    ///  transpiler injected code.
    ///
    /// # Arguments
    ///
    /// * `memory_accessor` - Provides access to user code memory to read panic information.
    /// * `inout_registers` - The register state at the time of panic.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The panic was successfully handled.
    /// * `Err(UserCodeRunnerError)` - If handling the panic fails.
    fn handle_internal_user_code_panic(
        &mut self,
        memory_accessor: &mut MA,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError>;

    /// Handles internal panics that occur within user code resulting from running out of gas
    ///
    /// This method is invoked when user code violates the sandbox's safety rules. It is issued by the loader or
    ///  transpiler injected code.
    ///
    /// # Arguments
    ///
    /// * `gas` - The (negative) amount of gas remaining (including deduction for the next pseudo basic block)
    /// * `memory_accessor` - Provides access to user code memory to read panic information.
    /// * `inout_registers` - The register state at the time of panic.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The panic was successfully handled.
    /// * `Err(UserCodeRunnerError)` - If handling the panic fails.
    fn handle_internal_user_out_of_gas(
        &mut self,
        memory_accessor: &mut MA,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError>;

    /// Handles internal panics that occur within user code resulting from encountering
    /// and illegal opcode
    ///
    /// This method is invoked when user code violates the sandbox's safety rules. It is issued by the loader or
    ///  transpiler injected code.
    ///
    /// # Arguments
    ///
    /// * `memory_accessor` - Provides access to user code memory to read panic information.
    /// * `inout_registers` - The register state at the time of panic.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The panic was successfully handled.
    /// * `Err(UserCodeRunnerError)` - If handling the panic fails.
    fn handle_internal_user_illegal_opcode(
        &mut self,
        memory_accessor: &mut MA,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError>;

    /// Checks whether execution should halt.
    ///
    /// This method indicates whether the handler has requested execution to pause,
    /// typically while serving an ecall.
    ///
    /// # Returns
    ///
    /// * `true` - Execution should halt and return control to the caller.
    /// * `false` - Execution should continue normally.
    fn should_halt(&self) -> bool;

    /// Resets the halt flag to allow execution to continue.
    ///
    /// This method clears any halt request, typically called before resuming
    /// execution after a halt.
    fn reset_should_halt(&mut self);
}

/// Trait for executing user code until an ecall or return.
///
/// This trait represents the execution backend for user code, for example transpiled native code.
/// Implementers execute user code instructions and pause execution when encountering an ecall
/// (system call) or return instruction, allowing the runtime to handle the event.
///
/// The trait extends `MemoryAccessor` to provide memory access capabilities during execution.
pub trait CodeRunner: MemoryAccessor {
    /// Executes user code until an ecall instruction or function return is encountered.
    ///
    /// This method runs user code starting from the program counter and register state
    /// specified in `inout_registers`. Execution continues until:
    /// - An ecall instruction is executed (system call to the runtime)
    /// - A return instruction matches the end return address (function completion)
    /// - An error occurs during execution
    ///
    /// # Arguments
    ///
    /// * `inout_registers` - Initial register state for execution and receives final register
    ///   state when execution pauses. Contains function code, arguments, and will hold return values.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - User code executed successfully until an ecall or return.
    /// * `Err(UserCodeRunnerError)` - If execution fails due to illegal instruction, memory fault,
    ///   or other runtime errors.
    fn run_till_ecall_or_return(
        &mut self,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError>;

    /// Returns the number of init array functions to run before the main entry point.
    ///
    /// # Returns
    ///
    /// The count of init array functions that need to be executed during initialization.
    fn n_init_array_functions(&self) -> usize;

    /// Returns the number of entry point functions available in the loaded code.
    ///
    /// # Returns
    ///
    /// The count of entry point functions in the loaded program.
    fn n_entry_point_functions(&self) -> usize;

    /// The indices of the init functions, which come first: `1..=n_init_array_functions()`.
    fn init_array_function_index_range(&self) -> RangeInclusive<u32> {
        1..=self.n_init_array_functions() as u32
    }

    /// The indices of the entry points, which follow the init functions.
    fn entry_point_function_index_range(&self) -> RangeInclusive<u32> {
        (self.n_init_array_functions().saturating_add(1) as u32)
            ..=(self
                .n_init_array_functions()
                .saturating_add(self.n_entry_point_functions()) as u32)
    }

    /// Retrieves the virtual stack bottom & top addresses.
    ///
    /// # Returns
    ///
    /// * `VirtualDataRange` - The stack bottom and top addresses.
    fn virtual_stack_range(&self) -> VirtualDataRange;
}

/// Register snapshot for passing arguments and results between user code and the runtime.
///
/// This structure holds a snapshot of the RISC-V register file used for communication
/// with transpiled user code. The registers follow the RISC-V calling convention
/// and are used both as input (passing arguments to user code) and output (receiving results
/// and control codes from user code).
///
/// # Registers
///
/// The eight registers are the RISC-V a0 to a7, at indices 0 to 7:
///
/// - **a0 to a6** (indices 0 to 6) pass arguments to user code and receive its results. They
///   follow the RISC-V calling convention.
/// - **a7** (index 7) is the control register. On input, the host puts the entry-point index
///   there, or 0 to return from an ecall. On output, user code puts there:
///   - a normal return ([`NORMAL_EXIT_VALUE`])
///   - a panic ([`PANIC_EXIT_VALUE`])
///   - an illegal instruction ([`ILLEGAL_INSTRUCTION_EXIT_VALUE`])
///   - any other value: the number of the ecall it requests
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistersInOut {
    registers: [i32; 8],
}

impl RegistersInOut {
    const FUNCTION_OR_ECALL_CODE_INDEX: usize = 7;
    const USER_CODE_EXIT_VALUE_INDEX: usize = 7;
    const NORMAL_RETURN_VALUE_INDEX: usize = 0;
    const RETURN_FROM_ECALL_FUNCTION_CODE: i32 = 0;

    /// Registers holding `register_values`, a0 to a7.
    pub fn new(register_values: [i32; 8]) -> Self {
        Self {
            registers: register_values,
        }
    }

    /// Registers that call the function with index `function_code`, with a0 to a6 zero.
    pub fn new_with_function_code(function_code: u32) -> Self {
        let mut registers = [0; 8];
        registers[Self::FUNCTION_OR_ECALL_CODE_INDEX] = function_code as i32;
        Self { registers }
    }

    /// Registers that resume the user code after the ecall it stopped at.
    pub fn new_return_from_ecall() -> Self {
        Self::new_with_function_code(Self::RETURN_FROM_ECALL_FUNCTION_CODE as u32)
    }

    /// The registers a0 to a7.
    pub fn registers(&self) -> &[i32; 8] {
        &self.registers
    }

    /// Sets register a`index` to `value`, for example to hand back an ecall's result.
    ///
    /// # Panics
    ///
    /// Panics if `index` is not below 8.
    pub fn set_register(&mut self, index: usize, value: i32) {
        self.registers[index] = value;
    }

    /// Sets all of a0 to a7.
    pub fn set_registers(&mut self, registers: [i32; 8]) {
        self.registers = registers;
    }

    /// a7: the function index on input, or the ecall number after the code stops at an ecall.
    pub fn get_function_or_ecall_code(&self) -> u32 {
        self.registers[Self::FUNCTION_OR_ECALL_CODE_INDEX] as u32
    }

    /// a0: the function's return value after a normal exit.
    pub fn get_normal_return_value(&self) -> i32 {
        self.registers[Self::NORMAL_RETURN_VALUE_INDEX]
    }

    /// a7 after the code stops: an exit value such as [`NORMAL_EXIT_VALUE`], or an ecall number.
    pub fn get_user_code_exit_value(&self) -> i32 {
        self.registers[Self::USER_CODE_EXIT_VALUE_INDEX]
    }

    /// Sets a7 to resume the user code after its ecall; a0 to a6 hold the ecall's results.
    pub fn set_return_from_ecall(&mut self) {
        self.registers[Self::FUNCTION_OR_ECALL_CODE_INDEX] = Self::RETURN_FROM_ECALL_FUNCTION_CODE;
    }

    fn is_return_from_ecall(&self) -> bool {
        self.registers[Self::FUNCTION_OR_ECALL_CODE_INDEX] == Self::RETURN_FROM_ECALL_FUNCTION_CODE
    }
}

/// Runs the user code from `inout_registers` until it exits or the handler halts, serving each
/// ecall in between, and says which. A normal, panic or illegal-instruction exit ends the run
/// even if the handler did not ask to halt: there is no ecall to return from.
///
/// The outcome is tracked here rather than read back from a7 afterwards: after an ecall, a7
/// holds the resume code, which equals [`NORMAL_EXIT_VALUE`].
fn run_until_exit<CR, EH>(
    code_runner: &mut CR,
    ecall_handler: &mut EH,
    inout_registers: &mut RegistersInOut,
) -> Result<RunOutcome, UserCodeRunnerError>
where
    CR: CodeRunner,
    EH: EcallHandler<CR>,
{
    loop {
        debug!(
            "before run_till_ecall_or_return registers: {:?} gas: {:#x}",
            inout_registers,
            code_runner.get_gas().as_i64()
        );
        code_runner.run_till_ecall_or_return(inout_registers)?;
        let gas = code_runner.get_gas();
        debug!(
            "after run_till_ecall_or_return registers: {:?} gas: {:#x}",
            inout_registers,
            gas.as_i64()
        );
        match inout_registers.get_user_code_exit_value() {
            NORMAL_EXIT_VALUE => {
                ecall_handler.handle_user_code_exited(code_runner, inout_registers)?;
                return Ok(RunOutcome::Returned);
            }
            PANIC_EXIT_VALUE if gas.is_negative() => {
                ecall_handler.handle_internal_user_out_of_gas(code_runner, inout_registers)?;
                return Ok(RunOutcome::OutOfGas);
            }
            PANIC_EXIT_VALUE => {
                ecall_handler.handle_internal_user_code_panic(code_runner, inout_registers)?;
                return Ok(RunOutcome::Panicked);
            }
            ILLEGAL_INSTRUCTION_EXIT_VALUE => {
                ecall_handler.handle_internal_user_illegal_opcode(code_runner, inout_registers)?;
                return Ok(RunOutcome::IllegalInstruction);
            }
            _ => {
                ecall_handler.handle_ecall(code_runner, inout_registers)?;
                inout_registers.set_return_from_ecall();
                if ecall_handler.should_halt() {
                    return Ok(RunOutcome::HaltedInEcall);
                }
            }
        }
    }
}

/// Orchestrates the user-code execution loop with ecall handling.
///
/// This function implements the core execution model for running sandboxed user code.
/// It repeatedly executes user code until an ecall or return, dispatches control to the
/// appropriate handler, and continues until the ecall handler signals that execution should halt.
///
/// # Execution Flow
///
/// 1. Runs user code via `code_runner.run_till_ecall_or_return()` until it yields control
/// 2. Inspects a7 ([`RegistersInOut::get_user_code_exit_value`]) to determine the reason:
///    - [`NORMAL_EXIT_VALUE`]: User code returned → calls `handle_user_code_exited()`
///    - [`PANIC_EXIT_VALUE`]: Panic, or out of gas when the gas is negative → calls
///      `handle_internal_user_code_panic()` or `handle_internal_user_out_of_gas()`
///    - [`ILLEGAL_INSTRUCTION_EXIT_VALUE`]: → calls `handle_internal_user_illegal_opcode()`
///    - Any other value: Ecall request → calls `handle_ecall()`, then
///      [`RegistersInOut::set_return_from_ecall`] to resume after the ecall
/// 3. Repeats until a normal, panic or illegal-instruction exit has been handled, or until
///    `ecall_handler.should_halt()` returns `true` after an ecall. The halt flag is reset when
///    the call starts, so a flag left over from an earlier call does not end this one.
///
/// # Type Parameters
///
/// * `EcallHandlerType` - A type implementing [`EcallHandler`] for processing system calls,
///   user code lifecycle events (exit, panic), and controlling execution flow via
///   `should_halt()`. This is typically the host runtime or a similar execution controller.
///
/// * `CodeRunnerType` - A type implementing [`CodeRunner`] that executes the user code until
///   an ecall or return. This is typically a wrapper around transpiled native code (e.g.,
///   `TranspiledCodeRunner`).
///
/// # Parameters
///
/// * `code_runner` - Mutable reference to the code executor. Its state may be modified during
///   execution (e.g., tracking instruction pointers, maintaining execution context).
///
/// * `ecall_handler` - Mutable reference to the ecall/lifecycle event handler. Processes
///   system calls from user code and determines when execution should terminate.
///
/// * `inout_registers` - Mutable reference to the register snapshot used for bidirectional
///   communication with user code. On entry, contains initial arguments; on exit, contains
///   final results. Modified throughout execution as user code runs and ecalls are serviced.
///
/// # Returns
///
/// * `Ok(outcome)` - How the call ended: see [`RunOutcome`]. The final register state is in
///   `inout_registers`.
///
/// # Errors
///
/// Returns [`UserCodeRunnerError`] in the following cases:
///
/// * [`UserCodeRunnerError::Runner`] - Propagated from `code_runner.run_till_ecall_or_return()`:
///   the code runner itself failed.
///
/// * [`UserCodeRunnerError::UnsupportedEcallCode`] - Propagated from `ecall_handler.handle_ecall()`.
///   The user code requested an ecall that the handler does not support.
///
/// * [`UserCodeRunnerError::InternalUserCodePanic`] - Propagated from `ecall_handler.handle_internal_user_code_panic()`.
///   The user code panicked or executed an illegal instruction, and the handler rejected the error.
///
/// * Any other [`UserCodeRunnerError`] variants propagated from the handler methods based on
///   application-specific error conditions.
pub fn call_entry_point<CR, EH>(
    code_runner: &mut CR,
    ecall_handler: &mut EH,
    inout_registers: &mut RegistersInOut,
) -> Result<RunOutcome, UserCodeRunnerError>
where
    CR: CodeRunner,
    EH: EcallHandler<CR>,
{
    debug!(
        "starting call_entry_point with initial registers: {:?} gas: {:#x}",
        inout_registers,
        code_runner.get_gas().as_i64()
    );

    if !inout_registers.is_return_from_ecall()
        && !code_runner
            .entry_point_function_index_range()
            .contains(&inout_registers.get_function_or_ecall_code())
    {
        return Err(UserCodeRunnerError::BadFunctionIndex {
            function_index: inout_registers.get_function_or_ecall_code(),
        });
    }

    ecall_handler.reset_should_halt();
    let outcome = run_until_exit(code_runner, ecall_handler, inout_registers)?;
    debug!(
        "finished call_entry_point ({outcome:?}) with final registers: {:?} gas: {:#x}",
        inout_registers,
        code_runner.get_gas().as_i64()
    );
    Ok(outcome)
}
/// Runs the static constructors (init array functions) once, in order.
///
/// Call it exactly once after loading a program, before the first entry point, with the stack
/// pointer and gas already set; calling it again runs the constructors again. Each init function
/// runs like an entry point: its ecalls go to `ecall_handler`, and a panic, illegal-instruction
/// or out-of-gas exit goes to the handler's matching method. The halt flag and the stack pointer
/// are reset before each one, so one constructor cannot leave the next a shifted stack.
///
/// # Errors
///
/// Returns an error from the code runner or the handler, or
/// [`UserCodeRunnerError::InitFunctionFailed`] if an init function does not return normally
/// (including when the handler accepts its panic, or halts during one of its ecalls). The
/// program's globals may then be half-initialized, so the host should not call its entry
/// points.
pub fn run_init_array<CR, EH>(
    code_runner: &mut CR,
    ecall_handler: &mut EH,
) -> Result<(), UserCodeRunnerError>
where
    CR: CodeRunner,
    EH: EcallHandler<CR>,
{
    debug!(
        "starting run_init_array with {} init array functions",
        code_runner.init_array_function_index_range().count()
    );
    let stack_pointer = code_runner.get_sp();
    for function_index in code_runner.init_array_function_index_range() {
        code_runner.set_sp(stack_pointer);
        ecall_handler.reset_should_halt();
        let mut inout_registers = RegistersInOut::new_with_function_code(function_index);
        let outcome = run_until_exit(code_runner, ecall_handler, &mut inout_registers)?;
        if outcome != RunOutcome::Returned {
            return Err(UserCodeRunnerError::InitFunctionFailed {
                function_index,
                outcome,
            });
        }
    }
    debug!("finished run_init_array loop");
    Ok(())
}
#[cfg(test)]
mod tests {
    use googletest::prelude::{assert_that, eq};
    use riscv32_elf::data_layout::ECALL_DATA_SIZE_BYTES;
    use rstest::rstest;

    use super::{MAX_ECALL_INPUT_SIZE_IN_BYTES, UserCodeRunnerError, validate_ecall_input_size};

    /// The ecall *input* bound is deliberately the same limit that already
    /// caps the ecall *return* value — the fixed `.ecall_data` section size —
    /// and not a second, independently chosen number.
    #[test]
    fn max_ecall_input_size_reuses_the_ecall_return_value_limit() {
        assert_that!(MAX_ECALL_INPUT_SIZE_IN_BYTES, eq(ECALL_DATA_SIZE_BYTES));
        assert_that!(MAX_ECALL_INPUT_SIZE_IN_BYTES, eq(64 * 1024));
    }

    #[rstest]
    #[case(0)]
    #[case(1)]
    #[case(MAX_ECALL_INPUT_SIZE_IN_BYTES - 1)]
    #[case(MAX_ECALL_INPUT_SIZE_IN_BYTES)]
    fn input_sizes_up_to_the_limit_are_accepted(#[case] input_size_in_bytes: u32) {
        assert_that!(
            validate_ecall_input_size(input_size_in_bytes).is_ok(),
            eq(true)
        );
    }

    /// Regression guard: the ecall input size is read from a
    /// guest register and so is fully attacker-controlled. Anything above the
    /// limit must be rejected with a typed error *before* the host sizes a
    /// buffer from it, instead of attempting a multi-gigabyte allocation.
    #[rstest]
    #[case(MAX_ECALL_INPUT_SIZE_IN_BYTES + 1)]
    #[case(16 * 1024 * 1024)]
    #[case(u32::MAX)]
    fn oversized_input_sizes_are_rejected(#[case] input_size_in_bytes: u32) {
        let error = validate_ecall_input_size(input_size_in_bytes)
            .expect_err("oversized ecall input size must be rejected");
        match error {
            UserCodeRunnerError::EcallInputTooLarge {
                input_size_in_bytes: reported_size,
                max_input_size_in_bytes,
            } => {
                assert_that!(reported_size, eq(input_size_in_bytes));
                assert_that!(max_input_size_in_bytes, eq(MAX_ECALL_INPUT_SIZE_IN_BYTES));
            }
            other => panic!("expected EcallInputTooLarge, got {other:?}"),
        }
    }
}
