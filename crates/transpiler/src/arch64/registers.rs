//! aarch64 registers and the mapping of RISC-V guest registers onto them.
//!
//! Guest x0 maps to `WZR`, `ra` to x30, and 26 more guest registers to host registers. `t3` to
//! `t6` do not fit and live on the host stack. The backend reserves x17 (`REG_TMP`) and x28
//! (`REG_TMP_2`) as scratch, x19 (`REG_GAS`) for gas, and never touches x18, the platform register.
use riscv32::registers::{RiscvRegister, RvPreservedRegister};
use std::fmt;
use strum::{EnumIter, FromRepr};

/// An aarch64 general-purpose register, plus `SP`, `ZR` and `PC`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, EnumIter, FromRepr)]
pub(crate) enum ArmRegister {
    /// Function argument and return value
    X0 = 0,
    /// Function argument
    X1 = 1,
    /// Function argument
    X2 = 2,
    /// Function argument
    X3 = 3,
    /// Function argument
    X4 = 4,
    /// Function argument
    X5 = 5,
    /// Function argument
    X6 = 6,
    /// Function argument
    X7 = 7,
    /// Indirect function call or function argument
    X8 = 8,
    /// Scratch register
    X9 = 9,
    /// Scratch register
    X10 = 10,
    /// Scratch register
    X11 = 11,
    /// Scratch register
    X12 = 12,
    /// Scratch register
    X13 = 13,
    /// Scratch register
    X14 = 14,
    /// Scratch register
    X15 = 15,
    /// Intra-procedure scratch register
    Ip0 = 16,
    /// Intra-procedure scratch register
    Ip1 = 17,
    /// Platform register, reserved by some host ABIs. Never used.
    X18 = 18,
    /// Non volatile general purpose register
    X19 = 19,
    /// Non volatile general purpose register
    X20 = 20,
    /// Non volatile general purpose register
    X21 = 21,
    /// Non volatile general purpose register
    X22 = 22,
    /// Non volatile general purpose register
    X23 = 23,
    /// Non volatile general purpose register
    X24 = 24,
    /// Non volatile general purpose register
    X25 = 25,
    /// Non volatile general purpose register
    X26 = 26,
    /// Non volatile general purpose register
    X27 = 27,
    /// Non volatile general purpose register
    X28 = 28,
    /// Frame Pointer
    Fp = 29,
    /// Link Register (where return address is typically stored)
    Lr = 30,
    /// Stack pointer; encoded as register 31 where an instruction reads it as `SP`.
    Sp,
    /// Zero register; encoded as register 31 where an instruction reads it as `ZR`.
    Zr,
    /// Program counter. Only for display; no instruction here encodes it.
    Pc,
}

impl fmt::Display for ArmRegister {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArmRegister::Zr => write!(f, "wzr"),
            ArmRegister::Sp => {
                write!(f, "<error: SP cannot be displayed as 32-bit register>")?;
                Err(fmt::Error)
            }
            ArmRegister::Pc => {
                write!(f, "<error: PC cannot be displayed as 32-bit register>")?;
                Err(fmt::Error)
            }
            _ => write!(f, "w{}", *self as usize),
        }
    }
}

/// A guest register kept on the host stack, at `[sp, #(4 * index)]`, while guest code runs.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, EnumIter, FromRepr)]
pub(crate) enum ArmMemoryMappedRegister {
    T3 = 0,
    T4 = 1,
    T5 = 2,
    T6 = 3,
}

/// Where a guest register lives on aarch64: in a host register or on the host stack.
pub(crate) enum ArmMappedRegister {
    Register(ArmRegister),
    Memory(ArmMemoryMappedRegister),
}

// x17 (`REG_TMP`), x18 (platform), x19 (`REG_GAS`) and x28 (`REG_TMP_2`) hold no guest
// register.
impl From<RiscvRegister> for ArmMappedRegister {
    fn from(reg: RiscvRegister) -> Self {
        match reg {
            RiscvRegister::Zero => ArmMappedRegister::Register(ArmRegister::Zr),
            RiscvRegister::Ra => ArmMappedRegister::Register(ArmRegister::Lr),
            RiscvRegister::Sp => ArmMappedRegister::Register(ArmRegister::X25),
            RiscvRegister::Gp => ArmMappedRegister::Register(ArmRegister::X26),
            RiscvRegister::Tp => ArmMappedRegister::Register(ArmRegister::X27),
            RiscvRegister::T0 => ArmMappedRegister::Register(ArmRegister::X9),
            RiscvRegister::T1 => ArmMappedRegister::Register(ArmRegister::X10),
            RiscvRegister::T2 => ArmMappedRegister::Register(ArmRegister::X11),
            RiscvRegister::S0 => ArmMappedRegister::Register(ArmRegister::Fp),
            RiscvRegister::S1 => ArmMappedRegister::Register(ArmRegister::X24),
            RiscvRegister::A0 => ArmMappedRegister::Register(ArmRegister::X0),
            RiscvRegister::A1 => ArmMappedRegister::Register(ArmRegister::X1),
            RiscvRegister::A2 => ArmMappedRegister::Register(ArmRegister::X2),
            RiscvRegister::A3 => ArmMappedRegister::Register(ArmRegister::X3),
            RiscvRegister::A4 => ArmMappedRegister::Register(ArmRegister::X4),
            RiscvRegister::A5 => ArmMappedRegister::Register(ArmRegister::X5),
            RiscvRegister::A6 => ArmMappedRegister::Register(ArmRegister::X6),
            RiscvRegister::A7 => ArmMappedRegister::Register(ArmRegister::X7),
            RiscvRegister::S2 => ArmMappedRegister::Register(ArmRegister::X20),
            RiscvRegister::S3 => ArmMappedRegister::Register(ArmRegister::X21),
            RiscvRegister::S4 => ArmMappedRegister::Register(ArmRegister::X22),
            RiscvRegister::S5 => ArmMappedRegister::Register(ArmRegister::X23),
            RiscvRegister::S6 => ArmMappedRegister::Register(ArmRegister::Ip0),
            RiscvRegister::S7 => ArmMappedRegister::Register(ArmRegister::X8),
            RiscvRegister::S8 => ArmMappedRegister::Register(ArmRegister::X12),
            RiscvRegister::S9 => ArmMappedRegister::Register(ArmRegister::X13),
            RiscvRegister::S10 => ArmMappedRegister::Register(ArmRegister::X14),
            RiscvRegister::S11 => ArmMappedRegister::Register(ArmRegister::X15),
            RiscvRegister::T3 => ArmMappedRegister::Memory(ArmMemoryMappedRegister::T3),
            RiscvRegister::T4 => ArmMappedRegister::Memory(ArmMemoryMappedRegister::T4),
            RiscvRegister::T5 => ArmMappedRegister::Memory(ArmMemoryMappedRegister::T5),
            RiscvRegister::T6 => ArmMappedRegister::Memory(ArmMemoryMappedRegister::T6),
        }
    }
}

impl ArmRegister {
    /// Returns the 64-bit name of the register. Only the assembly-text test oracle uses it.
    #[cfg(test)]
    pub(crate) fn x(&self) -> String {
        match self {
            ArmRegister::Zr => "xzr".to_string(),
            ArmRegister::Sp => "sp".to_string(),
            ArmRegister::Pc => "pc".to_string(),
            _ => format!("x{}", *self as usize),
        }
    }

    /// Returns the 5-bit register number used in instruction encodings. `Sp` and `Zr` both
    /// encode as 31; the instruction decides which one 31 means.
    pub(crate) const fn encoding(&self) -> u32 {
        match self {
            ArmRegister::Sp | ArmRegister::Zr => 31,
            ArmRegister::Pc => 31, // Never encoded; 31 keeps the function total.
            _ => *self as u32,
        }
    }
}

/// Holds the remaining gas as a signed 64-bit value; gas runs out when it goes negative.
pub(crate) const REG_GAS: ArmRegister = ArmRegister::X19;
/// Scratch register (x17, `IP1`), also the base register of most memory ops.
pub(crate) const REG_TMP: ArmRegister = ArmRegister::Ip1;
/// Second scratch register (x28). It also carries the guest return address across an ecall.
pub(crate) const REG_TMP_2: ArmRegister = ArmRegister::X28;

/// Callee-saved host registers, pushed in pairs on entry and popped before returning to the host.
pub(crate) const ARM_GLOBAL_PRESERVED_REGISTER_PAIRS: [(ArmRegister, ArmRegister); 6] = [
    (ArmRegister::X19, ArmRegister::X20),
    (ArmRegister::X21, ArmRegister::X22),
    (ArmRegister::X23, ArmRegister::X24),
    (ArmRegister::X25, ArmRegister::X26),
    (ArmRegister::X27, ArmRegister::X28),
    (ArmRegister::Fp, ArmRegister::Lr),
];

/// The stack-held guest registers, paired so the envelope can move them with `LDP`/`STP`.
pub(crate) const RISC_MEMORY_MAPPED_REGISTER_PAIRS: [(RvPreservedRegister, RvPreservedRegister);
    2] = [
    (RvPreservedRegister::T3, RvPreservedRegister::T4),
    (RvPreservedRegister::T5, RvPreservedRegister::T6),
];

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    // ── ArmRegister::encoding ─────────────────────────────────────────────────

    #[test]
    fn arm_register_encoding_numbered_registers_match_their_repr() {
        // X0..X28, Fp, Lr should encode as their discriminant value
        for reg in ArmRegister::iter() {
            let enc = reg.encoding();
            match reg {
                ArmRegister::Sp | ArmRegister::Zr | ArmRegister::Pc => {
                    assert_eq!(enc, 31, "{reg:?} should encode as 31");
                }
                _ => {
                    assert_eq!(enc, reg as u32, "{reg:?} should encode as its repr value");
                }
            }
        }
    }

    #[test]
    fn arm_register_encoding_all_in_valid_range() {
        for reg in ArmRegister::iter() {
            assert!(reg.encoding() <= 31, "{reg:?} encoding exceeds 31");
        }
    }

    #[test]
    fn arm_register_encoding_numbered_registers_are_below_31() {
        // Only Sp/Zr/Pc encode as 31; all others must be strictly less
        for reg in ArmRegister::iter() {
            match reg {
                ArmRegister::Sp | ArmRegister::Zr | ArmRegister::Pc => {}
                _ => assert!(reg.encoding() < 31, "{reg:?} encoding should be < 31"),
            }
        }
    }

    // ── ArmRegister::x() ─────────────────────────────────────────────────────

    #[test]
    fn arm_register_x_formats_64bit_names() {
        assert_eq!(ArmRegister::X0.x(), "x0");
        assert_eq!(ArmRegister::X9.x(), "x9");
        assert_eq!(ArmRegister::X19.x(), "x19");
        assert_eq!(ArmRegister::Fp.x(), "x29");
        assert_eq!(ArmRegister::Lr.x(), "x30");
    }

    #[test]
    fn arm_register_x_special_registers_have_named_forms() {
        assert_eq!(ArmRegister::Sp.x(), "sp");
        assert_eq!(ArmRegister::Zr.x(), "xzr");
        assert_eq!(ArmRegister::Pc.x(), "pc");
    }

    // ── ArmRegister Display (32-bit w-register names) ─────────────────────────

    #[test]
    fn arm_register_display_formats_as_w_registers() {
        assert_eq!(format!("{}", ArmRegister::X0), "w0");
        assert_eq!(format!("{}", ArmRegister::X19), "w19");
        assert_eq!(format!("{}", ArmRegister::Fp), "w29");
        assert_eq!(format!("{}", ArmRegister::Lr), "w30");
        assert_eq!(format!("{}", ArmRegister::Zr), "wzr");
    }

    // ── Special-purpose register constants ───────────────────────────────────

    #[test]
    fn special_register_constants_are_correct() {
        assert_eq!(REG_GAS, ArmRegister::X19);
        assert_eq!(REG_TMP, ArmRegister::Ip1);
        assert_eq!(REG_TMP_2, ArmRegister::X28);
    }

    #[test]
    fn special_registers_are_distinct_from_each_other() {
        assert_ne!(REG_GAS, REG_TMP);
        assert_ne!(REG_GAS, REG_TMP_2);
        assert_ne!(REG_TMP, REG_TMP_2);
    }
}
