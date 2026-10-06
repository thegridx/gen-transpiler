//! RISC-V ops and immediates.
use std::fmt;
use strum::{EnumIter, FromRepr};
use thiserror::Error;

use crate::common::SignType;
use crate::define_imm;
use crate::immediate::{BadImmediateValue, Immediate};
use crate::registers::RiscvRegister;

// ── Raw opcode / funct enums ───────────────────────────────────────────────

// see 2.4.1 and 2.4.2 (integer register operations)
#[repr(i32)]
#[derive(Debug, PartialEq, EnumIter, Clone, Copy, FromRepr)]
pub enum OpFunct3 {
    AddSub = 0,
    Sll = 1,  // shift left logical
    Slt = 2,  // set less then
    Sltu = 3, // set less then (U = zero extends)
    Xor = 4,
    SrlSra = 5, // shift right (logical / arithmetic = msb extends)
    Or = 6,
    And = 7,
}

// see 13.1 and 13.2 (extension M)
#[repr(i32)]
#[derive(Debug, PartialEq, EnumIter, FromRepr)]
pub(crate) enum OpMulDivFunct3 {
    Mul = 0,
    Mulh = 1,
    Mulhsu = 2,
    Mulhu = 3,
    Div = 4,
    Divu = 5,
    Rem = 6,
    Remu = 7,
}

#[repr(i32)]
#[derive(Debug, PartialEq, EnumIter, Clone, Copy, FromRepr)]
pub enum BranchFunct3 {
    Beq = 0,
    Bne = 1,
    Blt = 4,
    Bge = 5,
    Bltu = 6,
    Bgeu = 7,
}

#[repr(i32)]
#[derive(Debug, PartialEq, EnumIter, FromRepr)]
pub(crate) enum LoadStoreFunct3 {
    OneByte = 0,
    HalfWord = 1,
    OneWord = 2,
    UnsignedOneByte = 4,
    UnsignedHalfWord = 5,
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, EnumIter, FromRepr)]
pub enum AtomicFunct5 {
    AmoAdd = 0x0,
    AmoSwap = 0x1,
    Lr = 0x2,
    Sc = 0x3,
    AmoXor = 0x4,
    AmoOr = 0x8,
    AmoAnd = 0x0C,
    AmoMin = 0x10,
    AmoMax = 0x14,
    AmoMinU = 0x18,
    AmoMaxU = 0x1C,
}

// ── Immediate types ─────────────────────────────────────────────────────────

// I-type immediate: 12-bit signed [-2048..2048), used by OP-IMM, JALR, LOAD
define_imm!(RvImmI12, i16, 12, 0);
// S-type immediate: 12-bit signed [-2048..2048), stored as i32, used by STORE
define_imm!(RvImmS12, i32, 12, 0);
// B-type immediate: 13-bit signed [-4096..4096) with 1 trailing zero, used by BRANCH
define_imm!(RvImmB13, i32, 13, 1);
// J-type immediate: 21-bit signed [-1048576..1048576) with 1 trailing zero, used by JAL
define_imm!(RvImmJ21, i32, 21, 1);

/// U-type immediate: upper 20 bits with lower 12 bits always zero, used by LUI and AUIPC.
/// Cannot use the define_rv_imm macro because a full-width 32-bit range overflows the shift.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct RvImmU20s12(i32);
impl Immediate for RvImmU20s12 {
    type StorageType = i32;
    const N_BITS: u32 = 32;
    const N_TRAILING_ZEROS: u32 = 12;
    const IS_UNSIGNED: bool = false;
    // Per the RISC-V Unprivileged ISA specification (U-type immediates), the immediate fills bits
    // [31:12] and its low 12 bits are zero, so it spans the whole `i32` range in steps of 4096.
    const MAX_VAL: i32 = i32::MAX;
    const MIN_VAL: i32 = i32::MIN;
    fn is_valid(value: i32) -> bool {
        value.trailing_zeros() >= 12
    }
    fn value(&self) -> i32 {
        self.0
    }
}
impl TryFrom<i32> for RvImmU20s12 {
    type Error = BadImmediateValue;
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        if Self::is_valid(value) {
            Ok(Self(value))
        } else {
            Err(BadImmediateValue::new(value, "RvImmU20s12".to_string()))
        }
    }
}
impl fmt::Display for RvImmU20s12 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── Operation enums ─────────────────────────────────────────────────────────

/// RISC-V RV32I non-subtractive ALU operations (excluding shifts and set-less-than).
/// `Sub` is only valid in the register–register form and lives in [`RegArithmeticOp`].
#[derive(EnumIter, Clone, Copy, Debug)]
pub enum BasicOp {
    Add,
    And,
    Or,
    Xor,
}

/// RISC-V RV32I bit-shift operations.
#[derive(EnumIter, Clone, Copy, Debug)]
pub enum ShiftOp {
    Sll, // Shift left logical.
    Srl, // Shift right logical (zero-fill).
    Sra, // Shift right arithmetic (sign-fill).
}

/// In the RISC-V I-type encoding, `funct7` (and therefore the 5th bit that
/// distinguishes ADD/SUB) is absent for basic ops add/sub — the operation is always add.
/// Shifts still use the upper bits to select logical vs. arithmetic.
#[derive(Debug, Clone, Copy)]
pub enum ArithmeticOp {
    Basic(BasicOp),
    Shift(ShiftOp),
    SetLessThan(SignType), // SLT/SLTU
}
impl ArithmeticOp {
    pub fn new(op_funct_3: OpFunct3, funct_7_5th_bit: bool) -> Self {
        match op_funct_3 {
            OpFunct3::AddSub => Self::Basic(BasicOp::Add),
            OpFunct3::Or => Self::Basic(BasicOp::Or),
            OpFunct3::And => Self::Basic(BasicOp::And),
            OpFunct3::Xor => Self::Basic(BasicOp::Xor),
            OpFunct3::Sll => Self::Shift(ShiftOp::Sll),
            OpFunct3::SrlSra => Self::Shift(if funct_7_5th_bit {
                ShiftOp::Sra
            } else {
                ShiftOp::Srl
            }),
            OpFunct3::Slt => Self::SetLessThan(SignType::Signed),
            OpFunct3::Sltu => Self::SetLessThan(SignType::Unsigned),
        }
    }

    /// Returns the `funct3` encoding for this operation.
    pub(crate) fn funct3(&self) -> OpFunct3 {
        match self {
            Self::Basic(BasicOp::Add) => OpFunct3::AddSub,
            Self::Basic(BasicOp::Or) => OpFunct3::Or,
            Self::Basic(BasicOp::And) => OpFunct3::And,
            Self::Basic(BasicOp::Xor) => OpFunct3::Xor,
            Self::Shift(ShiftOp::Sll) => OpFunct3::Sll,
            Self::Shift(ShiftOp::Srl | ShiftOp::Sra) => OpFunct3::SrlSra,
            Self::SetLessThan(SignType::Signed) => OpFunct3::Slt,
            Self::SetLessThan(SignType::Unsigned) => OpFunct3::Sltu,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RegArithmeticOp {
    NonSub(ArithmeticOp),
    Sub, // Explicitly added for the 3-register form of the op
}
impl RegArithmeticOp {
    pub fn new(op_funct_3: OpFunct3, funct_7_5th_bit: bool) -> Self {
        match op_funct_3 {
            OpFunct3::AddSub => {
                if funct_7_5th_bit {
                    Self::Sub
                } else {
                    Self::NonSub(ArithmeticOp::Basic(BasicOp::Add))
                }
            }
            OpFunct3::Or => Self::NonSub(ArithmeticOp::Basic(BasicOp::Or)),
            OpFunct3::And => Self::NonSub(ArithmeticOp::Basic(BasicOp::And)),
            OpFunct3::Xor => Self::NonSub(ArithmeticOp::Basic(BasicOp::Xor)),
            OpFunct3::Sll => Self::NonSub(ArithmeticOp::Shift(ShiftOp::Sll)),
            OpFunct3::SrlSra => Self::NonSub(ArithmeticOp::Shift(if funct_7_5th_bit {
                ShiftOp::Sra
            } else {
                ShiftOp::Srl
            })),
            OpFunct3::Slt => Self::NonSub(ArithmeticOp::SetLessThan(SignType::Signed)),
            OpFunct3::Sltu => Self::NonSub(ArithmeticOp::SetLessThan(SignType::Unsigned)),
        }
    }

    /// Returns the `funct3` encoding for this operation.
    pub(crate) fn funct3(&self) -> OpFunct3 {
        match self {
            Self::Sub => OpFunct3::AddSub,
            Self::NonSub(arith) => arith.funct3(),
        }
    }

    /// Returns whether bit 5 of `funct7` (bit 30 of instruction) is set.
    pub(crate) fn funct7_bit5(&self) -> bool {
        matches!(
            self,
            Self::Sub | Self::NonSub(ArithmeticOp::Shift(ShiftOp::Sra))
        )
    }
}

/// RV32M multiply-high variants: which 32-bit half of the 64-bit product to keep.
#[derive(EnumIter, Debug, Clone, Copy)]
pub enum MulHighOp {
    Mulh,   // `MULH` — signed × signed, upper 32 bits
    Mulhu,  // `MULHU` — unsigned × unsigned, upper 32 bits
    Mulhsu, // `MULHSU` — signed × unsigned, upper 32 bits
}

/// RV32M integer division and remainder variants.
#[derive(EnumIter, Debug, Clone, Copy)]
pub enum DivRemOp {
    Div(SignType),
    Rem(SignType),
}

/// All RV32M multiply/divide/remainder operations, decoded from `funct3`.
#[derive(Debug, Clone, Copy)]
pub enum MulDivRemOp {
    MulLow,
    MulHigh(MulHighOp),
    DivRem(DivRemOp),
}

impl From<OpMulDivFunct3> for MulDivRemOp {
    fn from(op_mul_div_funct: OpMulDivFunct3) -> Self {
        match op_mul_div_funct {
            OpMulDivFunct3::Mul => Self::MulLow,
            OpMulDivFunct3::Mulh => Self::MulHigh(MulHighOp::Mulh),
            OpMulDivFunct3::Mulhu => Self::MulHigh(MulHighOp::Mulhu),
            OpMulDivFunct3::Mulhsu => Self::MulHigh(MulHighOp::Mulhsu),
            OpMulDivFunct3::Div => Self::DivRem(DivRemOp::Div(SignType::Signed)),
            OpMulDivFunct3::Divu => Self::DivRem(DivRemOp::Div(SignType::Unsigned)),
            OpMulDivFunct3::Rem => Self::DivRem(DivRemOp::Rem(SignType::Signed)),
            OpMulDivFunct3::Remu => Self::DivRem(DivRemOp::Rem(SignType::Unsigned)),
        }
    }
}

impl MulDivRemOp {
    /// Returns the `funct3` encoding for this operation.
    pub(crate) fn funct3(&self) -> OpMulDivFunct3 {
        match self {
            Self::MulLow => OpMulDivFunct3::Mul,
            Self::MulHigh(MulHighOp::Mulh) => OpMulDivFunct3::Mulh,
            Self::MulHigh(MulHighOp::Mulhu) => OpMulDivFunct3::Mulhu,
            Self::MulHigh(MulHighOp::Mulhsu) => OpMulDivFunct3::Mulhsu,
            Self::DivRem(DivRemOp::Div(SignType::Signed)) => OpMulDivFunct3::Div,
            Self::DivRem(DivRemOp::Div(SignType::Unsigned)) => OpMulDivFunct3::Divu,
            Self::DivRem(DivRemOp::Rem(SignType::Signed)) => OpMulDivFunct3::Rem,
            Self::DivRem(DivRemOp::Rem(SignType::Unsigned)) => OpMulDivFunct3::Remu,
        }
    }
}

/// Width and signedness of a RISC-V load instruction.
///
/// Matches the `funct3` encoding of `LB`/`LH`/`LW`/`LBU`/`LHU`. The `repr(u8)`
/// value equals the number of bytes transferred.
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum LoadSize {
    OneByte(SignType) = 1,
    HalfWord(SignType) = 2,
    OneWord = 4,
}
impl LoadSize {
    /// Returns the number of bytes *after* the first that must be read for a multi-byte load.
    /// Used to compute boundary checks: size_in_bytes - 1.
    pub fn remainder_size_in_bytes(&self) -> u8 {
        match self {
            LoadSize::OneByte(_) => 0,
            LoadSize::HalfWord(_) => 1,
            LoadSize::OneWord => 3,
        }
    }

    /// Returns the `funct3` encoding for this load size.
    pub(crate) fn funct3(&self) -> LoadStoreFunct3 {
        match self {
            Self::OneByte(SignType::Signed) => LoadStoreFunct3::OneByte,
            Self::OneByte(SignType::Unsigned) => LoadStoreFunct3::UnsignedOneByte,
            Self::HalfWord(SignType::Signed) => LoadStoreFunct3::HalfWord,
            Self::HalfWord(SignType::Unsigned) => LoadStoreFunct3::UnsignedHalfWord,
            Self::OneWord => LoadStoreFunct3::OneWord,
        }
    }
}

/// Width of a RISC-V store instruction.
///
/// Matches the `funct3` encoding of `SB`/`SH`/`SW`. The `repr(u8)` value equals
/// the number of bytes written.
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum StoreSize {
    OneByte = 1,
    HalfWord = 2,
    OneWord = 4,
}
impl StoreSize {
    /// Returns the `funct3` encoding for this store size.
    pub(crate) fn funct3(&self) -> LoadStoreFunct3 {
        match self {
            Self::OneByte => LoadStoreFunct3::OneByte,
            Self::HalfWord => LoadStoreFunct3::HalfWord,
            Self::OneWord => LoadStoreFunct3::OneWord,
        }
    }
}

/// Branch condition decoded from the RISC-V `funct3` field
pub type BranchCondition = BranchFunct3;

/// Atomic operation decoded from the RISC-V `funct5` field
pub type AtomicOp = AtomicFunct5;

// ── Instruction types ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ThreeRegisters {
    rs1: RiscvRegister,
    rs2: RiscvRegister,
    rd: RiscvRegister,
}
impl ThreeRegisters {
    pub fn new(rs1: RiscvRegister, rs2: RiscvRegister, rd: RiscvRegister) -> Self {
        Self { rs1, rs2, rd }
    }
}

/// The meaning of r2 is either src2 or dst depending on the instruction type
#[derive(Debug, Clone)]
pub struct RegisterAndImmediate<I: Immediate> {
    r: RiscvRegister,
    imm: I,
}
impl<I: Immediate> RegisterAndImmediate<I> {
    pub fn new(r: RiscvRegister, imm: I) -> Self {
        Self { r, imm }
    }
}

#[derive(Debug, Clone)]
pub struct TwoRegistersAndImmediate<I: Immediate> {
    r1: RiscvRegister,
    r2: RiscvRegister,
    imm: I,
}
impl<I: Immediate> TwoRegistersAndImmediate<I> {
    pub fn new(r1: RiscvRegister, r2: RiscvRegister, imm: I) -> Self {
        Self { r1, r2, imm }
    }
}

/// OP-IMM: I-type arithmetic/logical with immediate.
///
/// For shift instructions, the immediate encodes both the shift amount (lower 5 bits)
/// and the shift type (bit 10 distinguishes SRL from SRA). The `op` field is the
/// authoritative source for the operation type.
#[derive(Debug, Clone)]
pub struct RvOpImm {
    op: ArithmeticOp,
    params: TwoRegistersAndImmediate<RvImmI12>,
}
impl RvOpImm {
    pub fn new(op: ArithmeticOp, params: TwoRegistersAndImmediate<RvImmI12>) -> Self {
        Self { op, params }
    }
    pub fn op(&self) -> ArithmeticOp {
        self.op
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.r1
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.r2
    }
    pub fn imm(&self) -> RvImmI12 {
        self.params.imm
    }
}

/// OP: R-type register-register arithmetic/logical.
#[derive(Debug, Clone)]
pub struct RvOpReg {
    op: RegArithmeticOp,
    params: ThreeRegisters,
}
impl RvOpReg {
    pub fn new(op: RegArithmeticOp, params: ThreeRegisters) -> Self {
        Self { op, params }
    }
    pub fn op(&self) -> RegArithmeticOp {
        self.op
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.rs1
    }
    pub fn rs2(&self) -> RiscvRegister {
        self.params.rs2
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.rd
    }
}

/// LUI: U-type load upper immediate.
#[derive(Debug, Clone)]
pub struct RvLui {
    params: RegisterAndImmediate<RvImmU20s12>,
}
impl RvLui {
    pub fn new(params: RegisterAndImmediate<RvImmU20s12>) -> Self {
        Self { params }
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.r
    }
    pub fn imm(&self) -> RvImmU20s12 {
        self.params.imm
    }
}

/// AUIPC: U-type add upper immediate to PC.
#[derive(Debug, Clone)]
pub struct RvAuipc {
    params: RegisterAndImmediate<RvImmU20s12>,
}
impl RvAuipc {
    pub fn new(params: RegisterAndImmediate<RvImmU20s12>) -> Self {
        Self { params }
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.r
    }
    pub fn imm(&self) -> RvImmU20s12 {
        self.params.imm
    }
}

/// RV32M: R-type multiply/divide/remainder.
#[derive(Debug, Clone)]
pub struct RvMulDivRem {
    op: MulDivRemOp,
    params: ThreeRegisters,
}
impl RvMulDivRem {
    pub fn new(op: MulDivRemOp, params: ThreeRegisters) -> Self {
        Self { op, params }
    }
    pub fn op(&self) -> MulDivRemOp {
        self.op
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.rs1
    }
    pub fn rs2(&self) -> RiscvRegister {
        self.params.rs2
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.rd
    }
}

/// JAL: J-type jump and link.
#[derive(Debug, Clone)]
pub struct RvJal {
    params: RegisterAndImmediate<RvImmJ21>,
}
impl RvJal {
    pub fn new(params: RegisterAndImmediate<RvImmJ21>) -> Self {
        Self { params }
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.r
    }
    pub fn imm(&self) -> RvImmJ21 {
        self.params.imm
    }
}
/// JALR: I-type jump and link register.
#[derive(Debug, Clone)]
pub struct RvJalr {
    params: TwoRegistersAndImmediate<RvImmI12>,
}
impl RvJalr {
    pub fn new(params: TwoRegistersAndImmediate<RvImmI12>) -> Self {
        Self { params }
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.r1
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.r2
    }
    pub fn imm(&self) -> RvImmI12 {
        self.params.imm
    }
}

/// Branch: B-type conditional branch.
#[derive(Debug, Clone)]
pub struct RvBranch {
    condition: BranchCondition,
    params: TwoRegistersAndImmediate<RvImmB13>,
}
impl RvBranch {
    pub fn new(condition: BranchCondition, params: TwoRegistersAndImmediate<RvImmB13>) -> Self {
        Self { condition, params }
    }
    pub fn condition(&self) -> BranchCondition {
        self.condition
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.r1
    }
    pub fn rs2(&self) -> RiscvRegister {
        self.params.r2
    }
    pub fn imm(&self) -> RvImmB13 {
        self.params.imm
    }
    /// The same branch with the opposite condition: taken exactly when this one is not.
    pub fn inverted(&self) -> Self {
        let condition = match self.condition {
            BranchCondition::Beq => BranchCondition::Bne,
            BranchCondition::Bne => BranchCondition::Beq,
            BranchCondition::Blt => BranchCondition::Bge,
            BranchCondition::Bge => BranchCondition::Blt,
            BranchCondition::Bltu => BranchCondition::Bgeu,
            BranchCondition::Bgeu => BranchCondition::Bltu,
        };
        Self {
            condition,
            params: self.params.clone(),
        }
    }
}

/// Load: I-type load from memory.
#[derive(Debug, Clone)]
pub struct RvLoad {
    size: LoadSize,
    params: TwoRegistersAndImmediate<RvImmI12>,
}
impl RvLoad {
    pub fn new(size: LoadSize, params: TwoRegistersAndImmediate<RvImmI12>) -> Self {
        Self { size, params }
    }
    pub fn size(&self) -> LoadSize {
        self.size
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.r1
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.r2
    }
    pub fn imm(&self) -> RvImmI12 {
        self.params.imm
    }
}

/// Store: S-type store to memory.
#[derive(Debug, Clone)]
pub struct RvStore {
    size: StoreSize,
    params: TwoRegistersAndImmediate<RvImmS12>,
}
impl RvStore {
    pub fn new(size: StoreSize, params: TwoRegistersAndImmediate<RvImmS12>) -> Self {
        Self { size, params }
    }
    pub fn size(&self) -> StoreSize {
        self.size
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.r1
    }
    pub fn rs2(&self) -> RiscvRegister {
        self.params.r2
    }
    pub fn imm(&self) -> RvImmS12 {
        self.params.imm
    }
}

/// Atomic: R-type atomic memory operation (RV32A).
///
/// The `aq` (acquire) and `rl` (release) ordering bits are kept so the instruction re-encodes
/// exactly. They have no effect on single-threaded execution.
#[derive(Debug, Clone)]
pub struct RvAtomic {
    op: AtomicOp,
    params: ThreeRegisters,
    aq: bool,
    rl: bool,
}
impl RvAtomic {
    pub fn new(op: AtomicOp, params: ThreeRegisters) -> Self {
        Self {
            op,
            params,
            aq: false,
            rl: false,
        }
    }
    /// Sets the acquire (`aq`) and release (`rl`) ordering bits.
    pub(crate) fn with_ordering(mut self, aq: bool, rl: bool) -> Self {
        self.aq = aq;
        self.rl = rl;
        self
    }
    pub fn op(&self) -> AtomicOp {
        self.op
    }
    pub(crate) fn aq(&self) -> bool {
        self.aq
    }
    pub(crate) fn rl(&self) -> bool {
        self.rl
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.params.rs1
    }
    pub fn rs2(&self) -> RiscvRegister {
        self.params.rs2
    }
    pub fn rd(&self) -> RiscvRegister {
        self.params.rd
    }
}
impl From<&RvAtomic> for RvLoad {
    fn from(atomic: &RvAtomic) -> Self {
        Self {
            size: LoadSize::OneWord, // All RV32A atomic ops are word-sized
            params: TwoRegistersAndImmediate::new(
                atomic.rs1(),
                atomic.rd(),
                RvImmI12(0), // Atomics address `rs1` itself, with no offset
            ),
        }
    }
}
impl From<&RvAtomic> for RvStore {
    fn from(atomic: &RvAtomic) -> Self {
        Self {
            size: StoreSize::OneWord, // All RV32A atomic ops are word-sized
            params: TwoRegistersAndImmediate::new(
                atomic.rs1(),
                atomic.rs2(),
                RvImmS12(0), // Atomics address `rs1` itself, with no offset
            ),
        }
    }
}

/// Fence: orders memory and I/O accesses (MISC-MEM opcode, funct3 = 0).
///
/// A fence has no effect on single-threaded execution. Its fields are kept so the instruction
/// re-encodes exactly: `fm` is the fence mode (`0b1000` for `fence.tso`), and `pred`/`succ` are
/// the predecessor and successor sets as `IORW` bit masks. `rs1` and `rd` are reserved; the
/// spec requires implementations to ignore them.
#[derive(Debug, Clone)]
pub struct RvFence {
    fm: u8,
    pred: u8,
    succ: u8,
    rs1: RiscvRegister,
    rd: RiscvRegister,
}
impl RvFence {
    /// Creates a fence; `fm`, `pred` and `succ` are truncated to their 4-bit fields.
    pub fn new(fm: u8, pred: u8, succ: u8, rs1: RiscvRegister, rd: RiscvRegister) -> Self {
        Self {
            fm: fm & 0xF,
            pred: pred & 0xF,
            succ: succ & 0xF,
            rs1,
            rd,
        }
    }
    pub(crate) fn fm(&self) -> u8 {
        self.fm
    }
    pub(crate) fn pred(&self) -> u8 {
        self.pred
    }
    pub(crate) fn succ(&self) -> u8 {
        self.succ
    }
    pub fn rs1(&self) -> RiscvRegister {
        self.rs1
    }
    pub fn rd(&self) -> RiscvRegister {
        self.rd
    }
}

// ── Top-level instruction enum ──────────────────────────────────────────────

/// A decoded RV32IMA instruction.
#[derive(Debug, Clone)]
pub enum RiscvOp {
    OpImm(RvOpImm),
    OpReg(RvOpReg),
    Lui(RvLui),
    Auipc(RvAuipc),
    MulDivRem(RvMulDivRem),
    Jal(RvJal),
    Jalr(RvJalr),
    Branch(RvBranch),
    Load(RvLoad),
    Store(RvStore),
    Ecall,
    Atomic(RvAtomic),
    Fence(RvFence),
}

/// Error returned when decoding a raw `i32` into a [`RiscvOp`] fails.
#[derive(Debug, Clone, Copy, Error)]
pub enum RvDecodeError {
    #[error("compressed (16-bit) instruction")]
    CompressedInstruction,
    #[error("invalid opcode: {opcode:#09b}")]
    InvalidOpcode { opcode: i32 },
    #[error("invalid funct3: {funct3:#05b}")]
    InvalidFunct3 { funct3: i32 },
    #[error("invalid funct5 for atomic instruction: {funct5:#07b}")]
    InvalidAtomicFunct5 { funct5: i32 },
    #[error("invalid atomic width funct3: {funct3:#05b} (expected 2)")]
    InvalidAtomicWidth { funct3: i32 },
    #[error("store instruction with funct3 indicating a signed load")]
    SignedCodeInStore,
    #[error("bad immediate value: {imm}")]
    InvalidImmediate { imm: i64 },
    #[error("bad register value: {reg}")]
    InvalidRegister { reg: u32 },
    #[error("reserved funct7 bits set: funct7={funct7:#09b}")]
    ReservedFunct7Bits { funct7: i32 },
    #[error("LR.W with a non-zero rs2 field, which is reserved")]
    ReservedLrSource,
    #[error("system call is not supported")]
    UnsupportedSystemCall,
}

#[cfg(test)]
mod tests {
    use googletest::prelude::{assert_that, none};
    use strum::IntoEnumIterator;

    use super::BasicOp;

    #[test]
    fn basic_op_iteration_includes_all_variants_in_definition_order() {
        let mut iter = BasicOp::iter();

        let op_add = iter.next();
        assert!(
            matches!(op_add, Some(BasicOp::Add)),
            "expected Some(Add), got {op_add:?}"
        );
        let op_and = iter.next();
        assert!(
            matches!(op_and, Some(BasicOp::And)),
            "expected Some(And), got {op_and:?}"
        );
        let op_or = iter.next();
        assert!(
            matches!(op_or, Some(BasicOp::Or)),
            "expected Some(Or), got {op_or:?}"
        );
        let op_xor = iter.next();
        assert!(
            matches!(op_xor, Some(BasicOp::Xor)),
            "expected Some(Xor), got {op_xor:?}"
        );
        assert_that!(iter.next(), none());
    }
}
