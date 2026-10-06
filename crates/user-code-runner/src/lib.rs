//! Generic run loop for executing transpiled RISC-V user code.
//!
//! This crate provides the core abstractions and orchestration logic for running
//! RISC-V user code: it sits between the transpiled native code and the host
//! that loads it, managing the execution loop and dispatching system calls
//! (ecalls) from user programs to a host-supplied handler.
//!
//! # Architecture
//!
//! The execution model follows a cooperative loop where:
//! 1. Transpiled user code runs natively until it issues an ecall or returns
//! 2. Control transfers back to the runtime
//! 3. The runtime dispatches the ecall to appropriate handlers
//! 4. Execution resumes with the next instruction
//!
//! # Main Types
//!
//! - [`CodeRunner`]: Trait for executing transpiled code until an ecall or return.
//!   The `transpiler` crate implements it as `transpiler::execute::TranspiledCodeRunner`.
//!
//! - [`EcallHandler`]: Trait for processing system calls from user code. Implementers
//!   handle specific ecall codes, manage user code exit/panic scenarios, and determine
//!   when execution should halt.
//!
//! - [`RegistersInOut`]: A snapshot of the RISC-V register file (w0-w7) used for
//!   passing arguments to/from user code and ecall handlers.
//!
//! - [`UserCodeRunnerError`]: Errors that can occur during user code execution,
//!   including unsupported ecalls, missing symbols, and internal panics.
//!
//! [`CodeRunner`]: runner::CodeRunner
//! [`EcallHandler`]: runner::EcallHandler
//! [`RegistersInOut`]: runner::RegistersInOut
//! [`UserCodeRunnerError`]: runner::UserCodeRunnerError

/// The gas amount type.
pub mod gas;
/// The traits a host implements and the functions that run user code.
pub mod runner;
