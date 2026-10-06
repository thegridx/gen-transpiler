//! Loads a transpiled native ELF into memory and runs it.
//!
//! [`TranspiledVm::load`] validates the ELF and copies its segments into fresh memory, and a
//! [`TranspiledCodeRunner`] drives the loaded program through the `user_code_runner` traits.
pub use code_runner::TranspiledCodeRunner;
pub use error::LoaderError;
pub use loaded_layout::LoadedLayout;
pub use loader::LoaderContext;
pub(crate) use loader::SymbolHandle;
pub use transpiled_vm::TranspiledVm;

pub(crate) mod code_runner;
pub(crate) mod error;
pub(crate) mod loaded_layout;
pub(crate) mod loader;
pub(crate) mod transpiled_vm;
