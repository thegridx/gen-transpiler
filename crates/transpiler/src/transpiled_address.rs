//! Addresses in the output, relative to the start of `.text`, and their resolution during the
//! branch passes.
use riscv32::common::{ValueError, checked_add_u32, checked_sub_u32};
use riscv32_elf::types::RiscvAddress;

use crate::{
    TranspilerError,
    layout::{
        GlobalCodeLabel, GlobalLabel, GlobalLabelAddrMap, JumpTableEntry, LocalCodeLabel,
        SymbolicBranchTarget,
    },
};

/// An address in the output, as an offset from the start of `.text`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TranspiledAddress(u32);

impl TranspiledAddress {
    pub(crate) fn new(addr: u32) -> Self {
        Self(addr)
    }
    pub(crate) fn as_u32(&self) -> u32 {
        self.0
    }
    pub(crate) fn as_i32(&self) -> i32 {
        self.0 as i32
    }
    pub(crate) fn as_u64(&self) -> u64 {
        self.0 as u64
    }
    pub(crate) fn as_usize(&self) -> usize {
        self.0 as usize
    }
    pub(crate) fn checked_add(&self, v: u32) -> Result<Self, ValueError> {
        checked_add_u32(self.0, v, "TranspiledAddress::checked_add").map(Self)
    }
    pub(crate) fn checked_sub(&self, v: u32) -> Result<Self, ValueError> {
        checked_sub_u32(self.0, v, "TranspiledAddress::checked_sub").map(Self)
    }
    // Wrapping arithmetic, only for values meant modulo 2^32, such as PC-relative offsets.
    pub(crate) fn wrapping_add(&self, v: u32) -> Self {
        Self(self.0.wrapping_add(v))
    }
    pub(crate) fn wrapping_sub(&self, v: u32) -> Self {
        Self(self.0.wrapping_sub(v))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TranspiledAddressAndSize {
    pub(crate) addr: TranspiledAddress,
    pub(crate) size: u32,
}

pub(crate) enum AddressResolveOutcome {
    Resolved(TranspiledAddress),
    NotApplicable,
}

/// Resolves branch targets to addresses during the resolution passes.
pub(crate) struct TranspiledAddressResolver<'a> {
    pub(crate) global_addresses: &'a GlobalLabelAddrMap,
    pub(crate) jump_table_entries: &'a [JumpTableEntry],
}

impl TranspiledAddressResolver<'_> {
    pub(crate) fn new<'a>(
        global_addresses: &'a GlobalLabelAddrMap,
        jump_table_entries: &'a [JumpTableEntry],
    ) -> TranspiledAddressResolver<'a> {
        TranspiledAddressResolver {
            global_addresses,
            jump_table_entries,
        }
    }
}

impl TranspiledAddressResolver<'_> {
    pub(crate) fn resolve_label(
        &self,
        label: &GlobalLabel,
    ) -> Result<TranspiledAddress, TranspilerError> {
        self.global_addresses
            .get(label)
            .cloned()
            .ok_or_else(|| TranspilerError::missing_global_label(*label))
    }

    /// Resolves the target of a direct branch to a RISC-V address.
    pub(crate) fn resolve_risc_loc(
        &self,
        addr: RiscvAddress,
    ) -> Result<TranspiledAddress, TranspilerError> {
        let index = addr.index() as usize;
        match self.jump_table_entries.get(index) {
            // The jump table maps each original RISC-V instruction to its transpiled address
            Some(entry) => Ok(entry.transpiled_addr()),
            // The end of the code. Reaching it by a direct branch is like falling off the last
            // instruction: it runs into the illegal-instruction exit, which the envelope places
            // right after the user code. (An indirect jump there, such as the final `ret`, goes
            // through the jump table's exit sentinel instead.)
            None if index == self.jump_table_entries.len() => {
                self.resolve_label(&GlobalCodeLabel::ExitIllegalOpcode.into())
            }
            None => Err(TranspilerError::MissingRiscvAddress(addr)),
        }
    }

    /// Resolves `target` to an address. A local-label target resolves only if it is
    /// `local_label`, to `local_label_addr`, which the calling pass tracks; any other local
    /// label is `NotApplicable`, left for the other pass.
    pub(crate) fn resolve(
        &self,
        target: &SymbolicBranchTarget,
        local_label: LocalCodeLabel,
        local_label_addr: &Option<TranspiledAddress>,
    ) -> Result<AddressResolveOutcome, TranspilerError> {
        match target {
            SymbolicBranchTarget::Label(label) => Ok(AddressResolveOutcome::Resolved(
                self.resolve_label(&(*label).into())?,
            )),
            SymbolicBranchTarget::RiscLoc(addr) => Ok(AddressResolveOutcome::Resolved(
                self.resolve_risc_loc(*addr)?,
            )),
            SymbolicBranchTarget::Local(ll) => {
                if ll == &local_label {
                    Ok(AddressResolveOutcome::Resolved(
                        (*local_label_addr)
                            .ok_or_else(|| TranspilerError::local_label_not_found(local_label))?,
                    ))
                } else {
                    Ok(AddressResolveOutcome::NotApplicable)
                }
            }
        }
    }
}
