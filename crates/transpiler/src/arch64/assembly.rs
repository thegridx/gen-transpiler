//! Renders aarch64 ops as assembly text. Test-only: the encoder tests assemble this text with
//! `aarch64-linux-gnu-as` and compare the result with `arm64_encoding`.
use riscv32::immediate::Immediate;

use crate::arch64::{
    ops::{Arm64NativeOp, ImmI13_0},
    registers::{ArmRegister, REG_GAS, REG_TMP, REG_TMP_2},
};

fn pc_relative_offset_to_asm(off: i64) -> String {
    format!(".{}{}", if off >= 0 { "+" } else { "" }, off)
}

impl Arm64NativeOp {
    /// Returns the op in GNU assembler syntax.
    pub(crate) fn arm64_as_asm_string(&self) -> String {
        match self {
            Arm64NativeOp::Nop => "NOP".to_string(),
            Arm64NativeOp::Adrp_xRegTmp_Label(p) => {
                format!(
                    "ADRP {}, {}",
                    REG_TMP.x(),
                    pc_relative_offset_to_asm(p.value())
                )
            }
            Arm64NativeOp::Adr_xRegTmp(p) => {
                format!(
                    "ADR {}, {}",
                    REG_TMP.x(),
                    pc_relative_offset_to_asm(i64::from(p.value()))
                )
            }
            Arm64NativeOp::Add_xRegTmp_xRegTmp_Lo12Label(p) => {
                format!("ADD {}, {}, #{}", REG_TMP.x(), REG_TMP.x(), p.value())
            }
            Arm64NativeOp::Adds_W_W_Imm(w_d, w_1, imm) => {
                // w_1 is never wzr: Rn = 31 is WSP in this encoding
                format!("ADDS {w_d}, {w_1}, #{imm}")
            }
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_Imm(imm) => {
                format!("ADDS {}, {}, #{imm}", REG_TMP.x(), REG_TMP.x())
            }
            Arm64NativeOp::Adds_W_W_W(w_d, w_1, w_2) => {
                format!("ADDS {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Adds_xRegTmp_xRegTmp_w(w_2) => {
                format!("ADDS {}, {}, {w_2}, sxtw", REG_TMP.x(), REG_TMP.x(),)
            }
            Arm64NativeOp::Subs_W_W_Imm(w_d, w_1, imm) => {
                // w_1 is never wzr: Rn = 31 is WSP in this encoding
                format!("SUBS {w_d}, {w_1}, #{imm}")
            }
            Arm64NativeOp::Subs_wRegTmp_wRegTmp_Imm_Shift12(imm) => {
                format!("SUBS {REG_TMP}, {REG_TMP}, #{imm}, LSL #12")
            }
            Arm64NativeOp::Subs_xRegTmp_xRegTmp_Imm(imm) => {
                format!("SUBS {}, {}, #{imm}", REG_TMP.x(), REG_TMP.x())
            }
            Arm64NativeOp::Subs_W_W_W(w_d, w_1, w_2) => {
                format!("SUBS {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Eor_W_W_W(w_d, w_1, w_2) => {
                format!("EOR {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Eon_W_W_W(w_d, w_1, w_2) => {
                format!("EON {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Orr_W_W_W(w_d, w_1, w_2) => {
                format!("ORR {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Orn_W_W_W(w_d, w_1, w_2) => {
                format!("ORN {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::And_W_W_W(w_d, w_1, w_2) => {
                format!("AND {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Bic_W_W_3(w_d, w_1) => {
                format!("BIC {w_d}, {w_1}, #3")
            }
            Arm64NativeOp::Bic_W_W_W(w_d, w_1, w_2) => {
                format!("BIC {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Tst_W_2(w_1) => format!("TST {w_1}, #2"),
            Arm64NativeOp::Lsr_W_W_Imm(w_d, w_1, imm) => {
                format!("LSR {w_d}, {w_1}, #{imm}")
            }
            Arm64NativeOp::Lsr_X_xRegTmp_32(x_d) => {
                format!("LSR {}, {}, #32", x_d.x(), REG_TMP.x())
            }
            Arm64NativeOp::Lsr_W_W_W(w_d, w_1, w_2) => {
                format!("LSR {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Asr_W_W_Imm(w_d, w_1, imm) => {
                format!("ASR {w_d}, {w_1}, #{imm}")
            }
            Arm64NativeOp::Asr_xRegTmp_xRegTmp_Imm(imm) => {
                format!("ASR {}, {}, #{imm}", REG_TMP.x(), REG_TMP.x())
            }
            Arm64NativeOp::Asr_W_W_W(w_d, w_1, w_2) => {
                format!("ASR {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Lsl_W_W_Imm(w_d, w_1, imm) => {
                format!("LSL {w_d}, {w_1}, #{imm}")
            }
            Arm64NativeOp::Lsl_xRegTmp_xRegTmp_Imm(imm) => {
                format!("LSL {}, {}, #{imm}", REG_TMP.x(), REG_TMP.x())
            }
            Arm64NativeOp::Lsl_W_W_W(w_d, w_1, w_2) => {
                format!("LSL {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Cmp_W_Imm(w_1, imm) => format!("CMP {w_1}, #{imm}"),
            Arm64NativeOp::Cmp_W_W(w_1, w_2) => format!("CMP {w_1}, {w_2}"),
            Arm64NativeOp::Cset_W(w_d, cc) => format!("CSET {w_d}, {cc}"),
            Arm64NativeOp::Csel_wRegTmp2_wRegTmp2_wRegTmp(cc) => {
                format!("CSEL {REG_TMP_2}, {REG_TMP_2}, {REG_TMP}, {cc}",)
            }
            Arm64NativeOp::Movz_W_Imm_Shift(w_d, imm, shift) => {
                format!("MOVZ {w_d}, #{imm}, LSL #{shift}")
            }
            Arm64NativeOp::Movk_W_Imm_Shift(w_d, imm, shift) => {
                format!("MOVK {w_d}, #{imm}, LSL #{shift}")
            }
            Arm64NativeOp::Mov_W_Imm(w_d, imm) => format!("MOV {w_d}, #{imm}"),
            Arm64NativeOp::Mov_W_W(w_d, w_m) => format!("MOV {w_d}, {w_m}"),
            Arm64NativeOp::Mul_W_W_W(w_d, w_1, w_2) => {
                format!("MUL {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Mul_xRegTmp_xRegTmp_xRegTmp2 => {
                format!("MUL {}, {}, {}", REG_TMP.x(), REG_TMP.x(), REG_TMP_2.x())
            }
            Arm64NativeOp::Smull_xRegTmp_W_W(w_1, w_2) => {
                format!("SMULL {}, {w_1}, {w_2}", REG_TMP.x())
            }
            Arm64NativeOp::Umull_xRegTmp_W_W(w_1, w_2) => {
                format!("UMULL {}, {w_1}, {w_2}", REG_TMP.x())
            }
            Arm64NativeOp::Sxtw(x_d, w_s) => format!("SXTW {}, {w_s}", x_d.x()),
            Arm64NativeOp::Sdiv_W_W_W(w_d, w_1, w_2) => {
                format!("SDIV {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Udiv_W_W_W(w_d, w_1, w_2) => {
                format!("UDIV {w_d}, {w_1}, {w_2}")
            }
            Arm64NativeOp::Cbz_W(w_1, p) => {
                format!(
                    "CBZ {w_1}, {}",
                    pc_relative_offset_to_asm(i64::from(p.value()))
                )
            }
            Arm64NativeOp::Cbnz_W(w_1, p) => {
                format!(
                    "CBNZ {w_1}, {}",
                    pc_relative_offset_to_asm(i64::from(p.value()))
                )
            }
            Arm64NativeOp::B(p) => {
                format!("B {}", pc_relative_offset_to_asm(i64::from(p.value())))
            }
            Arm64NativeOp::Bcc(p, cc) => {
                format!("B.{cc} {}", pc_relative_offset_to_asm(i64::from(p.value())))
            }
            Arm64NativeOp::Br_xRegTmp => format!("BR {}", REG_TMP.x()),
            Arm64NativeOp::LdrsbW_xRegTmp_0(w_d) => {
                format!("LDRSB {w_d}, [{}, #0]", REG_TMP.x())
            }
            Arm64NativeOp::LdurshW_xRegTmp_0(w_d) => {
                format!("LDURSH {w_d}, [{}, #0]", REG_TMP.x())
            }
            Arm64NativeOp::Ldur_W_xRegTmp_Imm(w_d, imm) => {
                format!("LDUR {w_d}, [{}, #{imm}]", REG_TMP.x())
            }
            Arm64NativeOp::Ldur_X_xRegTmp_Imm(x_d, imm) => {
                format!("LDUR {}, [{}, #{imm}]", x_d.x(), REG_TMP.x())
            }
            Arm64NativeOp::Ldr_W_xSp_Imm(w_d, imm) => {
                format!("LDR {w_d}, [sp, #{imm}]")
            }
            Arm64NativeOp::Ldr_xRegTmp_X_Imm(x_2, imm) => {
                format!("LDR {}, [{}, #{imm}]", REG_TMP.x(), x_2.x())
            }
            Arm64NativeOp::LdrbW_xRegTmp_0(w_d) => {
                format!("LDRB {w_d}, [{}, #0]", REG_TMP.x())
            }
            Arm64NativeOp::LdurhW_xRegTmp_0(w_d) => {
                format!("LDURH {w_d}, [{}, #0]", REG_TMP.x())
            }
            Arm64NativeOp::Stur_W_xRegTmp_Imm(w_s, imm) => {
                format!("STUR {w_s}, [{}, #{imm}]", REG_TMP.x())
            }
            Arm64NativeOp::Stur_X_xRegTmp_Imm(x_s, imm) => {
                format!("STUR {}, [{}, #{imm}]", x_s.x(), REG_TMP.x())
            }
            Arm64NativeOp::Str_W_xSp_Imm(w_s, imm) => {
                format!("STR {w_s}, [sp, #{imm}]")
            }
            Arm64NativeOp::Str_xRegTmp_xSp_Imm(imm) => {
                format!("STR {}, [sp, #{imm}]", REG_TMP.x())
            }
            Arm64NativeOp::Strb_W_xRegTmp_0(w_s) => {
                format!("STRB {w_s}, [{}, #0]", REG_TMP.x())
            }
            Arm64NativeOp::Sturh_W_xRegTmp_0(w_s) => {
                format!("STURH {w_s}, [{}, #0]", REG_TMP.x())
            }
            Arm64NativeOp::Ldp_X_X_xSp_adv16(x_d1, x_d2) => {
                format!("LDP {}, {}, [sp], #16", x_d1.x(), x_d2.x())
            }
            Arm64NativeOp::Stp_X_X_xSp_minus16(x_s1, x_s2) => {
                format!("STP {}, {}, [sp, #-16]!", x_s1.x(), x_s2.x())
            }
            Arm64NativeOp::Ldp_W_W_xSp_Imm(w_d1, w_d2, imm) => {
                format!("LDP {w_d1}, {w_d2}, [sp, #{imm}]")
            }
            Arm64NativeOp::Stp_W_W_xSp_Imm(w_s1, w_s2, imm) => {
                format!("STP {w_s1}, {w_s2}, [sp, #{imm}]")
            }
            Arm64NativeOp::Add_xSp_Imm(imm) => {
                if imm.value() >= 0 {
                    format!("ADD sp, sp, #{imm}")
                } else {
                    // The range of `ImmI13_0` is symmetric, so the negation fits.
                    format!("SUB sp, sp, #{}", ImmI13_0::new(-imm.value()))
                }
            }
            Arm64NativeOp::Ret_Lr => format!("RET {}", ArmRegister::Lr.x()),
            Arm64NativeOp::Subs_xRegGas_xRegGas_X(x) => {
                format!("SUBS {}, {}, {}", REG_GAS.x(), REG_GAS.x(), x.x())
            }
            Arm64NativeOp::Subs_xRegGas_xRegGas_Imm(imm) => {
                format!("SUBS {}, {}, #{imm}", REG_GAS.x(), REG_GAS.x())
            }
        }
    }
}
