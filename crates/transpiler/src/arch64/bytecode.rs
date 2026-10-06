//! Encodes resolved aarch64 ops as 32-bit instruction words.
//!
//! The encoder tests compare each encoding with the output of `aarch64-linux-gnu-as`.
use riscv32::immediate::{ImmU12_0, Immediate};

use crate::arch64::{
    ops::{Arm64NativeOp, ImmI9_2},
    registers::{ArmRegister, REG_GAS, REG_TMP, REG_TMP_2},
};

/// Drops the immediate's implied trailing zeros and masks it to its encoded width.
fn encode_immediate_32bit<I: Immediate>(val: I) -> u32
where
    i32: From<<I as Immediate>::StorageType>,
{
    let mask: u32 = (1u32 << (I::N_BITS - I::N_TRAILING_ZEROS)) - 1;
    ((i32::from(val.value()) >> I::N_TRAILING_ZEROS) as u32) & mask
}

/// Like `encode_immediate_32bit`, for immediates stored in an `i64`.
fn encode_immediate_64bit<I: Immediate>(val: I) -> u32
where
    i64: From<<I as Immediate>::StorageType>,
{
    let mask: u32 = (1u32 << (I::N_BITS - I::N_TRAILING_ZEROS)) - 1;
    ((i64::from(val.value()) >> I::N_TRAILING_ZEROS) as u32) & mask
}

// The field encoders below mask every value to its field width, so an out-of-range value can
// never change the opcode bits around the field. Callers validate values before encoding.
const REG_MASK: u32 = 0x1F;
const IMM12_MASK: u32 = 0xFFF;
const IMM7_MASK: u32 = 0x7F;

/// Encodes three registers: Rm at bits `[20:16]`, Rn at bits `[9:5]`, Rd at bits `[4:0]`.
fn regs_rm_rn_rd(rm: u32, rn: u32, rd: u32) -> u32 {
    ((rm & REG_MASK) << 16) | ((rn & REG_MASK) << 5) | (rd & REG_MASK)
}

/// Encodes two registers: Rn at bits `[9:5]`, Rd at bits `[4:0]`.
fn regs_rn_rd(rn: u32, rd: u32) -> u32 {
    ((rn & REG_MASK) << 5) | (rd & REG_MASK)
}

/// Encodes imm12 at bits `[21:10]`, Rn at bits `[9:5]`, Rd at bits `[4:0]`.
fn imm12_rn_rd(imm12: u32, rn: u32, rd: u32) -> u32 {
    ((imm12 & IMM12_MASK) << 10) | ((rn & REG_MASK) << 5) | (rd & REG_MASK)
}

/// Encodes an `STP`/`LDP` pair: imm7 at bits `[21:15]`, Rt2 at `[14:10]`, Rn at `[9:5]` and
/// Rt1 at `[4:0]`.
fn pair_imm7_rt2_rn_rt1(imm7: u32, rt2: u32, rn: u32, rt1: u32) -> u32 {
    ((imm7 & IMM7_MASK) << 15)
        | ((rt2 & REG_MASK) << 10)
        | ((rn & REG_MASK) << 5)
        | (rt1 & REG_MASK)
}

/// Encodes a 32-bit-register `STP`/`LDP` pair with a 4-byte-scaled offset.
fn encode_pair_i9_w(imm: &ImmI9_2, rt2: u32, rn: u32, rt1: u32) -> u32 {
    pair_imm7_rt2_rn_rt1(encode_immediate_32bit(*imm) & 0x7F, rt2, rn, rt1)
}

impl Arm64NativeOp {
    /// Returns the 32-bit instruction word for this op. `NativeOp::encode` writes it out in
    /// little-endian byte order.
    pub(crate) fn arm64_encoding(&self) -> u32 {
        let encoding: u32 = match self {
            Arm64NativeOp::Nop => 0xD503201F,
            Arm64NativeOp::Adrp_xRegTmp_Label(pc_rel) => {
                // ADRP: immlo (2 bits) at [30:29], immhi (19 bits) at [23:5]
                let imm = encode_immediate_64bit(*pc_rel);
                let immlo = imm & 0x3;
                let immhi = (imm >> 2) & 0x7FFFF;
                0x90000000 | (immlo << 29) | (immhi << 5) | REG_TMP.encoding()
            }
            Arm64NativeOp::Adr_xRegTmp(pc_rel) => {
                // ADR: immlo (2 bits) at [30:29], immhi (19 bits) at [23:5]
                let imm = encode_immediate_32bit(*pc_rel);
                let immlo = imm & 0x3;
                let immhi = (imm >> 2) & 0x7FFFF;
                0x10000000 | (immlo << 29) | (immhi << 5) | REG_TMP.encoding()
            }
            Arm64NativeOp::Add_xRegTmp_xRegTmp_Lo12Label(lo12) => {
                0x91000000
                    | imm12_rn_rd(
                        encode_immediate_32bit(*lo12),
                        REG_TMP.encoding(),
                        REG_TMP.encoding(),
                    )
            }

            Arm64NativeOp::Adds_W_W_Imm(rd, rn, imm) => {
                0x31000000 | imm12_rn_rd(imm.0 as u32, rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_Imm(imm) => {
                0xB1000000 | imm12_rn_rd(imm.0 as u32, REG_TMP.encoding(), REG_TMP.encoding())
            }
            Arm64NativeOp::Adds_W_W_W(rd, rn, rm) => {
                0x2B000000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            // ADDS (extended register), option SXTW = 0b110 at bits [15:13]
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(rm) => {
                0xAB20C000 | regs_rm_rn_rd(rm.encoding(), REG_TMP.encoding(), REG_TMP.encoding())
            }
            Arm64NativeOp::Subs_W_W_Imm(rd, rn, imm) => {
                0x71000000 | imm12_rn_rd(imm.0 as u32, rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(imm) => {
                0x71400000 | imm12_rn_rd(imm.0 as u32, REG_TMP.encoding(), REG_TMP.encoding())
            }
            Arm64NativeOp::Subs_xRegTmp_xRegTmp_Imm(imm) => {
                0xF1000000 | imm12_rn_rd(imm.0 as u32, REG_TMP.encoding(), REG_TMP.encoding())
            }
            Arm64NativeOp::Subs_W_W_W(rd, rn, rm) => {
                0x6B000000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Eor_W_W_W(rd, rn, rm) => {
                0x4A000000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Eon_W_W_W(rd, rn, rm) => {
                0x4A200000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Orr_W_W_W(rd, rn, rm) => {
                0x2A000000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Orn_W_W_W(rd, rn, rm) => {
                0x2A200000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::And_W_W_W(rd, rn, rm) => {
                0x0A000000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            // AND (immediate) with the bitmask 0xFFFF_FFFC: 30 ones rotated right by 30.
            Arm64NativeOp::Bic_W_W_3(rd, rn) => {
                let (n, immr, imms) = (0, 30, 29);
                0x12000000
                    | (n << 22)
                    | (immr << 16)
                    | (imms << 10)
                    | regs_rn_rd(rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Bic_W_W_W(rd, rn, rm) => {
                0x0A200000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            // Alias of ANDS WZR, Wn, #2 (bitmask immediate: one set bit rotated right by 31)
            Arm64NativeOp::Tst_W_2(rn) => {
                let (n, immr, imms) = (0, 31, 0);
                0x72000000
                    | (n << 22)
                    | (immr << 16)
                    | (imms << 10)
                    | regs_rn_rd(rn.encoding(), ArmRegister::Zr.encoding())
            }
            // UBFM Wd, Wn, #shift, #31
            Arm64NativeOp::Lsr_W_W_Imm(rd, rn, imm) => {
                0x53007C00
                    | (encode_immediate_32bit(*imm) << 16)
                    | regs_rn_rd(rn.encoding(), rd.encoding())
            }
            // UBFM Xd, Xn, #32, #63
            Arm64NativeOp::Lsr_X_xRegTmp_32(xd) => {
                0xD360FC00 | regs_rn_rd(REG_TMP.encoding(), xd.encoding())
            }
            // LSRV
            Arm64NativeOp::Lsr_W_W_W(rd, rn, rm) => {
                0x1AC02400 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            // SBFM Wd, Wn, #shift, #31
            Arm64NativeOp::Asr_W_W_Imm(rd, rn, imm) => {
                0x13007C00
                    | (encode_immediate_32bit(*imm) << 16)
                    | regs_rn_rd(rn.encoding(), rd.encoding())
            }
            // SBFM Xd, Xn, #shift, #63
            Arm64NativeOp::Asr_xRegTmp_xRegTmp_Imm(imm) => {
                0x9340FC00
                    | (encode_immediate_32bit(*imm) << 16)
                    | regs_rn_rd(REG_TMP.encoding(), REG_TMP.encoding())
            }
            // ASRV
            Arm64NativeOp::Asr_W_W_W(rd, rn, rm) => {
                0x1AC02800 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            // UBFM Wd, Wn, #(-shift MOD 32), #(31-shift)
            Arm64NativeOp::Lsl_W_W_Imm(rd, rn, imm) => {
                let shift = encode_immediate_32bit(*imm);
                let immr = (32 - shift) & 0x1F;
                let imms = 31 - shift;
                0x53000000 | (immr << 16) | (imms << 10) | regs_rn_rd(rn.encoding(), rd.encoding())
            }
            // UBFM Xd, Xn, #(-shift MOD 64), #(63-shift)
            Arm64NativeOp::Lsl_xRegTmp_xRegTmp_Imm(imm) => {
                let shift = encode_immediate_32bit(*imm);
                let immr = (64 - shift) & 0x3F;
                let imms = 63 - shift;
                0xD3400000
                    | (immr << 16)
                    | (imms << 10)
                    | regs_rn_rd(REG_TMP.encoding(), REG_TMP.encoding())
            }
            // LSLV
            Arm64NativeOp::Lsl_W_W_W(rd, rn, rm) => {
                0x1AC02000 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            // Alias of SUBS (CMP) or ADDS (CMN) with Rd = WZR
            Arm64NativeOp::Cmp_W_Imm(rn, imm) => {
                if imm.0 >= 0 {
                    return Arm64NativeOp::Subs_W_W_Imm(
                        ArmRegister::Zr,
                        *rn,
                        ImmU12_0(imm.0 as u16),
                    )
                    .arm64_encoding();
                } else {
                    return Arm64NativeOp::Adds_W_W_Imm(
                        ArmRegister::Zr,
                        *rn,
                        ImmU12_0((-imm.0) as u16),
                    )
                    .arm64_encoding();
                }
            }
            // Alias of SUBS with Rd = WZR
            Arm64NativeOp::Cmp_W_W(rn, rm) => {
                return Arm64NativeOp::Subs_W_W_W(ArmRegister::Zr, *rn, *rm).arm64_encoding();
            }
            // Alias of CSINC Wd, WZR, WZR, invert(cond)
            Arm64NativeOp::Cset_W(rd, cond) => {
                0x1A9F07E0 | ((cond.inverted() as u32) << 12) | rd.encoding()
            }
            Arm64NativeOp::Csel_wRegTmp2_wRegTmp2_wRegTmp(cond) => {
                0x1A800000
                    | ((REG_TMP.encoding()) << 16)
                    | ((*cond as u32) << 12)
                    | regs_rn_rd(REG_TMP_2.encoding(), REG_TMP_2.encoding())
            }
            Arm64NativeOp::Movz_W_Imm_Shift(rd, imm16, shift) => {
                0x52800000
                    | (encode_immediate_32bit(*shift) << 21)
                    | ((*imm16 as u32) << 5)
                    | rd.encoding()
            }
            Arm64NativeOp::Movk_W_Imm_Shift(rd, imm16, shift) => {
                0x72800000
                    | (encode_immediate_32bit(*shift) << 21)
                    | ((*imm16 as u32) << 5)
                    | rd.encoding()
            }
            Arm64NativeOp::Mov_W_Imm(rd, imm) => {
                let val = imm.value();
                if val >= 0 {
                    // MOVZ Wd, #imm
                    0x52800000 | ((val as u32) << 5) | rd.encoding()
                } else {
                    // MOVN Wd, #~imm (preferred encoding for negative values)
                    let inverted = (!(val as i32) as u32) & 0xFFFF;
                    0x12800000 | (inverted << 5) | rd.encoding()
                }
            }
            // Alias of ORR Wd, WZR, Wm
            Arm64NativeOp::Mov_W_W(rd, rm) => {
                0x2A000000 | regs_rm_rn_rd(rm.encoding(), ArmRegister::Zr.encoding(), rd.encoding())
            }
            // Alias of MADD Wd, Wn, Wm, WZR
            Arm64NativeOp::Mul_W_W_W(rd, rn, rm) => {
                0x1B007C00 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Mul_xRegTmp_xRegTmp_xRegTmp2 => {
                0x9B007C00
                    | regs_rm_rn_rd(REG_TMP_2.encoding(), REG_TMP.encoding(), REG_TMP.encoding())
            }
            Arm64NativeOp::Smull_xRegTmp_W_W(wn, wm) => {
                0x9B207C00 | regs_rm_rn_rd(wm.encoding(), wn.encoding(), REG_TMP.encoding())
            }
            Arm64NativeOp::Umull_xRegTmp_W_W(wn, wm) => {
                0x9BA07C00 | regs_rm_rn_rd(wm.encoding(), wn.encoding(), REG_TMP.encoding())
            }
            // Alias of SBFM Xd, Xn, #0, #31
            Arm64NativeOp::Sxtw(xd, wn) => 0x93407C00 | regs_rn_rd(wn.encoding(), xd.encoding()),
            Arm64NativeOp::Sdiv_W_W_W(rd, rn, rm) => {
                0x1AC00C00 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Udiv_W_W_W(rd, rn, rm) => {
                0x1AC00800 | regs_rm_rn_rd(rm.encoding(), rn.encoding(), rd.encoding())
            }
            Arm64NativeOp::Cbz_W(rt, p) => {
                let off = *p;
                0x34000000 | ((encode_immediate_32bit(off) & 0x7FFFF) << 5) | rt.encoding()
            }
            Arm64NativeOp::Cbnz_W(rt, p) => {
                let off = *p;
                0x35000000 | ((encode_immediate_32bit(off) & 0x7FFFF) << 5) | rt.encoding()
            }
            Arm64NativeOp::B(p) => {
                let off = *p;
                0x14000000 | (encode_immediate_32bit(off) & 0x3FFFFFF)
            }
            Arm64NativeOp::Bcc(p, cc) => {
                let off = *p;
                0x54000000 | ((encode_immediate_32bit(off) & 0x7FFFF) << 5) | *cc as u32
            }
            Arm64NativeOp::Br_xRegTmp => 0xD61F0000 | ((REG_TMP.encoding()) << 5),
            Arm64NativeOp::LdrsbW_xRegTmp_0(rt) => {
                0x39C00000 | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
            }
            Arm64NativeOp::LdurshW_xRegTmp_0(rt) => {
                0x78C00000 | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
            }
            Arm64NativeOp::Ldur_W_xRegTmp_Imm(rt, imm) => {
                0xB8400000
                    | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
                    | ((imm.0 as u32 & 0x1FF) << 12)
            }
            Arm64NativeOp::Ldur_X_xRegTmp_Imm(rt, imm) => {
                0xF8400000
                    | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
                    | ((imm.0 as u32 & 0x1FF) << 12)
            }
            Arm64NativeOp::Ldr_W_xSp_Imm(rt, imm) => {
                0xB9400000 | imm12_rn_rd(encode_immediate_32bit(*imm), 31, rt.encoding())
            }
            Arm64NativeOp::Ldr_xRegTmp_X_Imm(xn, imm) => {
                0xF9400000
                    | imm12_rn_rd(
                        encode_immediate_32bit(*imm),
                        xn.encoding(),
                        REG_TMP.encoding(),
                    )
            }
            Arm64NativeOp::LdrbW_xRegTmp_0(rt) => {
                0x39400000 | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
            }
            Arm64NativeOp::LdurhW_xRegTmp_0(rt) => {
                0x78400000 | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
            }
            Arm64NativeOp::Stur_W_xRegTmp_Imm(rt, imm) => {
                0xB8000000
                    | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
                    | ((imm.0 as u32 & 0x1FF) << 12)
            }
            Arm64NativeOp::Stur_X_xRegTmp_Imm(rt, imm) => {
                0xF8000000
                    | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
                    | ((imm.0 as u32 & 0x1FF) << 12)
            }
            Arm64NativeOp::Str_W_xSp_Imm(rt, imm) => {
                0xB9000000 | imm12_rn_rd(encode_immediate_32bit(*imm), 31, rt.encoding())
            }
            Arm64NativeOp::Str_xRegTmp_xSp_Imm(imm) => {
                0xF9000000 | imm12_rn_rd(encode_immediate_32bit(*imm), 31, REG_TMP.encoding())
            }
            Arm64NativeOp::Strb_W_xRegTmp_0(rt) => {
                0x39000000 | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
            }
            Arm64NativeOp::Sturh_W_xRegTmp_0(rt) => {
                0x78000000 | regs_rn_rd(REG_TMP.encoding(), rt.encoding())
            }
            Arm64NativeOp::Ldp_X_X_xSp_adv16(rt1, rt2) => {
                0xA8C00000 | pair_imm7_rt2_rn_rt1(2, rt2.encoding(), 31, rt1.encoding())
            }
            Arm64NativeOp::Stp_X_X_xSp_minus16(rt1, rt2) => {
                0xA9800000
                    | pair_imm7_rt2_rn_rt1(
                        ((-2i32) as u32) & 0x7F,
                        rt2.encoding(),
                        31,
                        rt1.encoding(),
                    )
            }
            Arm64NativeOp::Ldp_W_W_xSp_Imm(rt1, rt2, imm) => {
                0x29400000 | encode_pair_i9_w(imm, rt2.encoding(), 31, rt1.encoding())
            }
            Arm64NativeOp::Stp_W_W_xSp_Imm(rt1, rt2, imm) => {
                0x29000000 | encode_pair_i9_w(imm, rt2.encoding(), 31, rt1.encoding())
            }
            Arm64NativeOp::Add_xSp_Imm(imm) => {
                if imm.value() >= 0 {
                    0x91000000 | imm12_rn_rd(imm.value() as u32, 31, 31)
                } else {
                    0xD1000000 | imm12_rn_rd((-imm.value()) as u32, 31, 31)
                }
            }
            Arm64NativeOp::Ret_Lr => 0xD65F0000 | ((ArmRegister::Lr.encoding()) << 5),
            Arm64NativeOp::Subs_xRegGas_xRegGas_X(xm) => {
                0xEB000000 | regs_rm_rn_rd(xm.encoding(), REG_GAS.encoding(), REG_GAS.encoding())
            }
            Arm64NativeOp::Subs_xRegGas_xRegGas_Imm(imm) => {
                0xF1000000 | imm12_rn_rd(imm.0 as u32, REG_GAS.encoding(), REG_GAS.encoding())
            }
        };

        encoding
    }
}

#[cfg(test)]
mod tests {
    use googletest::prelude::{anything, err, ok};
    use googletest::{assert_that, matchers::pat};
    use rstest::rstest;

    use crate::arch64::bytecode::encode_immediate_32bit;
    use crate::arch64::registers::ArmRegister;
    use crate::{
        arch64::ops::{Arm64NativeOp, ConditionCode, ImmI9_0, ImmI13_0, ImmI28_2},
        traits::NativeOp as _,
    };
    use riscv32::immediate::ImmU12_0;

    #[test]
    fn register_display() {
        assert_eq!(ArmRegister::X0.x(), "x0");
        assert_eq!(ArmRegister::Sp.x(), "sp");
        assert_eq!(ArmRegister::Zr.x(), "xzr");
    }

    #[test]
    fn instruction_to_asm() {
        let add: Arm64NativeOp =
            Arm64NativeOp::Adds_W_W_W(ArmRegister::X0, ArmRegister::X1, ArmRegister::X2);
        assert_eq!(add.as_assembly(), "ADDS w0, w1, w2");

        let nop: Arm64NativeOp = Arm64NativeOp::Nop;
        assert_eq!(nop.as_assembly(), "NOP");
    }

    #[test]
    fn condition_code_display() {
        assert_eq!(ConditionCode::Eq.to_string(), "EQ");
        assert_eq!(ConditionCode::Ne.to_string(), "NE");
    }

    // ── Immediate encode boundaries ───────────────────────────────────────────

    #[rstest]
    #[case(0)]
    #[case(1)]
    #[case(4095)]
    fn imm_u12_0_accepts_boundary_values(#[case] value: u16) {
        assert_that!(ImmU12_0::try_from(value), ok(anything()));
    }

    #[test]
    fn imm_u12_0_rejects_out_of_range() {
        assert!(ImmU12_0::try_from(4096).is_err());
    }

    #[test]
    fn imm_u12_0_encoded_value_equals_raw_value() {
        assert_eq!(encode_immediate_32bit(ImmU12_0::try_from(0).unwrap()), 0);
        assert_eq!(
            encode_immediate_32bit(ImmU12_0::try_from(100).unwrap()),
            100
        );
        assert_eq!(
            encode_immediate_32bit(ImmU12_0::try_from(4095).unwrap()),
            4095
        );
    }

    #[rstest]
    #[case(-256)]
    #[case(0)]
    #[case(255)]
    fn imm_i9_0_accepts_boundary_values(#[case] value: i16) {
        assert_that!(ImmI9_0::try_from(value), ok(anything()));
    }

    #[test]
    fn imm_i9_0_rejects_out_of_range() {
        assert!(ImmI9_0::try_from(-257).is_err());
        assert!(ImmI9_0::try_from(256).is_err());
    }

    #[test]
    fn imm_i9_0_negative_one_encodes_to_9_bit_all_ones() {
        // -1 as 9 bits = 0x1FF
        assert_eq!(
            encode_immediate_32bit(ImmI9_0::try_from(-1).unwrap()),
            0x1FF
        );
    }

    #[rstest]
    #[case(-4095)]
    #[case(0)]
    #[case(4095)]
    fn imm_i13_0_accepts_symmetric_range(#[case] value: i16) {
        assert_that!(ImmI13_0::try_from(value), ok(anything()));
    }

    #[test]
    fn imm_i13_0_rejects_out_of_symmetric_range() {
        assert!(ImmI13_0::try_from(-4096).is_err());
        assert!(ImmI13_0::try_from(4096).is_err());
    }

    // Must be a multiple of 4 (2 trailing zero bits).
    #[rstest]
    #[case(4)]
    #[case(-4)]
    #[case(0)]
    fn imm_i28_2_accepts_aligned_values(#[case] value: i32) {
        assert_that!(ImmI28_2::try_from(value), ok(anything()));
    }

    // Values that are not a multiple of 4 must be rejected.
    #[rstest]
    #[case(2)]
    #[case(1)]
    #[case(3)]
    fn imm_i28_2_rejects_misaligned_values(#[case] value: i32) {
        assert_that!(ImmI28_2::try_from(value), err(anything()));
    }

    // ── ConditionCode::inverted ───────────────────────────────────────────────

    #[test]
    fn condition_code_inverted_gives_logical_negation() {
        use ConditionCode::*;
        assert_that!(Eq.inverted(), pat!(Ne));
        assert_that!(Ne.inverted(), pat!(Eq));
        assert_that!(Hs.inverted(), pat!(Lo));
        assert_that!(Lo.inverted(), pat!(Hs));
        assert_that!(Mi.inverted(), pat!(Pl));
        assert_that!(Pl.inverted(), pat!(Mi));
        assert_that!(Hi.inverted(), pat!(Ls));
        assert_that!(Ls.inverted(), pat!(Hi));
        assert_that!(Ge.inverted(), pat!(Lt));
        assert_that!(Lt.inverted(), pat!(Ge));
        assert_that!(Gt.inverted(), pat!(Le));
        assert_that!(Le.inverted(), pat!(Gt));
    }

    #[test]
    fn condition_code_inverted_is_involutive() {
        use ConditionCode::*;
        for cc in [Eq, Ne, Hs, Lo, Mi, Pl, Hi, Ls, Ge, Lt, Gt, Le] {
            let restored = cc.inverted().inverted();
            assert_eq!(
                cc as u32, restored as u32,
                "{cc:?} double-invert should be identity"
            );
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "File operations are required for the assembler comparison tests."
)]
mod machine_code_tests {
    use crate::arch64::ops::{
        Arm64NativeOp, ConditionCode, ImmI9_0, ImmI9_2, ImmI12_0, ImmI13_0, ImmI21_0, ImmI21_2,
        ImmI28_2, ImmI33_12, ImmU5_4, ImmU6_0, ImmU14_2, ImmU15_3,
    };
    use crate::arch64::registers::ArmRegister;
    use crate::test_support::seeded_rng;
    use crate::traits::NativeOp as _;
    use rand::Rng;
    use riscv32::immediate::{ImmU5_0, ImmU12_0};
    use riscv32_elf::consts::TEXT_SECTION_NAME;
    use std::io::Write;
    use std::process::Command;

    /// Assembles multiple ARM64 instructions and returns the bytecode bytes.
    fn assemble_instructions(instructions: &[String]) -> Vec<[u8; 4]> {
        let temp_dir = tempfile::tempdir().expect("Failed to create a temporary directory");
        let asm_file = temp_dir.path().join("arm64_test.s");
        let obj_file = temp_dir.path().join("arm64_test.o");
        let bin_file = temp_dir.path().join("arm64_test.bin");

        // Write all instructions to a single file
        let mut file = std::fs::File::create(&asm_file).expect("Failed to create asm file");
        writeln!(file, "{TEXT_SECTION_NAME}").expect("Failed to write .text");
        for instr in instructions {
            writeln!(file, "{instr}").expect("Failed to write instruction");
        }
        drop(file);

        // Assemble
        let as_output = Command::new("aarch64-linux-gnu-as")
            .arg("-o")
            .arg(&obj_file)
            .arg(&asm_file)
            .output()
            .expect("Failed to run aarch64-linux-gnu-as. Is it installed?");

        if !as_output.status.success() {
            panic!(
                "Assembly failed: {}",
                String::from_utf8_lossy(&as_output.stderr)
            );
        }

        // Extract bytecode
        let objcopy_output = Command::new("aarch64-linux-gnu-objcopy")
            .args(["-O", "binary", "-j", TEXT_SECTION_NAME])
            .arg(&obj_file)
            .arg(&bin_file)
            .output()
            .expect("Failed to run aarch64-linux-gnu-objcopy");

        if !objcopy_output.status.success() {
            panic!(
                "objcopy failed: {}",
                String::from_utf8_lossy(&objcopy_output.stderr)
            );
        }

        let bytes = std::fs::read(&bin_file).expect("Failed to read binary");

        // Convert to 4-byte chunks
        bytes.as_chunks::<4>().0.to_vec()
    }

    /// Generates a random unsigned value with the given bit width and alignment.
    /// Range: [0, 2^bits) with alignment to 2^trailing_zeros.
    fn random_unsigned<T>(rng: &mut impl Rng, bits: u32, trailing_zeros: u32) -> T
    where
        T: TryFrom<u64>,
        <T as TryFrom<u64>>::Error: std::fmt::Debug,
    {
        let max_shifted = 1u64 << (bits - trailing_zeros);
        let shifted_val = rng.random_range(0..max_shifted);
        T::try_from(shifted_val << trailing_zeros).unwrap()
    }

    /// Generates a random signed value with the given bit width and alignment.
    /// Range: [-2^(bits-1), 2^(bits-1)) with alignment to 2^trailing_zeros.
    fn random_signed<T>(rng: &mut impl Rng, bits: u32, trailing_zeros: u32) -> T
    where
        T: TryFrom<i64>,
        <T as TryFrom<i64>>::Error: std::fmt::Debug,
    {
        let half = 1i64 << (bits - trailing_zeros - 1);
        let shifted_val = rng.random_range(-half..half);
        T::try_from(shifted_val << trailing_zeros).unwrap()
    }

    fn random_register(rng: &mut impl Rng) -> ArmRegister {
        let idx = rng.random_range(0..31);
        ArmRegister::from_repr(idx).unwrap_or_else(|| unreachable!())
    }

    /// Like random_register, but occasionally returns Zr (zero register).
    /// This helps catch encoding bugs where Zr (register 31) might be
    /// incorrectly encoded as a different register.
    fn random_register_including_zr(rng: &mut impl Rng) -> ArmRegister {
        // 10% chance of returning Zr
        if rng.random_range(0..10) == 0 {
            ArmRegister::Zr
        } else {
            random_register(rng)
        }
    }

    fn random_condition(rng: &mut impl Rng) -> ConditionCode {
        // Valid condition codes are 0-5 and 8-13 (12 values with a gap at 6-7)
        let idx: u32 = rng.random_range(0..12);
        let repr = if idx < 6 { idx } else { idx + 2 };
        ConditionCode::from_repr(repr).unwrap_or_else(|| unreachable!())
    }

    /// Generates test cases for all encodable Arm64Op variants.
    fn generate_test_cases(rng: &mut impl Rng) -> Vec<Arm64NativeOp> {
        let r1 = random_register(rng);
        let r2 = random_register(rng);
        let r3 = random_register(rng);
        // RiscvRegister that may include Zr to test zero register encoding
        let rz1 = random_register_including_zr(rng);
        let rz2 = random_register_including_zr(rng);
        let rz3 = random_register_including_zr(rng);
        let cond = random_condition(rng);

        vec![
            Arm64NativeOp::Nop,
            // Adds (rz variants may include Zr to test zero register encoding)
            Arm64NativeOp::Adds_W_W_Imm(r1, r2, ImmU12_0(random_unsigned(rng, 12, 0))),
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_Imm(ImmU12_0(random_unsigned(rng, 12, 0))),
            Arm64NativeOp::Adds_W_W_W(rz1, rz2, rz3),
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(r1),
            // Subs
            Arm64NativeOp::Subs_W_W_Imm(r1, r2, ImmU12_0(random_unsigned(rng, 12, 0))),
            Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(ImmU12_0(random_unsigned(rng, 12, 0))),
            Arm64NativeOp::Subs_xRegTmp_xRegTmp_Imm(ImmU12_0(random_unsigned(rng, 12, 0))),
            Arm64NativeOp::Subs_W_W_W(rz1, rz2, rz3),
            // Logic (rz variants may include Zr)
            Arm64NativeOp::Eor_W_W_W(rz1, rz2, rz3),
            Arm64NativeOp::Eon_W_W_W(rz1, rz2, rz3),
            Arm64NativeOp::Orr_W_W_W(rz1, rz2, rz3),
            Arm64NativeOp::Orn_W_W_W(rz1, rz2, rz3),
            Arm64NativeOp::And_W_W_W(rz1, rz2, rz3),
            // Bitmask with random valid pattern: consecutive 1-bits repeated across bytes
            Arm64NativeOp::Bic_W_W_3(r1, r2),
            Arm64NativeOp::Bic_W_W_W(r1, r2, r3),
            Arm64NativeOp::Tst_W_2(r1),
            // Shifts
            Arm64NativeOp::Lsr_W_W_Imm(r1, r2, ImmU5_0(random_unsigned(rng, 5, 0))),
            Arm64NativeOp::Lsr_X_xRegTmp_32(r1),
            Arm64NativeOp::Lsr_W_W_W(r1, r2, r3),
            Arm64NativeOp::Asr_W_W_Imm(r1, r2, ImmU5_0(random_unsigned(rng, 5, 0))),
            Arm64NativeOp::Asr_xRegTmp_xRegTmp_Imm(ImmU6_0(random_unsigned(rng, 6, 0))),
            Arm64NativeOp::Asr_W_W_W(r1, r2, r3),
            Arm64NativeOp::Lsl_W_W_Imm(r1, r2, ImmU5_0(random_unsigned(rng, 5, 0))),
            Arm64NativeOp::Lsl_xRegTmp_xRegTmp_Imm(ImmU6_0(random_unsigned(rng, 6, 0))),
            Arm64NativeOp::Lsl_W_W_W(r1, r2, r3),
            // Compare
            Arm64NativeOp::Cmp_W_Imm(r1, ImmI12_0(random_signed(rng, 12, 0))),
            Arm64NativeOp::Cmp_W_Imm(r1, ImmI12_0(random_signed(rng, 12, 0))),
            Arm64NativeOp::Cmp_W_W(r1, r2),
            Arm64NativeOp::Cset_W(r1, cond),
            Arm64NativeOp::Csel_wRegTmp2_wRegTmp2_wRegTmp(cond),
            // Move
            Arm64NativeOp::Movz_W_Imm_Shift(r1, rng.random::<u16>(), ImmU5_4(0)),
            Arm64NativeOp::Movz_W_Imm_Shift(r1, rng.random::<u16>(), ImmU5_4(16)),
            Arm64NativeOp::Movk_W_Imm_Shift(r1, rng.random::<u16>(), ImmU5_4(0)),
            // Mov with random value in low 12 bits (high bits zero - MOVZ encodable)
            Arm64NativeOp::Mov_W_Imm(r1, ImmI12_0(random_signed(rng, 12, 0))),
            Arm64NativeOp::Mov_W_W(rz1, rz2),
            // Multiply/Divide
            Arm64NativeOp::Mul_W_W_W(r1, r2, r3),
            Arm64NativeOp::Mul_xRegTmp_xRegTmp_xRegTmp2,
            Arm64NativeOp::Smull_xRegTmp_W_W(r1, r2),
            Arm64NativeOp::Umull_xRegTmp_W_W(r1, r2),
            Arm64NativeOp::Sxtw(r1, r2),
            Arm64NativeOp::Sdiv_W_W_W(r1, r2, r3),
            Arm64NativeOp::Udiv_W_W_W(r1, r2, r3),
            // Branches
            Arm64NativeOp::Cbz_W(r1, ImmI21_2(random_signed(rng, 21, 2))),
            Arm64NativeOp::Cbnz_W(r1, ImmI21_2(random_signed(rng, 21, 2))),
            Arm64NativeOp::B(ImmI28_2(random_signed(rng, 28, 2))),
            Arm64NativeOp::Bcc(ImmI21_2(random_signed(rng, 21, 2)), cond),
            Arm64NativeOp::Br_xRegTmp,
            // Loads
            Arm64NativeOp::LdrsbW_xRegTmp_0(r1),
            Arm64NativeOp::LdurshW_xRegTmp_0(r1),
            Arm64NativeOp::Ldur_W_xRegTmp_Imm(r1, ImmI9_0(random_signed(rng, 9, 0))),
            Arm64NativeOp::Ldur_X_xRegTmp_Imm(r1, ImmI9_0(random_signed(rng, 9, 0))),
            Arm64NativeOp::Ldr_W_xSp_Imm(r1, ImmU14_2(random_unsigned(rng, 14, 2))),
            Arm64NativeOp::Ldr_xRegTmp_X_Imm(r1, ImmU15_3(random_unsigned(rng, 15, 3))),
            Arm64NativeOp::LdrbW_xRegTmp_0(r1),
            Arm64NativeOp::LdurhW_xRegTmp_0(r1),
            // Stores
            Arm64NativeOp::Stur_W_xRegTmp_Imm(r1, ImmI9_0(random_signed(rng, 9, 0))),
            Arm64NativeOp::Stur_X_xRegTmp_Imm(r1, ImmI9_0(random_signed(rng, 9, 0))),
            Arm64NativeOp::Str_W_xSp_Imm(r1, ImmU14_2(random_unsigned(rng, 14, 2))),
            Arm64NativeOp::Str_xRegTmp_xSp_Imm(ImmU15_3(random_unsigned(rng, 15, 3))),
            Arm64NativeOp::Strb_W_xRegTmp_0(r1),
            Arm64NativeOp::Sturh_W_xRegTmp_0(r1),
            // Pair loads/stores
            Arm64NativeOp::Ldp_X_X_xSp_adv16(r1, r2),
            Arm64NativeOp::Stp_X_X_xSp_minus16(r1, r2),
            Arm64NativeOp::Ldp_W_W_xSp_Imm(r1, r2, ImmI9_2(random_signed(rng, 9, 2))),
            Arm64NativeOp::Stp_W_W_xSp_Imm(r1, r2, ImmI9_2(random_signed(rng, 9, 2))),
            // Stack (ImmI13_0 has symmetric range -4095..=4095)
            Arm64NativeOp::Add_xSp_Imm(ImmI13_0::new(rng.random_range(-4095..=4095))),
            Arm64NativeOp::Add_xSp_Imm(ImmI13_0::new(rng.random_range(-4095..=4095))),
            // Note: ADRP and ADD :lo12: are tested separately (they require symbol resolution
            // which the assembler can't do with raw relative offsets)
            // ADR with PC-relative offset (byte-aligned, ±1MB range)
            Arm64NativeOp::Adr_xRegTmp(ImmI21_0(random_signed(rng, 21, 0))),
            // Misc
            Arm64NativeOp::Ret_Lr,
            Arm64NativeOp::Subs_xRegGas_xRegGas_X(r1),
            Arm64NativeOp::Subs_xRegGas_xRegGas_Imm(ImmU12_0(random_unsigned(rng, 12, 0))),
        ]
    }

    #[test]
    fn all_arm64_opcodes_match_assembler() {
        let mut rng = seeded_rng("all_arm64_opcodes_match_assembler");
        let test_cases: Vec<Arm64NativeOp> = (0..500)
            .flat_map(|_| generate_test_cases(&mut rng))
            .collect();

        // Generate assembly strings and our bytecode
        let mut asm_lines: Vec<String> = Vec::new();
        let mut our_codes: Vec<Vec<u8>> = Vec::new();
        let mut descriptions: Vec<String> = Vec::new();

        for op in &test_cases {
            asm_lines.push(op.as_assembly());
            our_codes.push(op.arm64_encoding().to_le_bytes().to_vec());
            descriptions.push(format!("{op:?}"));
        }

        // Assemble all at once
        let assembler_codes = assemble_instructions(&asm_lines);

        // Compare and collect mismatches
        let mut mismatches: Vec<String> = Vec::new();
        for (i, ((ours, theirs), desc)) in our_codes
            .iter()
            .zip(assembler_codes.iter())
            .zip(descriptions.iter())
            .enumerate()
        {
            if ours != theirs {
                mismatches.push(format!(
                    "[{i}] {desc}\n    asm: {}\n    ours:   {ours:02x?}\n    theirs: {theirs:02x?}",
                    asm_lines[i]
                ));
            }
        }

        if !mismatches.is_empty() {
            panic!(
                "Bytecode mismatches ({} of {}):\n{}",
                mismatches.len(),
                our_codes.len(),
                mismatches.join("\n\n")
            );
        }
    }

    /// Assembles ADRP/ADD :lo12: with a symbol at a given offset and returns the bytecode.
    /// Returns (adrp_bytes, add_bytes).
    fn assemble_adrp_add_with_symbol(space_bytes: u32, symbol_before: bool) -> ([u8; 4], [u8; 4]) {
        let temp_dir = tempfile::tempdir().expect("Failed to create a temporary directory");
        let asm_file = temp_dir.path().join("adrp_test.s");
        let obj_file = temp_dir.path().join("adrp_test.o");
        let elf_file = temp_dir.path().join("adrp_test.elf");
        let bin_file = temp_dir.path().join("adrp_test.bin");

        // Generate assembly file - symbol and instructions with space between them
        let symbol_def = "symbol:\n.word 0\n";
        let instructions = "_start:\nADRP x17, symbol\nADD x17, x17, :lo12:symbol\n";
        let (first, second) = if symbol_before {
            (symbol_def, instructions)
        } else {
            (instructions, symbol_def)
        };
        let asm_content = if space_bytes > 0 {
            format!(".text\n.global _start\n{first}.space {space_bytes}\n{second}")
        } else {
            format!(".text\n.global _start\n{first}{second}")
        };

        std::fs::write(&asm_file, &asm_content).expect("Failed to write asm file");

        // Assemble
        let as_status = Command::new("aarch64-linux-gnu-as")
            .args(["-o", obj_file.to_str().unwrap(), asm_file.to_str().unwrap()])
            .status()
            .expect("Failed to run assembler");
        assert!(as_status.success(), "Assembler failed");

        // Link to resolve relocations (place .text at address 0)
        let ld_status = Command::new("aarch64-linux-gnu-ld")
            .args([
                "-Ttext=0",
                "-o",
                elf_file.to_str().unwrap(),
                obj_file.to_str().unwrap(),
            ])
            .status()
            .expect("Failed to run linker");
        assert!(ld_status.success(), "Linker failed");

        // Extract .text section from linked ELF
        let objcopy_status = Command::new("aarch64-linux-gnu-objcopy")
            .args([
                "-O",
                "binary",
                "-j",
                TEXT_SECTION_NAME,
                elf_file.to_str().unwrap(),
                bin_file.to_str().unwrap(),
            ])
            .status()
            .expect("Failed to run objcopy");
        assert!(objcopy_status.success(), "objcopy failed");

        // Read the binary
        let bin_data = std::fs::read(&bin_file).expect("Failed to read binary");

        // Extract ADRP and ADD instructions
        // When symbol_before: symbol (4 bytes) + space + alignment padding + instructions
        // When !symbol_before: instructions at offset 0
        let raw_offset = if symbol_before {
            4 + space_bytes as usize
        } else {
            0
        };
        let adrp_offset = (raw_offset + 3) & !3; // Round up to 4-byte alignment
        let add_offset = adrp_offset + 4;

        let mut adrp_bytes = [0u8; 4];
        let mut add_bytes = [0u8; 4];
        adrp_bytes.copy_from_slice(&bin_data[adrp_offset..adrp_offset + 4]);
        add_bytes.copy_from_slice(&bin_data[add_offset..add_offset + 4]);

        (adrp_bytes, add_bytes)
    }

    /// Tests ADRP and ADD :lo12: encodings against the system assembler.
    #[test]
    fn adrp_and_add_lo12_match_assembler() {
        let mut rng = seeded_rng("adrp_and_add_lo12_match_assembler");
        let mut mismatches: Vec<String> = Vec::new();

        // Test cases: (space_bytes, symbol_before)
        // Include small values, page boundary crossings, and random values
        let space_values = [0, 4, 100, 0xFFC, 0x1000, 0x1004, 0x2000, 0x3000];
        let mut test_cases: Vec<(u32, bool)> = space_values
            .iter()
            .flat_map(|&space| [(space, false), (space, true)])
            .collect();

        // Add random test cases
        for _ in 0..50 {
            // Random space within a few pages (up to 64KB)
            let space = rng.random_range(0..0x10000_u32);
            let symbol_before = rng.random::<bool>();
            test_cases.push((space, symbol_before));
        }

        for (space_bytes, symbol_before) in test_cases {
            let (asm_adrp, asm_add) = assemble_adrp_add_with_symbol(space_bytes, symbol_before);

            // Calculate the expected offsets for our encoding
            // When symbol_before: symbol at 0, ADRP at aligned(4 + space_bytes)
            // When !symbol_before: ADRP at 0, symbol at (8 + space_bytes)
            let raw_adrp_offset = if symbol_before { 4 + space_bytes } else { 0 };
            let adrp_pc = (raw_adrp_offset + 3) & !3; // Round up to 4-byte alignment
            let symbol_addr = if symbol_before { 0 } else { 8 + space_bytes };

            // ADRP byte offset (page-aligned)
            let adrp_page_addr = (adrp_pc & !0xFFF) as i64;
            let symbol_page_addr = (symbol_addr & !0xFFF) as i64;
            let byte_offset = symbol_page_addr - adrp_page_addr;

            // ADD lo12
            let lo12 = (symbol_addr & 0xFFF) as u16;

            // Generate our bytecode
            let our_adrp = Arm64NativeOp::Adrp_xRegTmp_Label(ImmI33_12(byte_offset))
                .arm64_encoding()
                .to_le_bytes();
            let our_add = Arm64NativeOp::Add_xRegTmp_xRegTmp_Lo12Label(ImmU12_0(lo12))
                .arm64_encoding()
                .to_le_bytes();

            if our_adrp != asm_adrp {
                mismatches.push(format!(
                    "ADRP mismatch: space={space_bytes}, symbol_before={symbol_before}, \
                     byte_offset={byte_offset}\n  ours:   {our_adrp:02x?}\n  theirs: {asm_adrp:02x?}"
                ));
            }

            if our_add != asm_add {
                mismatches.push(format!(
                    "ADD mismatch: space={space_bytes}, symbol_before={symbol_before}, \
                     lo12={lo12:#x}\n  ours:   {our_add:02x?}\n  theirs: {asm_add:02x?}"
                ));
            }
        }

        if !mismatches.is_empty() {
            panic!(
                "ADRP/ADD :lo12: mismatches ({}):\n{}",
                mismatches.len(),
                mismatches.join("\n\n")
            );
        }
    }
}
