//! RISC-V registers.
use strum::{EnumIter, FromRepr};

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, EnumIter, Clone, Copy, FromRepr)]
pub enum RiscvRegister {
    Zero = 0, // hard wired 0
    Ra = 1,   // return address
    Sp = 2,   // stack pointer
    Gp = 3,   // global pointer
    Tp = 4,   // thread pointer
    T0 = 5,   // temporaries
    T1 = 6,
    T2 = 7,
    S0 = 8,  // saved register/stack pointer
    S1 = 9,  // saved register
    A0 = 10, // function arguments/return values
    A1 = 11,
    A2 = 12, // function arguments
    A3 = 13,
    A4 = 14,
    A5 = 15,
    A6 = 16,
    A7 = 17,
    S2 = 18, // saved registers
    S3 = 19,
    S4 = 20,
    S5 = 21,
    S6 = 22,
    S7 = 23,
    S8 = 24,
    S9 = 25,
    S10 = 26,
    S11 = 27,
    T3 = 28, // temporaries
    T4 = 29,
    T5 = 30,
    T6 = 31,
}

impl RiscvRegister {
    pub fn name(&self) -> &'static str {
        match self {
            RiscvRegister::Zero => "zero",
            RiscvRegister::Ra => "ra",
            RiscvRegister::Sp => "sp",
            RiscvRegister::Gp => "gp",
            RiscvRegister::Tp => "tp",
            RiscvRegister::T0 => "t0",
            RiscvRegister::T1 => "t1",
            RiscvRegister::T2 => "t2",
            RiscvRegister::S0 => "s0",
            RiscvRegister::S1 => "s1",
            RiscvRegister::A0 => "a0",
            RiscvRegister::A1 => "a1",
            RiscvRegister::A2 => "a2",
            RiscvRegister::A3 => "a3",
            RiscvRegister::A4 => "a4",
            RiscvRegister::A5 => "a5",
            RiscvRegister::A6 => "a6",
            RiscvRegister::A7 => "a7",
            RiscvRegister::S2 => "s2",
            RiscvRegister::S3 => "s3",
            RiscvRegister::S4 => "s4",
            RiscvRegister::S5 => "s5",
            RiscvRegister::S6 => "s6",
            RiscvRegister::S7 => "s7",
            RiscvRegister::S8 => "s8",
            RiscvRegister::S9 => "s9",
            RiscvRegister::S10 => "s10",
            RiscvRegister::S11 => "s11",
            RiscvRegister::T3 => "t3",
            RiscvRegister::T4 => "t4",
            RiscvRegister::T5 => "t5",
            RiscvRegister::T6 => "t6",
        }
    }
}

/// The guest registers the register store keeps between runs, in their order there: every
/// register except x0 and a0 to a7, which pass through `RegistersInOut` instead.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumIter, FromRepr)]
pub enum RvPreservedRegister {
    Ra = 0,
    Sp,
    Gp,
    Tp,
    T0,
    T1,
    T2,
    S0,
    S1,
    S2,
    S3,
    S4,
    S5,
    S6,
    S7,
    S8,
    S9,
    S10,
    S11,
    T3,
    T4,
    T5,
    T6,
}

impl RvPreservedRegister {
    pub const COUNT: u32 = (Self::T6 as u32) + 1;
}

impl From<RvPreservedRegister> for RiscvRegister {
    fn from(preserved: RvPreservedRegister) -> Self {
        match preserved {
            RvPreservedRegister::Ra => RiscvRegister::Ra,
            RvPreservedRegister::Sp => RiscvRegister::Sp,
            RvPreservedRegister::Gp => RiscvRegister::Gp,
            RvPreservedRegister::Tp => RiscvRegister::Tp,
            RvPreservedRegister::T0 => RiscvRegister::T0,
            RvPreservedRegister::T1 => RiscvRegister::T1,
            RvPreservedRegister::T2 => RiscvRegister::T2,
            RvPreservedRegister::S0 => RiscvRegister::S0,
            RvPreservedRegister::S1 => RiscvRegister::S1,
            RvPreservedRegister::S2 => RiscvRegister::S2,
            RvPreservedRegister::S3 => RiscvRegister::S3,
            RvPreservedRegister::S4 => RiscvRegister::S4,
            RvPreservedRegister::S5 => RiscvRegister::S5,
            RvPreservedRegister::S6 => RiscvRegister::S6,
            RvPreservedRegister::S7 => RiscvRegister::S7,
            RvPreservedRegister::S8 => RiscvRegister::S8,
            RvPreservedRegister::S9 => RiscvRegister::S9,
            RvPreservedRegister::S10 => RiscvRegister::S10,
            RvPreservedRegister::S11 => RiscvRegister::S11,
            RvPreservedRegister::T3 => RiscvRegister::T3,
            RvPreservedRegister::T4 => RiscvRegister::T4,
            RvPreservedRegister::T5 => RiscvRegister::T5,
            RvPreservedRegister::T6 => RiscvRegister::T6,
        }
    }
}
