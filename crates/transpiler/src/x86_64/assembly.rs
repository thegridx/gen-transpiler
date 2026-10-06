//! Renders `X86NativeOp`s as GNU assembler text in Intel syntax. It exists only as the oracle
//! for the encoder tests, which compare each op's encoding with the system assembler's output.
use riscv32::{
    common::SignType,
    ops::{LoadSize, StoreSize},
};

use super::registers::{X86_REG_GAS, X86Bitness, X86TempReg};
use crate::{traits::PcRelativeOffset, x86_64::ops::X86NativeOp};

fn x86_mem_loc_to_string(mem_loc: String, offset: i32, ptr_size: X86Bitness) -> String {
    format!(
        "{ptr_size} [{mem_loc}{}]",
        if offset == 0 {
            "".to_string()
        } else {
            format!("{offset:+}")
        }
    )
}

fn x86_pc_relative_memory_to_asm(offset: &PcRelativeOffset, ptr_size: X86Bitness) -> String {
    x86_mem_loc_to_string("rip".to_string(), offset.as_i32(), ptr_size)
}

fn x86_tmp_reg_memory_location_to_asm(tmp_reg: &X86TempReg, ptr_size: X86Bitness) -> String {
    x86_mem_loc_to_string(format!("{}", tmp_reg.q()), 0, ptr_size)
}

fn mul_div_sign_prefix(sign: &SignType) -> &'static str {
    if matches!(sign, SignType::Signed) {
        "i"
    } else {
        ""
    }
}

fn mov_extend_asm(sign: &SignType) -> &'static str {
    if matches!(sign, SignType::Signed) {
        "movsx"
    } else {
        "movzx"
    }
}

impl X86NativeOp {
    pub(crate) fn x86_as_asm_string(&self) -> String {
        match self {
            X86NativeOp::Lea_Tmp_Mem(tmp_reg, mem_loc) => {
                format!(
                    "lea {}, {}",
                    tmp_reg.q(),
                    x86_pc_relative_memory_to_asm(mem_loc, X86Bitness::Qword)
                )
            }
            X86NativeOp::Mov_Reg_Mem(dst, src) => {
                format!(
                    "mov {dst}, {}",
                    x86_pc_relative_memory_to_asm(
                        src,
                        if dst.is_64_bit() {
                            X86Bitness::Qword
                        } else {
                            X86Bitness::Dword
                        }
                    )
                )
            }
            X86NativeOp::Mov_Reg_MemTmp(dst, tmp_reg) => {
                format!(
                    "mov {dst}, {}",
                    x86_tmp_reg_memory_location_to_asm(tmp_reg, X86Bitness::Dword)
                )
            }
            X86NativeOp::Mov_Mem_Reg(dst, src) => {
                format!(
                    "mov {}, {src}",
                    x86_pc_relative_memory_to_asm(
                        dst,
                        if src.is_64_bit() {
                            X86Bitness::Qword
                        } else {
                            X86Bitness::Dword
                        }
                    )
                )
            }
            X86NativeOp::Mov_Mem_Imm(dst, src) => {
                format!(
                    "mov {}, {src}",
                    x86_pc_relative_memory_to_asm(dst, X86Bitness::Dword)
                ) // Always dword ptr
            }
            X86NativeOp::Mov_Reg_RoI(dst, src) => {
                format!("mov {dst}, {src}")
            }
            X86NativeOp::Add_Reg_RoI(dst, src) => {
                format!("add {dst}, {src}")
            }
            X86NativeOp::Sub_Reg_RoI(dst, src) => {
                format!("sub {dst}, {src}")
            }
            X86NativeOp::Neg_Reg(reg) => {
                format!("neg {reg}")
            }
            X86NativeOp::Mul_Reg_RoI(dst, src) => {
                format!("imul {dst}, {src}")
            }
            X86NativeOp::MulClobber(sign, src) => {
                format!("{}mul {src}", mul_div_sign_prefix(sign))
            }
            X86NativeOp::DivClobber(sign, src) => {
                format!("{}div {src}", mul_div_sign_prefix(sign))
            }
            X86NativeOp::Add_Tmp64_Reg64(dst, src) => {
                format!("add {}, {src}", dst.q())
            }
            X86NativeOp::And_Reg_RoI(dst, src) => {
                format!("and {dst}, {src}")
            }
            X86NativeOp::Or_Reg_RoI(dst, src) => {
                format!("or {dst}, {src}")
            }
            X86NativeOp::Xor_Reg_RoI(dst, src) => {
                format!("xor {dst}, {src}")
            }
            X86NativeOp::Xor_Self(reg) => {
                format!("xor {reg}, {reg}")
            }
            X86NativeOp::Shl_Reg_Sft(dst, amt) => {
                format!("shl {dst}, {amt}")
            }
            X86NativeOp::Shr_Reg_Sft(dst, amt) => {
                format!("shr {dst}, {amt}")
            }
            X86NativeOp::Sar_Reg_Sft(dst, amt) => {
                format!("sar {dst}, {amt}")
            }
            X86NativeOp::Cmp_Reg_RoI(dst, src) => {
                format!("cmp {dst}, {src}")
            }
            X86NativeOp::Test_Reg_RoI(reg, src) => {
                format!("test {reg}, {src}")
            }
            // Jumps are emitted as raw bytes, so the assembler cannot pick a shorter rel8 form.
            X86NativeOp::Jmp(offset) => {
                format!(".byte 0xE9 # jmp\n  .long {}", offset.as_i32())
            }
            X86NativeOp::Jcc(cond, offset) => {
                format!(
                    ".byte 0x0F, 0x{:02x} # j{cond}\n  .long {}",
                    cond.encoding(),
                    offset.as_i32()
                )
            }
            X86NativeOp::Jmp_Tmp(tmp_reg) => {
                format!("jmp {}", tmp_reg.q())
            }
            X86NativeOp::SetLow8(sign, dst) => match sign {
                SignType::Signed => format!("setl {dst}"),
                SignType::Unsigned => format!("setb {dst}"),
            },
            X86NativeOp::Movzx_Self(r) => {
                format!("movzx {r}, {}", r.b())
            }
            X86NativeOp::Movsxd_Tmp(tmp_reg) => {
                format!("movsxd {}, {}", tmp_reg.q(), tmp_reg.d())
            }
            X86NativeOp::Cdq => "cdq".to_string(),
            X86NativeOp::Load_Reg_MemTmp(size, dst, tmp_reg) => {
                let (mov_inst, ptr_type) = match size {
                    LoadSize::OneByte(sign) => (mov_extend_asm(sign), X86Bitness::Byte),
                    LoadSize::HalfWord(sign) => (mov_extend_asm(sign), X86Bitness::Word),
                    LoadSize::OneWord => ("mov", X86Bitness::Dword),
                };
                format!(
                    "{mov_inst} {dst}, {}",
                    x86_tmp_reg_memory_location_to_asm(tmp_reg, ptr_type)
                )
            }
            X86NativeOp::Store_MemTmp_Reg(size, dst, src) => {
                let (ptr_type, reg_str) = match size {
                    StoreSize::OneByte => (X86Bitness::Byte, src.b().to_string()),
                    StoreSize::HalfWord => (X86Bitness::Word, src.w().to_string()),
                    StoreSize::OneWord => (X86Bitness::Dword, src.to_string()),
                };
                format!(
                    "mov {}, {reg_str}",
                    x86_tmp_reg_memory_location_to_asm(dst, ptr_type)
                )
            }
            X86NativeOp::Push(src) => {
                format!("push {src}")
            }
            X86NativeOp::Pop(dst) => {
                format!("pop {dst}")
            }
            X86NativeOp::Sub_Gas_Imm(imm) => {
                format!("sub {X86_REG_GAS}, {imm}")
            }
            X86NativeOp::Sub_Gas_Tmp(tmp_reg) => {
                format!("sub {X86_REG_GAS}, {}", tmp_reg.q())
            }
            X86NativeOp::Ret() => "ret".to_string(),
        }
    }
}
