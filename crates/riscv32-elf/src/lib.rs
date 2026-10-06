//! ELF reading and validation utilities for RISC-V 32-bit binaries.
//!
//! This crate provides the ELF parsing logic used by the transpiler: format
//! validation, init-array extraction, section and symbol extraction, and the
//! register-store layout shared with transpiled code.
pub mod consts;
pub mod data_layout;
pub mod reader;
pub mod types;
