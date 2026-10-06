//! RISC-V 32-bit abstractions.
pub mod assembly;
pub mod blocks;
pub mod bytecode;
pub mod common;
pub mod gas_cost;
pub mod immediate;
pub mod ops;
pub mod registers;

// I mean, it has to be defined *somewhere*.
pub const N_BYTES_IN_WORD: usize = 4;
