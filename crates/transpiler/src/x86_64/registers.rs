//! x86_64 register model: hardware encodings, 8/32/64-bit register views, and the mapping of
//! RISC-V registers to host registers or `.register_store` slots.
use riscv32::registers::{RiscvRegister, RvPreservedRegister};
use std::fmt::{self};
use strum::{EnumIter, FromRepr};

/// The x86_64 general-purpose registers, numbered by their hardware encoding (0-15):
/// rax=0, rcx=1, rdx=2, rbx=3, rsp=4, rbp=5, rsi=6, rdi=7, r8=8 … r15=15.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, EnumIter, FromRepr)]
pub(crate) enum X86RegisterIndex {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum X86Bitness {
    Byte,
    /// 16-bit operand size; only the assembly-text renderer (the encoder-test oracle) uses it.
    #[cfg(test)]
    Word,
    Dword,
    Qword,
}
impl fmt::Display for X86Bitness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            X86Bitness::Byte => "byte",
            #[cfg(test)]
            X86Bitness::Word => "word",
            X86Bitness::Dword => "dword",
            X86Bitness::Qword => "qword",
        };
        write!(f, "{s} ptr")
    }
}

// A register index together with an operand size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct X86Register {
    index: X86RegisterIndex,
    bitness: X86Bitness,
}
impl X86Register {
    const fn new(index: X86RegisterIndex, bits: X86Bitness) -> Self {
        Self {
            index,
            bitness: bits,
        }
    }
    // Without a REX prefix, the byte registers of these encode as dh, bh and ch instead of sil,
    // dil and bpl, so 8-bit accesses to them need an empty REX prefix.
    pub(crate) const fn is_esi_edi_ebp(&self) -> bool {
        matches!(
            self,
            X86Register { index, .. } if matches!(index, X86RegisterIndex::Rsi | X86RegisterIndex::Rdi | X86RegisterIndex::Rbp)
        )
    }
    // True for r8-r15, which need a REX prefix bit to encode.
    pub(crate) const fn is_rex(&self) -> bool {
        matches!(
            self,
            X86Register { index, .. } if matches!(index, X86RegisterIndex::R8 | X86RegisterIndex::R9 | X86RegisterIndex::R10 | X86RegisterIndex::R11 | X86RegisterIndex::R12 | X86RegisterIndex::R13 | X86RegisterIndex::R14 | X86RegisterIndex::R15)
        )
    }
    pub(crate) const fn hw_enc(&self) -> u8 {
        self.index as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct X86Reg8Bit(X86Register);
impl X86Reg8Bit {
    const fn new(reg_index: X86RegisterIndex) -> Self {
        Self(X86Register::new(reg_index, X86Bitness::Byte))
    }
    pub(crate) const fn register(&self) -> X86Register {
        self.0
    }
}
impl fmt::Display for X86Reg8Bit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self.0.index {
                X86RegisterIndex::Rax => "al",
                X86RegisterIndex::Rbx => "bl",
                X86RegisterIndex::Rcx => "cl",
                X86RegisterIndex::Rdx => "dl",
                X86RegisterIndex::Rsi => "sil",
                X86RegisterIndex::Rdi => "dil",
                X86RegisterIndex::Rbp => "bpl",
                X86RegisterIndex::Rsp => unreachable!("rsp has no 8-bit version"),
                X86RegisterIndex::R8 => "r8b",
                X86RegisterIndex::R9 => "r9b",
                X86RegisterIndex::R10 => "r10b",
                X86RegisterIndex::R11 => "r11b",
                X86RegisterIndex::R12 => "r12b",
                X86RegisterIndex::R13 => "r13b",
                X86RegisterIndex::R14 => "r14b",
                X86RegisterIndex::R15 => "r15b",
            }
        )
    }
}

// The 16-bit register view is used only by the assembly-text renderer (the encoder-test oracle).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct X86Reg16Bit(X86Register);
#[cfg(test)]
impl X86Reg16Bit {
    const fn new(reg_index: X86RegisterIndex) -> Self {
        Self(X86Register::new(reg_index, X86Bitness::Word))
    }
}
#[cfg(test)]
impl fmt::Display for X86Reg16Bit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self.0.index {
                X86RegisterIndex::Rax => "ax",
                X86RegisterIndex::Rbx => "bx",
                X86RegisterIndex::Rcx => "cx",
                X86RegisterIndex::Rdx => "dx",
                X86RegisterIndex::Rsi => "si",
                X86RegisterIndex::Rdi => "di",
                X86RegisterIndex::Rbp => "bp",
                X86RegisterIndex::Rsp => unreachable!("rsp has no 16-bit version"),
                X86RegisterIndex::R8 => "r8w",
                X86RegisterIndex::R9 => "r9w",
                X86RegisterIndex::R10 => "r10w",
                X86RegisterIndex::R11 => "r11w",
                X86RegisterIndex::R12 => "r12w",
                X86RegisterIndex::R13 => "r13w",
                X86RegisterIndex::R14 => "r14w",
                X86RegisterIndex::R15 => "r15w",
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct X86Reg32Bit(X86Register);
impl X86Reg32Bit {
    const fn new(reg_index: X86RegisterIndex) -> Self {
        Self(X86Register::new(reg_index, X86Bitness::Dword))
    }
    pub(crate) const fn register(&self) -> X86Register {
        self.0
    }
    // The 32-bit view is the one guest registers use; these return the other views of the same
    // register.
    pub(crate) const fn b(&self) -> X86Reg8Bit {
        X86Reg8Bit::new(self.0.index)
    }
    #[cfg(test)]
    pub(crate) const fn w(&self) -> X86Reg16Bit {
        X86Reg16Bit::new(self.0.index)
    }
}
impl fmt::Display for X86Reg32Bit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self.0.index {
                X86RegisterIndex::Rax => "eax",
                X86RegisterIndex::Rbx => "ebx",
                X86RegisterIndex::Rcx => "ecx",
                X86RegisterIndex::Rdx => "edx",
                X86RegisterIndex::Rsi => "esi",
                X86RegisterIndex::Rdi => "edi",
                X86RegisterIndex::Rbp => "ebp",
                X86RegisterIndex::Rsp => unreachable!("rsp has no 32-bit version"),
                X86RegisterIndex::R8 => "r8d",
                X86RegisterIndex::R9 => "r9d",
                X86RegisterIndex::R10 => "r10d",
                X86RegisterIndex::R11 => "r11d",
                X86RegisterIndex::R12 => "r12d",
                X86RegisterIndex::R13 => "r13d",
                X86RegisterIndex::R14 => "r14d",
                X86RegisterIndex::R15 => "r15d",
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct X86Reg64Bit(X86Register);
impl X86Reg64Bit {
    const fn new(reg_index: X86RegisterIndex) -> Self {
        Self(X86Register::new(reg_index, X86Bitness::Qword))
    }
    pub(crate) const fn register(&self) -> X86Register {
        self.0
    }
    pub(crate) const fn hw_enc(&self) -> u8 {
        self.0.hw_enc()
    }
}
impl fmt::Display for X86Reg64Bit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self.0.index {
                X86RegisterIndex::Rax => "rax",
                X86RegisterIndex::Rbx => "rbx",
                X86RegisterIndex::Rcx => "rcx",
                X86RegisterIndex::Rdx => "rdx",
                X86RegisterIndex::Rsi => "rsi",
                X86RegisterIndex::Rdi => "rdi",
                X86RegisterIndex::Rbp => "rbp",
                X86RegisterIndex::Rsp => "rsp",
                X86RegisterIndex::R8 => "r8",
                X86RegisterIndex::R9 => "r9",
                X86RegisterIndex::R10 => "r10",
                X86RegisterIndex::R11 => "r11",
                X86RegisterIndex::R12 => "r12",
                X86RegisterIndex::R13 => "r13",
                X86RegisterIndex::R14 => "r14",
                X86RegisterIndex::R15 => "r15",
            }
        )
    }
}

// A 32-bit or 64-bit register operand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum X86Reg32Or64 {
    Reg32(X86Reg32Bit),
    Reg64(X86Reg64Bit),
}
impl X86Reg32Or64 {
    pub(crate) const fn is_rex(&self) -> bool {
        match self {
            X86Reg32Or64::Reg32(r) => r.0.is_rex(),
            X86Reg32Or64::Reg64(r) => r.0.is_rex(),
        }
    }
    pub(crate) const fn is_64_bit(&self) -> bool {
        matches!(self, X86Reg32Or64::Reg64(_))
    }
}
impl fmt::Display for X86Reg32Or64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            X86Reg32Or64::Reg32(r) => write!(f, "{r}"),
            X86Reg32Or64::Reg64(r) => write!(f, "{r}"),
        }
    }
}
impl From<X86Reg32Bit> for X86Reg32Or64 {
    fn from(r: X86Reg32Bit) -> Self {
        X86Reg32Or64::Reg32(r)
    }
}
impl From<X86Reg64Bit> for X86Reg32Or64 {
    fn from(r: X86Reg64Bit) -> Self {
        X86Reg32Or64::Reg64(r)
    }
}

// Named views of each register.
pub(crate) const REG_EAX: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::Rax);
pub(crate) const REG_RAX: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::Rax);
pub(crate) const REG_EBX: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::Rbx);
pub(crate) const REG_RBX: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::Rbx);
pub(crate) const REG_CL: X86Reg8Bit = X86Reg8Bit::new(X86RegisterIndex::Rcx);
pub(crate) const REG_ECX: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::Rcx);
pub(crate) const REG_RCX: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::Rcx);
pub(crate) const REG_EDX: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::Rdx);
pub(crate) const REG_RDX: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::Rdx);
pub(crate) const REG_ESI: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::Rsi);
pub(crate) const REG_RSI: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::Rsi);
pub(crate) const REG_EDI: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::Rdi);
#[cfg(test)]
pub(crate) const REG_RDI: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::Rdi);
pub(crate) const REG_EBP: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::Rbp);
pub(crate) const REG_RBP: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::Rbp);
pub(crate) const REG_R8D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R8);
#[cfg(test)]
pub(crate) const REG_R8: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R8);
pub(crate) const REG_R9D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R9);
#[cfg(test)]
pub(crate) const REG_R9: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R9);
pub(crate) const REG_R10D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R10);
#[cfg(test)]
pub(crate) const REG_R10: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R10);
pub(crate) const REG_R11D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R11);
#[cfg(test)]
pub(crate) const REG_R11: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R11);
pub(crate) const REG_R12D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R12);
pub(crate) const REG_R12: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R12);
pub(crate) const REG_R13D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R13);
pub(crate) const REG_R13: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R13);
pub(crate) const REG_R14D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R14);
pub(crate) const REG_R14: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R14);
pub(crate) const REG_R15D: X86Reg32Bit = X86Reg32Bit::new(X86RegisterIndex::R15);
pub(crate) const REG_R15: X86Reg64Bit = X86Reg64Bit::new(X86RegisterIndex::R15);

// The host's callee-saved registers (except rsp), which the envelope pushes on entry and pops
// before returning.
pub(crate) const X86_CALLEE_PRESERVED_REGISTERS: [X86Reg64Bit; 6] =
    [REG_RBX, REG_RBP, REG_R12, REG_R13, REG_R14, REG_R15];

// Where a RISC-V register lives on x86_64.
#[derive(Debug, Clone, Copy)]
pub(crate) enum X86MappedRegister {
    Register(X86Reg32Bit),
    MemoryMapped(RvPreservedRegister),
    Zero,
}
impl From<RiscvRegister> for X86MappedRegister {
    fn from(r: RiscvRegister) -> Self {
        // x0 has no host register. ra, sp, t0, s0 and a0-a7 live in host registers; the others
        // live in `.register_store` and are accessed in memory. Of the remaining host
        // registers, eax and ebx are the temporaries (`X86TempReg`), rsi holds gas
        // (`X86_REG_GAS`), and rsp stays the host stack pointer.
        match r {
            RiscvRegister::Zero => X86MappedRegister::Zero,
            RiscvRegister::Ra => X86MappedRegister::Register(REG_EDI),
            RiscvRegister::Sp => X86MappedRegister::Register(REG_EBP),
            RiscvRegister::Gp => X86MappedRegister::MemoryMapped(RvPreservedRegister::Gp),
            RiscvRegister::Tp => X86MappedRegister::MemoryMapped(RvPreservedRegister::Tp),
            RiscvRegister::T0 => X86MappedRegister::Register(REG_R14D),
            RiscvRegister::T1 => X86MappedRegister::MemoryMapped(RvPreservedRegister::T1),
            RiscvRegister::T2 => X86MappedRegister::MemoryMapped(RvPreservedRegister::T2),
            RiscvRegister::S0 => X86MappedRegister::Register(REG_R15D),
            RiscvRegister::S1 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S1),
            RiscvRegister::A0 => X86MappedRegister::Register(REG_ECX),
            RiscvRegister::A1 => X86MappedRegister::Register(REG_EDX),
            RiscvRegister::A2 => X86MappedRegister::Register(REG_R8D),
            RiscvRegister::A3 => X86MappedRegister::Register(REG_R9D),
            RiscvRegister::A4 => X86MappedRegister::Register(REG_R10D),
            RiscvRegister::A5 => X86MappedRegister::Register(REG_R11D),
            RiscvRegister::A6 => X86MappedRegister::Register(REG_R12D),
            RiscvRegister::A7 => X86MappedRegister::Register(REG_R13D),
            RiscvRegister::S2 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S2),
            RiscvRegister::S3 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S3),
            RiscvRegister::S4 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S4),
            RiscvRegister::S5 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S5),
            RiscvRegister::S6 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S6),
            RiscvRegister::S7 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S7),
            RiscvRegister::S8 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S8),
            RiscvRegister::S9 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S9),
            RiscvRegister::S10 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S10),
            RiscvRegister::S11 => X86MappedRegister::MemoryMapped(RvPreservedRegister::S11),
            RiscvRegister::T3 => X86MappedRegister::MemoryMapped(RvPreservedRegister::T3),
            RiscvRegister::T4 => X86MappedRegister::MemoryMapped(RvPreservedRegister::T4),
            RiscvRegister::T5 => X86MappedRegister::MemoryMapped(RvPreservedRegister::T5),
            RiscvRegister::T6 => X86MappedRegister::MemoryMapped(RvPreservedRegister::T6),
        }
    }
}

// Whether `risc_reg` lives in the host register `x86_reg`.
pub(crate) fn is_x86_reg(risc_reg: RiscvRegister, x86_reg: X86Reg32Bit) -> bool {
    matches!(X86MappedRegister::from(risc_reg), X86MappedRegister::Register(r) if r == x86_reg)
}

// The two scratch registers (eax and ebx). Instruction lowerings may clobber them freely.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum X86TempReg {
    Tmp1,
    Tmp2,
}
impl X86TempReg {
    pub(crate) const fn d(&self) -> X86Reg32Bit {
        match self {
            X86TempReg::Tmp1 => REG_EAX,
            X86TempReg::Tmp2 => REG_EBX,
        }
    }
    pub(crate) const fn q(&self) -> X86Reg64Bit {
        match self {
            X86TempReg::Tmp1 => REG_RAX,
            X86TempReg::Tmp2 => REG_RBX,
        }
    }
    pub(crate) const fn hw_enc(&self) -> u8 {
        self.d().0.hw_enc()
    }
}

// Names for the two temporaries.
pub(crate) const X86_REG_TMP_EAX: X86TempReg = X86TempReg::Tmp1;
pub(crate) const X86_REG_TMP_EBX: X86TempReg = X86TempReg::Tmp2;
/// Holds the remaining gas as a signed 64-bit value; a negative value means out of gas.
pub(crate) const X86_REG_GAS: X86Reg64Bit = REG_RSI;
/// The host register that holds guest `ra`.
pub(crate) const X86_RISC_RA_REG: X86Reg32Bit = REG_EDI;
/// The host register that holds guest a7, which selects the entry point or ecall return on
/// entry and carries the exit code on exit.
pub(crate) const X86_RISC_A7_REG: X86Reg32Bit = REG_R13D;
