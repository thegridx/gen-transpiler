//! RISC-V encoding to bytecode bytes.
//! Based on the RISC-V spec, available at <https://riscv.org/specifications>
use strum::FromRepr;

use crate::common::SignType;
use crate::immediate::Immediate;
use crate::ops::{
    ArithmeticOp, AtomicFunct5, BranchFunct3, LoadSize, LoadStoreFunct3, MulDivRemOp, OpFunct3,
    OpMulDivFunct3, RegArithmeticOp, RegisterAndImmediate, RiscvOp, RvAtomic, RvAuipc, RvBranch,
    RvDecodeError, RvFence, RvImmB13, RvImmI12, RvImmJ21, RvImmS12, RvImmU20s12, RvJal, RvJalr,
    RvLoad, RvLui, RvMulDivRem, RvOpImm, RvOpReg, RvStore, ShiftOp, StoreSize, ThreeRegisters,
    TwoRegistersAndImmediate,
};
use crate::registers::RiscvRegister;

// ── Immediate extraction (decode) ──────────────────────────────────────────

pub(crate) fn inst_to_imm_j(inst: i32) -> i32 {
    ((((inst >> 20) as u32) & 0xFF_F0_07_FE) as i32) + (inst & 0xF_F0_00) + ((inst >> 9) & 0x8_00)
}
pub(crate) fn inst_to_imm_b(inst: i32) -> i32 {
    ((((inst >> 20) as u32) & 0xFF_FF_F7_E0) as i32) + ((inst << 4) & 0x8_00) + ((inst >> 7) & 0x1E)
}
pub(crate) fn inst_to_imm_s(inst: i32) -> i32 {
    ((((inst >> 20) as u32) & 0xFF_FF_FF_E0) + (((inst >> 7) as u32) & 0x1F)) as i32
}
pub(crate) fn inst_to_imm_i(inst: i32) -> i16 {
    (inst >> 20) as i16
}
pub(crate) fn inst_to_imm_u(inst: i32) -> i32 {
    const BITS_12_TO_31_MASK: u32 = 0xFFFF_F000;
    (inst as u32 & BITS_12_TO_31_MASK) as i32
}

pub(crate) const ATOMIC_FUNCT_3_CODE: i32 = 2;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, FromRepr)]
pub(crate) enum Rv32ImaOpCode {
    // integer register immediate (see 2.4.1)
    OpImm = 0b0_01_00_11,
    Lui = 0b0_11_01_11,
    Auipc = 0b0_01_01_11,
    // integer register register (see 2.4.2)
    Op = 0b0_11_00_11,
    // unconditional jumps (2.5.1)
    Jal = 0b1_10_11_11,  // Jump and link
    Jalr = 0b1_10_01_11, // Jump and link register
    // conditional branches (2.5.2)
    Branch = 0b1_10_00_11,
    // load store (2.6)
    Load = 0b0_00_00_11,
    Store = 0b0_10_00_11,
    // environment call (2.8)
    System = 0b1_11_00_11,
    // atomic (14)
    Atomic = 0b0_10_11_11,
    // memory ordering (2.7)
    MiscMem = 0b0_00_11_11,
}

/// funct3 of `FENCE` within the MISC-MEM opcode. `FENCE.I` (funct3 = 1) belongs to the
/// Zifencei extension, which is not supported.
pub(crate) const FENCE_FUNCT_3_CODE: i32 = 0;

// ── Encode helpers ─────────────────────────────────────────────────────────

fn encode_rd(reg: RiscvRegister) -> i32 {
    (reg as u8 as i32) << 7
}
fn encode_rs1(reg: RiscvRegister) -> i32 {
    (reg as u8 as i32) << 15
}
fn encode_rs2(reg: RiscvRegister) -> i32 {
    (reg as u8 as i32) << 20
}
/// Encode a 12-bit I-type immediate into instruction bits [31:20].
fn encode_imm_i(imm: i16) -> i32 {
    (imm as i32) << 20
}
/// Encode a 12-bit S-type immediate into instruction bits [31:25] and [11:7].
fn encode_imm_s(imm: i32) -> i32 {
    let bits_4_0 = imm & 0x1F;
    let bits_11_5 = (imm >> 5) & 0x7F;
    (bits_4_0 << 7) | (bits_11_5 << 25)
}
/// Encode a 13-bit B-type immediate into instruction bits.
fn encode_imm_b(imm: i32) -> i32 {
    let bit_11 = (imm >> 11) & 1;
    let bits_4_1 = (imm >> 1) & 0xF;
    let bits_10_5 = (imm >> 5) & 0x3F;
    let bit_12 = (imm >> 12) & 1;
    (bit_11 << 7) | (bits_4_1 << 8) | (bits_10_5 << 25) | (bit_12 << 31)
}
/// Encode a 21-bit J-type immediate into instruction bits [31:12].
fn encode_imm_j(imm: i32) -> i32 {
    let bits_19_12 = (imm >> 12) & 0xFF;
    let bit_11 = (imm >> 11) & 1;
    let bits_10_1 = (imm >> 1) & 0x3FF;
    let bit_20 = (imm >> 20) & 1;
    (bits_19_12 << 12) | (bit_11 << 20) | (bits_10_1 << 21) | (bit_20 << 31)
}

// ── Decode helpers ─────────────────────────────────────────────────────────

const LOW_3_BITS_MASK: i32 = 7;
const LOW_4_BITS_MASK: i32 = 0xF;
const LOW_5_BITS_MASK: i32 = 0x1F;
const LOW_7_BITS_MASK: i32 = 0x7F;
const BIT_5_MASK: i32 = 0x20;

fn decode_registers(inst: i32) -> Result<[RiscvRegister; 3], RvDecodeError> {
    let mut regs = [RiscvRegister::Zero; 3];
    for (i, shift) in [7, 15, 20].iter().enumerate() {
        let idx = ((inst >> shift) & LOW_5_BITS_MASK) as u8;
        regs[i] = RiscvRegister::from_repr(idx)
            .ok_or(RvDecodeError::InvalidRegister { reg: idx as u32 })?;
    }
    Ok(regs)
}

// ── Decode / Encode implementations ─────────────────────────────────────────

impl RiscvOp {
    /// Decode a raw 32-bit instruction word into a strongly-typed [`RiscvOp`].
    ///
    /// Returns [`RvDecodeError`] for invalid encodings (unknown opcodes, bad funct3, etc.).
    /// This performs pure instruction decoding without context-dependent validation
    /// (e.g. target address range checks, block boundary tracking).
    pub fn decode(inst: i32) -> Result<Self, RvDecodeError> {
        if inst & 0b11 != 0b11 {
            return Err(RvDecodeError::CompressedInstruction);
        }
        let opcode = Rv32ImaOpCode::from_repr(inst & LOW_7_BITS_MASK).ok_or({
            RvDecodeError::InvalidOpcode {
                opcode: inst & LOW_7_BITS_MASK,
            }
        })?;
        let funct3 = (inst >> 12) & LOW_3_BITS_MASK;
        let [rd, rs1, rs2] = decode_registers(inst)?;
        let funct7 = (inst >> 25) & LOW_7_BITS_MASK;
        let funct7_5th_bit = (funct7 & BIT_5_MASK) == BIT_5_MASK;

        match opcode {
            Rv32ImaOpCode::OpImm => {
                let parsed_f3 =
                    OpFunct3::from_repr(funct3).ok_or(RvDecodeError::InvalidFunct3 { funct3 })?;
                // Shift immediates carry a funct7: 0000000 for SLLI and SRLI, 0100000 for SRAI.
                // Other values are reserved encodings.
                let funct7_valid = match parsed_f3 {
                    OpFunct3::Sll => funct7 == 0x00,
                    OpFunct3::SrlSra => funct7 == 0x00 || funct7 == 0x20,
                    _ => true,
                };
                if !funct7_valid {
                    return Err(RvDecodeError::ReservedFunct7Bits { funct7 });
                }
                let imm = inst_to_imm_i(inst);
                let imm_i = RvImmI12::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                let op = ArithmeticOp::new(parsed_f3, funct7_5th_bit);
                Ok(Self::OpImm(RvOpImm::new(
                    op,
                    TwoRegistersAndImmediate::new(rs1, rd, imm_i),
                )))
            }
            Rv32ImaOpCode::Lui => {
                let imm = inst_to_imm_u(inst);
                let imm_u = RvImmU20s12::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                Ok(Self::Lui(RvLui::new(RegisterAndImmediate::new(rd, imm_u))))
            }
            Rv32ImaOpCode::Auipc => {
                let imm = inst_to_imm_u(inst);
                let imm_u = RvImmU20s12::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                Ok(Self::Auipc(RvAuipc::new(RegisterAndImmediate::new(
                    rd, imm_u,
                ))))
            }
            Rv32ImaOpCode::Op => {
                // funct7 must be exactly 0000001 (M extension), 0000000, or 0100000 (SUB/SRA).
                match funct7 {
                    0x01 => {
                        // RV32M extension
                        let parsed_f3 = OpMulDivFunct3::from_repr(funct3)
                            .ok_or(RvDecodeError::InvalidFunct3 { funct3 })?;
                        let op = MulDivRemOp::from(parsed_f3);
                        Ok(Self::MulDivRem(RvMulDivRem::new(
                            op,
                            ThreeRegisters::new(rs1, rs2, rd),
                        )))
                    }
                    0x00 | 0x20 => {
                        let parsed_f3 = OpFunct3::from_repr(funct3)
                            .ok_or(RvDecodeError::InvalidFunct3 { funct3 })?;
                        // 0100000 selects SUB and SRA only; with any other funct3 it is a
                        // reserved encoding (used by extensions such as Zbb's `andn`).
                        if funct7 == 0x20
                            && !matches!(parsed_f3, OpFunct3::AddSub | OpFunct3::SrlSra)
                        {
                            return Err(RvDecodeError::ReservedFunct7Bits { funct7 });
                        }
                        let op = RegArithmeticOp::new(parsed_f3, funct7_5th_bit);
                        Ok(Self::OpReg(RvOpReg::new(
                            op,
                            ThreeRegisters::new(rs1, rs2, rd),
                        )))
                    }
                    _ => Err(RvDecodeError::ReservedFunct7Bits { funct7 }),
                }
            }
            Rv32ImaOpCode::Jal => {
                let imm = inst_to_imm_j(inst);
                let imm_j = RvImmJ21::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                Ok(Self::Jal(RvJal::new(RegisterAndImmediate::new(rd, imm_j))))
            }
            Rv32ImaOpCode::Jalr => {
                if funct3 != 0 {
                    return Err(RvDecodeError::InvalidFunct3 { funct3 });
                }
                let imm = inst_to_imm_i(inst);
                let imm_i = RvImmI12::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                Ok(Self::Jalr(RvJalr::new(TwoRegistersAndImmediate::new(
                    rs1, rd, imm_i,
                ))))
            }
            Rv32ImaOpCode::Branch => {
                let condition = BranchFunct3::from_repr(funct3)
                    .ok_or(RvDecodeError::InvalidFunct3 { funct3 })?;
                let imm = inst_to_imm_b(inst);
                let imm_b = RvImmB13::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                Ok(Self::Branch(RvBranch::new(
                    condition,
                    TwoRegistersAndImmediate::new(rs1, rs2, imm_b),
                )))
            }
            Rv32ImaOpCode::Load => {
                let parsed_f3 = LoadStoreFunct3::from_repr(funct3)
                    .ok_or(RvDecodeError::InvalidFunct3 { funct3 })?;
                let size = match parsed_f3 {
                    LoadStoreFunct3::OneByte => LoadSize::OneByte(SignType::Signed),
                    LoadStoreFunct3::HalfWord => LoadSize::HalfWord(SignType::Signed),
                    LoadStoreFunct3::OneWord => LoadSize::OneWord,
                    LoadStoreFunct3::UnsignedOneByte => LoadSize::OneByte(SignType::Unsigned),
                    LoadStoreFunct3::UnsignedHalfWord => LoadSize::HalfWord(SignType::Unsigned),
                };
                let imm = inst_to_imm_i(inst);
                let imm_i = RvImmI12::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                Ok(Self::Load(RvLoad::new(
                    size,
                    TwoRegistersAndImmediate::new(rs1, rd, imm_i),
                )))
            }
            Rv32ImaOpCode::Store => {
                let parsed_f3 = LoadStoreFunct3::from_repr(funct3)
                    .ok_or(RvDecodeError::InvalidFunct3 { funct3 })?;
                let size = match parsed_f3 {
                    LoadStoreFunct3::OneByte => StoreSize::OneByte,
                    LoadStoreFunct3::HalfWord => StoreSize::HalfWord,
                    LoadStoreFunct3::OneWord => StoreSize::OneWord,
                    _ => return Err(RvDecodeError::SignedCodeInStore),
                };
                let imm = inst_to_imm_s(inst);
                let imm_s = RvImmS12::try_from(imm)
                    .map_err(|_| RvDecodeError::InvalidImmediate { imm: imm.into() })?;
                Ok(Self::Store(RvStore::new(
                    size,
                    TwoRegistersAndImmediate::new(rs1, rs2, imm_s),
                )))
            }
            Rv32ImaOpCode::System => {
                let is_ecall = funct3 == 0
                    && rd == RiscvRegister::Zero
                    && rs1 == RiscvRegister::Zero
                    && (inst >> 20) == 0;
                if !is_ecall {
                    return Err(RvDecodeError::UnsupportedSystemCall);
                }
                Ok(Self::Ecall)
            }
            Rv32ImaOpCode::Atomic => {
                if funct3 != ATOMIC_FUNCT_3_CODE {
                    return Err(RvDecodeError::InvalidAtomicWidth { funct3 });
                }
                let aq = (inst >> 26) & 1 != 0;
                let rl = (inst >> 25) & 1 != 0;
                let funct5 = (inst >> 27) & LOW_5_BITS_MASK;
                let parsed_f5 = AtomicFunct5::from_repr(funct5)
                    .ok_or(RvDecodeError::InvalidAtomicFunct5 { funct5 })?;
                // `LR.W` has no source operand: its rs2 field is reserved and must be 0.
                if parsed_f5 == AtomicFunct5::Lr && rs2 != RiscvRegister::Zero {
                    return Err(RvDecodeError::ReservedLrSource);
                }
                Ok(Self::Atomic(
                    RvAtomic::new(parsed_f5, ThreeRegisters::new(rs1, rs2, rd))
                        .with_ordering(aq, rl),
                ))
            }
            Rv32ImaOpCode::MiscMem => {
                if funct3 != FENCE_FUNCT_3_CODE {
                    return Err(RvDecodeError::InvalidFunct3 { funct3 });
                }
                let field = |shift: i32| ((inst >> shift) & LOW_4_BITS_MASK) as u8;
                Ok(Self::Fence(RvFence::new(
                    field(28),
                    field(24),
                    field(20),
                    rs1,
                    rd,
                )))
            }
        }
    }

    /// Encode this instruction back into a raw 32-bit instruction word.
    pub fn encode(&self) -> i32 {
        match self {
            Self::OpImm(inst) => inst.encode(),
            Self::OpReg(inst) => inst.encode(),
            Self::Lui(inst) => inst.encode(),
            Self::Auipc(inst) => inst.encode(),
            Self::MulDivRem(inst) => inst.encode(),
            Self::Jal(inst) => inst.encode(),
            Self::Jalr(inst) => inst.encode(),
            Self::Branch(inst) => inst.encode(),
            Self::Load(inst) => inst.encode(),
            Self::Store(inst) => inst.encode(),
            Self::Ecall => Rv32ImaOpCode::System as i32,
            Self::Atomic(inst) => inst.encode(),
            Self::Fence(inst) => inst.encode(),
        }
    }
}

// ── Per-instruction encode methods ──────────────────────────────────────────

impl RvOpImm {
    pub fn encode(&self) -> i32 {
        // For shift ops, the upper immediate bits encode the operation variant
        // (e.g. bit 10 = SRA vs SRL), so derive them from `op` rather than
        // trusting the raw immediate payload.
        let imm = match self.op() {
            ArithmeticOp::Shift(ShiftOp::Sra) => (self.imm().value() & 0x1F) | 0x400,
            ArithmeticOp::Shift(_) => self.imm().value() & 0x1F,
            _ => self.imm().value(),
        };
        Rv32ImaOpCode::OpImm as i32
            | encode_rd(self.rd())
            | ((self.op().funct3() as i32) << 12)
            | encode_rs1(self.rs1())
            | encode_imm_i(imm)
    }
}

impl RvOpReg {
    pub fn encode(&self) -> i32 {
        let funct7 = if self.op().funct7_bit5() { 0x20 } else { 0 };
        Rv32ImaOpCode::Op as i32
            | encode_rd(self.rd())
            | ((self.op().funct3() as i32) << 12)
            | encode_rs1(self.rs1())
            | encode_rs2(self.rs2())
            | (funct7 << 25)
    }
}

impl RvLui {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::Lui as i32 | encode_rd(self.rd()) | (self.imm().value() & !0xFFF)
    }
}

impl RvAuipc {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::Auipc as i32 | encode_rd(self.rd()) | (self.imm().value() & !0xFFF)
    }
}

impl RvMulDivRem {
    pub fn encode(&self) -> i32 {
        // funct7 bit 0 = 1 for M extension
        Rv32ImaOpCode::Op as i32
            | encode_rd(self.rd())
            | ((self.op().funct3() as i32) << 12)
            | encode_rs1(self.rs1())
            | encode_rs2(self.rs2())
            | (1 << 25)
    }
}

impl RvJal {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::Jal as i32 | encode_rd(self.rd()) | encode_imm_j(self.imm().value())
    }
}

impl RvJalr {
    pub fn encode(&self) -> i32 {
        // funct3 = 0 for JALR
        Rv32ImaOpCode::Jalr as i32
            | encode_rd(self.rd())
            | encode_rs1(self.rs1())
            | encode_imm_i(self.imm().value())
    }
}

impl RvBranch {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::Branch as i32
            | ((self.condition() as i32) << 12)
            | encode_rs1(self.rs1())
            | encode_rs2(self.rs2())
            | encode_imm_b(self.imm().value())
    }
}

impl RvLoad {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::Load as i32
            | encode_rd(self.rd())
            | ((self.size().funct3() as i32) << 12)
            | encode_rs1(self.rs1())
            | encode_imm_i(self.imm().value())
    }
}

impl RvStore {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::Store as i32
            | ((self.size().funct3() as i32) << 12)
            | encode_rs1(self.rs1())
            | encode_rs2(self.rs2())
            | encode_imm_s(self.imm().value())
    }
}

impl RvAtomic {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::Atomic as i32
            | encode_rd(self.rd())
            | (ATOMIC_FUNCT_3_CODE << 12)
            | encode_rs1(self.rs1())
            | encode_rs2(self.rs2())
            | (i32::from(self.rl()) << 25)
            | (i32::from(self.aq()) << 26)
            | ((self.op() as i32) << 27)
    }
}

impl RvFence {
    pub fn encode(&self) -> i32 {
        Rv32ImaOpCode::MiscMem as i32
            | encode_rd(self.rd())
            | (FENCE_FUNCT_3_CODE << 12)
            | encode_rs1(self.rs1())
            | (i32::from(self.succ()) << 20)
            | (i32::from(self.pred()) << 24)
            | (i32::from(self.fm()) << 28)
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "File operations are required for the assembler comparison tests."
)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{BasicOp, BranchCondition, DivRemOp, MulHighOp};
    const TEXT_SECTION_NAME: &str = ".text";
    use rand::{Rng, SeedableRng, rngs::StdRng};
    use strum::IntoEnumIterator;

    /// A random generator for `test`, seeded so that a failure can be replayed: `TEST_SEED`
    /// (decimal or `0x`-prefixed hex) if set, else a fixed seed. The seed is printed.
    fn seeded_rng(test: &str) -> StdRng {
        let seed = std::env::var("TEST_SEED").map_or(0x5EED_5EED, |value| {
            match value.strip_prefix("0x") {
                Some(hex) => u64::from_str_radix(hex, 16).ok(),
                None => value.parse().ok(),
            }
            .unwrap_or_else(|| panic!("TEST_SEED={value:?} is not a u64"))
        });
        println!("{test}: seed {seed:#x} (rerun with TEST_SEED={seed:#x})");
        StdRng::seed_from_u64(seed)
    }

    /// Verify that decoding then re-encoding produces the original instruction word.
    fn roundtrip(inst: i32) {
        let decoded = RiscvOp::decode(inst).unwrap_or_else(|e| panic!("decode 0x{inst:08x}: {e}"));
        let re_encoded = decoded.encode();
        assert_eq!(
            re_encoded, inst,
            "roundtrip failed for 0x{inst:08x}: decoded as {decoded:?}, re-encoded as 0x{re_encoded:08x}"
        );
    }

    #[test]
    fn roundtrip_addi() {
        let inst = 0x06410093; // ADDI x1, x2, 100
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_add() {
        let inst = 0x002081B3; // ADD x3, x1, x2
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_sub() {
        let inst = 0x402081B3; // SUB x3, x1, x2
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_lui() {
        let inst = 0x1234_52B7; // LUI x5, 0x12345
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_jal() {
        let inst = 0x100000EF; // JAL x1, offset=256
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_beq() {
        let inst = 0x00208463; // BEQ x1, x2, 8
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_lw() {
        let inst = 0x00452283; // LW x5, 4(x10)
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_sw() {
        let inst = 0x00552223; // SW x5, 4(x10)
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_ecall() {
        let inst = 0x00000073; // ECALL
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_srai() {
        let inst = 0x40355293; // SRAI x5, x10, 3
        roundtrip(inst);
    }

    #[test]
    fn roundtrip_mul() {
        let inst = 0x022081B3; // MUL x3, x1, x2
        roundtrip(inst);
    }

    #[test]
    fn decode_compressed_instruction_errors() {
        let result = RiscvOp::decode(0x0001);
        assert!(
            matches!(result, Err(RvDecodeError::CompressedInstruction)),
            "expected CompressedInstruction, got {result:?}"
        );
    }

    #[test]
    fn decode_invalid_opcode_errors() {
        let result = RiscvOp::decode(0b0000111);
        assert!(
            matches!(result, Err(RvDecodeError::InvalidOpcode { .. })),
            "expected InvalidOpcode, got {result:?}"
        );
    }

    #[test]
    fn roundtrip_negative_immediate() {
        let inst: i32 = 0xFFF00093_u32 as i32; // ADDI x1, x0, -1
        roundtrip(inst);
    }

    // ── Assembler-matching test ────────────────────────────────────────────

    use std::io::Write;
    use std::process::Command;

    /// Assembles multiple RISC-V instructions and returns 4-byte instruction words (little-endian).
    fn assemble_instructions(instructions: &[String]) -> Vec<[u8; 4]> {
        let temp_dir = tempfile::tempdir().expect("Failed to create a temporary directory");
        let asm_file = temp_dir.path().join("riscv32_test.s");
        let obj_file = temp_dir.path().join("riscv32_test.o");
        let bin_file = temp_dir.path().join("riscv32_test.bin");

        let mut file = std::fs::File::create(&asm_file).expect("Failed to create asm file");
        writeln!(file, "{TEXT_SECTION_NAME}").expect("Failed to write .text");
        for instr in instructions {
            writeln!(file, "{instr}").expect("Failed to write instruction");
        }
        drop(file);

        // Use riscv64-unknown-elf-as with -march=rv32ima to assemble as 32-bit
        let as_output = Command::new("riscv64-unknown-elf-as")
            .args(["-march=rv32ima", "-mabi=ilp32"])
            .arg("-o")
            .arg(&obj_file)
            .arg(&asm_file)
            .output()
            .expect("Failed to run riscv64-unknown-elf-as. Is it installed?");

        if !as_output.status.success() {
            let stderr = String::from_utf8_lossy(&as_output.stderr);
            panic!("Assembly failed: {stderr}");
        }

        let objcopy_output = Command::new("riscv64-unknown-elf-objcopy")
            .args(["-O", "binary", "-j", TEXT_SECTION_NAME])
            .arg(&obj_file)
            .arg(&bin_file)
            .output()
            .expect("Failed to run riscv64-unknown-elf-objcopy");

        if !objcopy_output.status.success() {
            panic!(
                "objcopy failed: {}",
                String::from_utf8_lossy(&objcopy_output.stderr)
            );
        }

        let bytes = std::fs::read(&bin_file).expect("Failed to read binary");

        // Convert to 4-byte chunks (little-endian)
        bytes.as_chunks::<4>().0.to_vec()
    }

    fn random_register(rng: &mut impl Rng) -> RiscvRegister {
        let idx: u8 = rng.random_range(0..32);
        RiscvRegister::from_repr(idx).unwrap_or_else(|| unreachable!())
    }

    fn random_signed_i16(rng: &mut impl Rng, bits: u32) -> i16 {
        let half = 1i16 << (bits - 1);
        rng.random_range(-half..half)
    }

    fn random_signed_i32(rng: &mut impl Rng, bits: u32, trailing_zeros: u32) -> i32 {
        let half = 1i32 << (bits - trailing_zeros - 1);
        let shifted_val = rng.random_range(-half..half);
        shifted_val << trailing_zeros
    }

    /// Generates one random test case per instruction variant.
    fn generate_test_cases(rng: &mut impl Rng) -> Vec<RiscvOp> {
        let rd = random_register(rng);
        let rs1 = random_register(rng);
        let rs2 = random_register(rng);

        let imm_i = RvImmI12(random_signed_i16(rng, 12));
        let imm_s = RvImmS12(random_signed_i32(rng, 12, 0));
        // B-type: 13-bit with 1 trailing zero, but we also need bit 1 = 0
        // (no compressed inst address), so we need multiples of 4
        let imm_b = RvImmB13(random_signed_i32(rng, 13, 2));
        // J-type: 21-bit with 1 trailing zero, no bit-1 set
        let imm_j = RvImmJ21(random_signed_i32(rng, 21, 2));
        let imm_u: RvImmU20s12 = {
            let val = rng.random::<i32>() & !0xFFF_i32;
            RvImmU20s12::try_from(val).unwrap()
        };
        let shift_amount = RvImmI12(rng.random_range(0..32_i16));

        let r2i_params = TwoRegistersAndImmediate::new(rs1, rd, imm_i);
        let r2i_shift_params = TwoRegistersAndImmediate::new(rs1, rd, shift_amount);
        let r3_params = ThreeRegisters::new(rs1, rs2, rd);
        let r_u_params = RegisterAndImmediate::new(rd, imm_u);
        let r2s_params = TwoRegistersAndImmediate::new(rs1, rs2, imm_s);
        let r2b_params = TwoRegistersAndImmediate::new(rs1, rs2, imm_b);

        let mut ops = vec![];

        // OP-IMM: all basic + shift variants
        for basic_op in BasicOp::iter() {
            ops.push(RiscvOp::OpImm(RvOpImm::new(
                ArithmeticOp::Basic(basic_op),
                r2i_params.clone(),
            )));
        }
        for shift_op in ShiftOp::iter() {
            ops.push(RiscvOp::OpImm(RvOpImm::new(
                ArithmeticOp::Shift(shift_op),
                r2i_shift_params.clone(),
            )));
        }
        for sign in SignType::iter() {
            ops.push(RiscvOp::OpImm(RvOpImm::new(
                ArithmeticOp::SetLessThan(sign),
                r2i_params.clone(),
            )));
        }
        // OP: register-register
        for basic_op in BasicOp::iter() {
            ops.push(RiscvOp::OpReg(RvOpReg::new(
                RegArithmeticOp::NonSub(ArithmeticOp::Basic(basic_op)),
                r3_params.clone(),
            )));
        }
        ops.push(RiscvOp::OpReg(RvOpReg::new(
            RegArithmeticOp::Sub,
            r3_params.clone(),
        )));
        for shift_op in ShiftOp::iter() {
            ops.push(RiscvOp::OpReg(RvOpReg::new(
                RegArithmeticOp::NonSub(ArithmeticOp::Shift(shift_op)),
                r3_params.clone(),
            )));
        }
        for sign in SignType::iter() {
            ops.push(RiscvOp::OpReg(RvOpReg::new(
                RegArithmeticOp::NonSub(ArithmeticOp::SetLessThan(sign)),
                r3_params.clone(),
            )));
        }
        // LUI / AUIPC
        ops.push(RiscvOp::Lui(RvLui::new(r_u_params.clone())));
        ops.push(RiscvOp::Auipc(RvAuipc::new(r_u_params.clone())));
        // MulDivRem
        ops.push(RiscvOp::MulDivRem(RvMulDivRem::new(
            MulDivRemOp::MulLow,
            r3_params.clone(),
        )));
        for mul_high_op in MulHighOp::iter() {
            ops.push(RiscvOp::MulDivRem(RvMulDivRem::new(
                MulDivRemOp::MulHigh(mul_high_op),
                r3_params.clone(),
            )));
        }
        for sign in SignType::iter() {
            ops.push(RiscvOp::MulDivRem(RvMulDivRem::new(
                MulDivRemOp::DivRem(DivRemOp::Div(sign)),
                r3_params.clone(),
            )));
            ops.push(RiscvOp::MulDivRem(RvMulDivRem::new(
                MulDivRemOp::DivRem(DivRemOp::Rem(sign)),
                r3_params.clone(),
            )));
        }
        // JAL / JALR
        ops.push(RiscvOp::Jal(RvJal::new(RegisterAndImmediate::new(
            rd, imm_j,
        ))));
        ops.push(RiscvOp::Jalr(RvJalr::new(r2i_params.clone())));
        // Branches
        for condition in BranchCondition::iter() {
            ops.push(RiscvOp::Branch(RvBranch::new(
                condition,
                r2b_params.clone(),
            )));
        }
        // Loads
        for sign in SignType::iter() {
            ops.push(RiscvOp::Load(RvLoad::new(
                LoadSize::OneByte(sign),
                r2i_params.clone(),
            )));
            ops.push(RiscvOp::Load(RvLoad::new(
                LoadSize::HalfWord(sign),
                r2i_params.clone(),
            )));
        }
        ops.push(RiscvOp::Load(RvLoad::new(
            LoadSize::OneWord,
            r2i_params.clone(),
        )));

        ops.push(RiscvOp::Store(RvStore::new(
            StoreSize::OneByte,
            r2s_params.clone(),
        )));
        ops.push(RiscvOp::Store(RvStore::new(
            StoreSize::HalfWord,
            r2s_params.clone(),
        )));
        ops.push(RiscvOp::Store(RvStore::new(
            StoreSize::OneWord,
            r2s_params.clone(),
        )));
        // Ecall
        ops.push(RiscvOp::Ecall);
        // Atomics: iterate all funct5 variants
        for funct5 in AtomicFunct5::iter() {
            let rs2_atomic = if funct5 == AtomicFunct5::Lr {
                RiscvRegister::Zero // LR.W has rs2=0
            } else {
                rs2
            };
            ops.push(RiscvOp::Atomic(
                RvAtomic::new(funct5, ThreeRegisters::new(rs1, rs2_atomic, rd))
                    .with_ordering(rng.random(), rng.random()),
            ));
        }
        // Fences: any non-empty predecessor and successor sets, plus `fence.tso`
        ops.push(RiscvOp::Fence(RvFence::new(
            0,
            rng.random_range(1..=0xF),
            rng.random_range(1..=0xF),
            RiscvRegister::Zero,
            RiscvRegister::Zero,
        )));
        ops.push(RiscvOp::Fence(RvFence::new(
            0b1000,
            0b0011,
            0b0011,
            RiscvRegister::Zero,
            RiscvRegister::Zero,
        )));
        ops
    }

    /// Encodings that share an opcode with a supported instruction but set reserved bits, for
    /// example Zbb's `andn`, must be rejected rather than run as the base instruction.
    #[test]
    fn reserved_encodings_are_rejected() {
        let reserved: [(u32, &str); 8] = [
            (0x4031_70B3, "andn x1, x2, x3 (Zbb)"),
            (0x4031_60B3, "orn x1, x2, x3 (Zbb)"),
            (0x4031_40B3, "xnor x1, x2, x3 (Zbb)"),
            (0x4031_10B3, "sll with funct7 0100000"),
            (0x4031_20B3, "slt with funct7 0100000"),
            (0x4031_30B3, "sltu with funct7 0100000"),
            (0x4031_1093, "slli with funct7 0100000"),
            (0x1031_20AF, "lr.w x1, (x2) with rs2 = x3"),
        ];
        for (word, what) in reserved {
            assert!(
                RiscvOp::decode(word as i32).is_err(),
                "{what} ({word:#010x}) should be rejected"
            );
        }
        let valid: [(u32, &str); 4] = [
            (0x4031_00B3, "sub x1, x2, x3"),
            (0x4031_50B3, "sra x1, x2, x3"),
            (0x4031_5093, "srai x1, x2, 3"),
            (0x1001_20AF, "lr.w x1, (x2)"),
        ];
        for (word, what) in valid {
            assert!(
                RiscvOp::decode(word as i32).is_ok(),
                "{what} ({word:#010x}) should decode"
            );
        }
    }

    #[test]
    fn all_riscv_opcodes_match_assembler() {
        let mut rng = seeded_rng("all_riscv_opcodes_match_assembler");
        let test_cases: Vec<RiscvOp> = (0..500)
            .flat_map(|_| generate_test_cases(&mut rng))
            .collect();

        let mut asm_lines: Vec<String> = Vec::new();
        let mut our_words: Vec<[u8; 4]> = Vec::new();
        let mut descriptions: Vec<String> = Vec::new();

        for op in &test_cases {
            let encoded = op.encode();
            let bytes = encoded.to_le_bytes();
            asm_lines.push(op.as_asm_string());
            our_words.push(bytes);
            descriptions.push(format!("{op:?}"));
        }

        let assembled = assemble_instructions(&asm_lines);

        let mut mismatches: Vec<String> = Vec::new();

        if assembled.len() != our_words.len() {
            mismatches.push(format!(
                "Instruction count mismatch: we encoded {} but assembler produced {}",
                our_words.len(),
                assembled.len()
            ));
        }

        let compare_count = our_words.len().min(assembled.len());
        for i in 0..compare_count {
            if our_words[i] != assembled[i] {
                mismatches.push(format!(
                    "[{i}] {}\n    asm: {}\n    ours:   {:02x?}\n    theirs: {:02x?}",
                    descriptions[i], asm_lines[i], our_words[i], assembled[i]
                ));
            }
        }

        if !mismatches.is_empty() {
            panic!(
                "Bytecode mismatches ({} of {}):\n{}",
                mismatches.len(),
                our_words.len(),
                mismatches.join("\n\n")
            );
        }
    }
}
