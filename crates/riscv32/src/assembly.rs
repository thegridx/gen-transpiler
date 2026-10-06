//! RISC-V textual assembly representation.
//! Based on the RISC-V spec, available at <https://riscv.org/specifications>
use crate::common::SignType;
use crate::immediate::Immediate;
use crate::ops::{
    ArithmeticOp, AtomicFunct5, BranchFunct3, LoadSize, MulDivRemOp, RegArithmeticOp, RiscvOp,
    RvAtomic, RvBranch, RvFence, RvLoad, RvMulDivRem, RvOpImm, RvOpReg, RvStore, StoreSize,
};

impl RiscvOp {
    /// Returns the RISC-V assembly mnemonic and operands for this instruction.
    pub fn as_asm_string(&self) -> String {
        match self {
            Self::OpImm(inst) => inst.as_asm_string(),
            Self::OpReg(inst) => inst.as_asm_string(),
            Self::Lui(inst) => {
                // Assembler expects unsigned 20-bit page number
                let page = ((inst.imm().value() as u32) >> 12) & 0xF_FFFF;
                format!("lui {}, {page}", inst.rd().name())
            }
            Self::Auipc(inst) => {
                let page = ((inst.imm().value() as u32) >> 12) & 0xF_FFFF;
                format!("auipc {}, {page}", inst.rd().name())
            }
            Self::MulDivRem(inst) => inst.as_asm_string(),
            Self::Jal(inst) => {
                // JAL encodes a PC-relative offset; emit as .+offset
                format!("jal {}, .+{}", inst.rd().name(), inst.imm().0)
            }
            Self::Jalr(inst) => {
                format!(
                    "jalr {}, {}({})",
                    inst.rd().name(),
                    inst.imm().0,
                    inst.rs1().name()
                )
            }
            Self::Branch(inst) => inst.as_asm_string(),
            Self::Load(inst) => inst.as_asm_string(),
            Self::Store(inst) => inst.as_asm_string(),
            Self::Ecall => "ecall".to_string(),
            Self::Atomic(inst) => inst.as_asm_string(),
            Self::Fence(inst) => inst.as_asm_string(),
        }
    }
}

impl RvOpImm {
    fn as_asm_string(&self) -> String {
        use crate::ops::{BasicOp, ShiftOp};
        let (mnemonic, imm_str) = match &self.op() {
            ArithmeticOp::Basic(BasicOp::Add) => ("addi", format!("{}", self.imm().value())),
            ArithmeticOp::Basic(BasicOp::And) => ("andi", format!("{}", self.imm().value())),
            ArithmeticOp::Basic(BasicOp::Or) => ("ori", format!("{}", self.imm().value())),
            ArithmeticOp::Basic(BasicOp::Xor) => ("xori", format!("{}", self.imm().value())),
            ArithmeticOp::Shift(ShiftOp::Sll) => ("slli", format!("{}", self.imm().value() & 0x1F)),
            ArithmeticOp::Shift(ShiftOp::Srl) => ("srli", format!("{}", self.imm().value() & 0x1F)),
            ArithmeticOp::Shift(ShiftOp::Sra) => ("srai", format!("{}", self.imm().value() & 0x1F)),
            ArithmeticOp::SetLessThan(SignType::Signed) => {
                ("slti", format!("{}", self.imm().value()))
            }
            ArithmeticOp::SetLessThan(SignType::Unsigned) => {
                ("sltiu", format!("{}", self.imm().value()))
            }
        };
        format!(
            "{mnemonic} {}, {}, {imm_str}",
            self.rd().name(),
            self.rs1().name()
        )
    }
}

impl RvOpReg {
    fn as_asm_string(&self) -> String {
        use crate::ops::{BasicOp, ShiftOp};
        let mnemonic = match &self.op() {
            RegArithmeticOp::Sub => "sub",
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::Add)) => "add",
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::And)) => "and",
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::Or)) => "or",
            RegArithmeticOp::NonSub(ArithmeticOp::Basic(BasicOp::Xor)) => "xor",
            RegArithmeticOp::NonSub(ArithmeticOp::Shift(ShiftOp::Sll)) => "sll",
            RegArithmeticOp::NonSub(ArithmeticOp::Shift(ShiftOp::Srl)) => "srl",
            RegArithmeticOp::NonSub(ArithmeticOp::Shift(ShiftOp::Sra)) => "sra",
            RegArithmeticOp::NonSub(ArithmeticOp::SetLessThan(SignType::Signed)) => "slt",
            RegArithmeticOp::NonSub(ArithmeticOp::SetLessThan(SignType::Unsigned)) => "sltu",
        };
        format!(
            "{mnemonic} {}, {}, {}",
            self.rd().name(),
            self.rs1().name(),
            self.rs2().name()
        )
    }
}

impl RvMulDivRem {
    fn as_asm_string(&self) -> String {
        use crate::ops::{DivRemOp, MulHighOp};
        let mnemonic = match &self.op() {
            MulDivRemOp::MulLow => "mul",
            MulDivRemOp::MulHigh(MulHighOp::Mulh) => "mulh",
            MulDivRemOp::MulHigh(MulHighOp::Mulhu) => "mulhu",
            MulDivRemOp::MulHigh(MulHighOp::Mulhsu) => "mulhsu",
            MulDivRemOp::DivRem(DivRemOp::Div(SignType::Signed)) => "div",
            MulDivRemOp::DivRem(DivRemOp::Div(SignType::Unsigned)) => "divu",
            MulDivRemOp::DivRem(DivRemOp::Rem(SignType::Signed)) => "rem",
            MulDivRemOp::DivRem(DivRemOp::Rem(SignType::Unsigned)) => "remu",
        };
        format!(
            "{mnemonic} {}, {}, {}",
            self.rd().name(),
            self.rs1().name(),
            self.rs2().name()
        )
    }
}

impl RvBranch {
    fn as_asm_string(&self) -> String {
        let mnemonic = match self.condition() {
            BranchFunct3::Beq => "beq",
            BranchFunct3::Bne => "bne",
            BranchFunct3::Blt => "blt",
            BranchFunct3::Bge => "bge",
            BranchFunct3::Bltu => "bltu",
            BranchFunct3::Bgeu => "bgeu",
        };
        format!(
            "{mnemonic} {}, {}, .+{}",
            self.rs1().name(),
            self.rs2().name(),
            self.imm().0
        )
    }
}

impl RvLoad {
    fn as_asm_string(&self) -> String {
        let mnemonic = match self.size() {
            LoadSize::OneByte(SignType::Signed) => "lb",
            LoadSize::OneByte(SignType::Unsigned) => "lbu",
            LoadSize::HalfWord(SignType::Signed) => "lh",
            LoadSize::HalfWord(SignType::Unsigned) => "lhu",
            LoadSize::OneWord => "lw",
        };
        format!(
            "{mnemonic} {}, {}({})",
            self.rd().name(),
            self.imm().0,
            self.rs1().name()
        )
    }
}

impl RvStore {
    fn as_asm_string(&self) -> String {
        let mnemonic = match self.size() {
            StoreSize::OneByte => "sb",
            StoreSize::HalfWord => "sh",
            StoreSize::OneWord => "sw",
        };
        format!(
            "{mnemonic} {}, {}({})",
            self.rs2().name(),
            self.imm().0,
            self.rs1().name()
        )
    }
}

impl RvFence {
    fn as_asm_string(&self) -> String {
        if self.fm() == 0b1000 {
            return "fence.tso".to_string();
        }
        let set = |bits: u8| -> String {
            [(0b1000, 'i'), (0b0100, 'o'), (0b0010, 'r'), (0b0001, 'w')]
                .iter()
                .filter(|(bit, _)| bits & bit != 0)
                .map(|(_, c)| c)
                .collect()
        };
        format!("fence {},{}", set(self.pred()), set(self.succ()))
    }
}

impl RvAtomic {
    fn as_asm_string(&self) -> String {
        let (base_mnemonic, has_rs2) = match self.op() {
            AtomicFunct5::AmoAdd => ("amoadd.w", true),
            AtomicFunct5::AmoSwap => ("amoswap.w", true),
            AtomicFunct5::Lr => ("lr.w", false),
            AtomicFunct5::Sc => ("sc.w", true),
            AtomicFunct5::AmoXor => ("amoxor.w", true),
            AtomicFunct5::AmoOr => ("amoor.w", true),
            AtomicFunct5::AmoAnd => ("amoand.w", true),
            AtomicFunct5::AmoMin => ("amomin.w", true),
            AtomicFunct5::AmoMax => ("amomax.w", true),
            AtomicFunct5::AmoMinU => ("amominu.w", true),
            AtomicFunct5::AmoMaxU => ("amomaxu.w", true),
        };
        let ordering = match (self.aq(), self.rl()) {
            (false, false) => "",
            (true, false) => ".aq",
            (false, true) => ".rl",
            (true, true) => ".aqrl",
        };
        let mnemonic = format!("{base_mnemonic}{ordering}");
        if has_rs2 {
            format!(
                "{mnemonic} {}, {}, ({})",
                self.rd().name(),
                self.rs2().name(),
                self.rs1().name()
            )
        } else {
            format!("{mnemonic} {}, ({})", self.rd().name(), self.rs1().name())
        }
    }
}
