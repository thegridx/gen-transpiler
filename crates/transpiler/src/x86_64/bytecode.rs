//! Bytecode encoder for `X86NativeOp`. The same encoder also measures each op's size,
//! because x86_64 instructions are variable-length.
use riscv32::{
    common::SignType,
    ops::{LoadSize, StoreSize},
};

use super::registers::{REG_EAX, X86_REG_GAS, X86Reg32Bit, X86Reg32Or64, X86Reg64Bit};
use crate::{
    traits::PcRelativeOffset,
    x86_64::{
        ops::{X86Immediate, X86NativeOp, X86Reg32OrImm, X86ShiftAmount},
        registers::X86Register,
    },
};

/// A sink for encoded x86 bytecode bytes. Abstracts over a heap-backed `Vec<u8>`
/// (used when emitting real code via `encode`) and a zero-storage byte counter
/// ([`ByteCounter`], used by `size_in_bytes`, which only needs the resulting byte count).
/// This lets encoding measure an instruction's length without a per-call heap allocation.
pub(crate) trait ByteSink {
    fn push(&mut self, byte: u8);
    fn extend_from_slice(&mut self, bytes: &[u8]);
    /// Push an optional prefix byte (e.g. an optional REX prefix), when present.
    fn extend_opt(&mut self, byte: Option<u8>) {
        if let Some(byte_val) = byte {
            self.push(byte_val);
        }
    }
}

impl ByteSink for Vec<u8> {
    fn push(&mut self, byte: u8) {
        Vec::push(self, byte);
    }
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        Vec::extend_from_slice(self, bytes);
    }
}

/// A [`ByteSink`] that discards the bytes and only tracks how many were written.
/// Used by `size_in_bytes` to measure an op's encoded length without allocating or
/// materializing the bytes.
#[derive(Default)]
pub(crate) struct ByteCounter(u32);

impl ByteCounter {
    pub(crate) fn count(&self) -> u32 {
        self.0
    }
}

impl ByteSink for ByteCounter {
    fn push(&mut self, _byte: u8) {
        self.0 = self.0.saturating_add(1);
    }
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        self.0 = self.0.saturating_add(bytes.len() as u32);
    }
}

/// Encodes `OP r32, r/imm` for the standard ALU instructions.
/// `ext` is the opcode extension: ADD=0, OR=1, AND=4, SUB=5, XOR=6, CMP=7.
/// Reg-reg form:  [REX] (ext<<3|0x01) ModRM           (Reg=src, RM=dst)
/// Imm8 form:     [REX] 0x83 ModRM imm8               (RM=dst, /ext in Reg)
/// Imm32 form:    [REX] 0x81 ModRM imm32_le, or EAX short form (ext<<3|0x05) imm32_le
fn encode_alu_reg_roi<S: ByteSink>(out: &mut S, ext: u8, dst: &X86Reg32Bit, src: &X86Reg32OrImm) {
    let hw_dst = dst.register().hw_enc();
    match src {
        X86Reg32OrImm::Reg(src_reg) => {
            let hw_src = src_reg.register().hw_enc();
            let opcode = (ext << 3) | 0x01;
            let modrm = 0xC0 | ((hw_src & 7) << 3) | (hw_dst & 7);
            let rex_r = src_reg.register().is_rex(); // REX.R extends ModRM.Reg (src)
            let rex_b = dst.register().is_rex(); // REX.B extends ModRM.RM (dst)
            if rex_r || rex_b {
                out.extend_from_slice(&[
                    0x40 | (u8::from(rex_r) << 2) | u8::from(rex_b),
                    opcode,
                    modrm,
                ]);
            } else {
                out.extend_from_slice(&[opcode, modrm]);
            }
        }
        X86Reg32OrImm::Imm(X86Immediate::Imm8(imm8)) => {
            let modrm = 0xC0 | (ext << 3) | (hw_dst & 7);
            if dst.register().is_rex() {
                out.extend_from_slice(&[0x41, 0x83, modrm, imm8.get() as u8]); // REX.B=1
            } else {
                out.extend_from_slice(&[0x83, modrm, imm8.get() as u8]);
            }
        }
        X86Reg32OrImm::Imm(X86Immediate::Imm32(imm32)) => {
            let [d0, d1, d2, d3] = imm32.get().to_le_bytes();
            if dst == &REG_EAX {
                // EAX short form: (ext<<3|0x05) + imm32, no ModRM (5 bytes)
                out.extend_from_slice(&[(ext << 3) | 0x05, d0, d1, d2, d3]);
            } else {
                let modrm = 0xC0 | (ext << 3) | (hw_dst & 7);
                if dst.register().is_rex() {
                    out.extend_from_slice(&[0x41, 0x81, modrm, d0, d1, d2, d3]); // REX.B=1
                } else {
                    out.extend_from_slice(&[0x81, modrm, d0, d1, d2, d3]);
                }
            }
        }
    }
}

/// Encodes `shl/shr/sar r32, 1|cl|imm8` given the opcode extension:
/// SHL=/4, SHR=/5, SAR=/7. Opcode: D1(by 1), D3(by CL), C1(by imm8).
fn encode_shift<S: ByteSink>(out: &mut S, ext: u8, dst: &X86Reg32Bit, shift: &X86ShiftAmount) {
    let hw = dst.register().hw_enc();
    let modrm = 0xC0 | (ext << 3) | (hw & 7);
    let rex: Option<u8> = dst.register().is_rex().then_some(0x41); // REX.B for R8D-R15D
    out.extend_opt(rex);
    match shift {
        X86ShiftAmount::Imm(imm) if imm.get() == 1 => out.extend_from_slice(&[0xD1, modrm]),
        X86ShiftAmount::Imm(imm) => out.extend_from_slice(&[0xC1, modrm, imm.get() as u8]),
        X86ShiftAmount::Cl => out.extend_from_slice(&[0xD3, modrm]),
    }
}

/// Encodes `MOV r, [mem]` (load, opcode 0x8B) or `MOV [mem], r` (store, opcode 0x89) using
/// RIP-relative addressing. `hw_reg` is the hardware register number of the register operand
/// (in ModRM.Reg). `rex` is the optional REX prefix byte (already computed by the caller).
/// Encoding: `[REX] opcode ModRM(Mod=00, Reg=hw_reg&7, RM=5) disp32`.
fn encode_mov_reg_mem<S: ByteSink>(
    out: &mut S,
    reg: X86Reg32Or64,
    opcode: u8,
    offset: &PcRelativeOffset,
) {
    let hw_reg = match reg {
        X86Reg32Or64::Reg32(r) => r.register().hw_enc(),
        X86Reg32Or64::Reg64(r) => r.register().hw_enc(),
    };
    let rex_w = reg.is_64_bit();
    let rex_r = reg.is_rex(); // REX.R extends ModRM.Reg
    let rex: Option<u8> =
        (rex_w || rex_r).then_some(0x40 | (u8::from(rex_w) << 3) | (u8::from(rex_r) << 2));
    let modrm = ((hw_reg & 7) << 3) | 5;
    let [d0, d1, d2, d3] = offset.as_i32().to_le_bytes();
    out.extend_opt(rex);
    out.extend_from_slice(&[opcode, modrm, d0, d1, d2, d3]);
}

fn encode_f7_ext<S: ByteSink>(out: &mut S, ext: u8, r: &X86Reg32Bit) {
    let hw = r.register().hw_enc();
    let modrm = 0xC0 | (ext << 3) | (hw & 7);
    if r.register().is_rex() {
        out.extend_from_slice(&[0x41, 0xF7, modrm]);
    } else {
        out.extend_from_slice(&[0xF7, modrm]);
    }
}

fn encode_push_pop<S: ByteSink>(out: &mut S, base: u8, r: &X86Reg64Bit) {
    let hw = r.hw_enc();
    if hw < 8 {
        out.push(base.wrapping_add(hw));
    } else {
        out.extend_from_slice(&[0x41, base.wrapping_add(hw & 7)]);
    }
}

impl X86NativeOp {
    pub(crate) fn x86_encode<S: ByteSink>(&self, out: &mut S) {
        // REX prefix for an 8-bit register operand: `opt1` for r8b-r15b, `opt2` (a REX that
        // selects sil, dil or bpl instead of dh, bh or ch) for esi, edi and ebp, none otherwise.
        let get_reg_rex_esi_edi = |r: &X86Register, opt1, opt2| -> Option<u8> {
            if r.is_rex() {
                Some(opt1)
            } else if r.is_esi_edi_ebp() {
                Some(opt2)
            } else {
                None
            }
        };

        match self {
            X86NativeOp::Lea_Tmp_Mem(tmp_reg, offset) => {
                // LEA r64, [RIP + disp32]
                // Encoding: REX.W[.R] | 0x8D | ModRM(Mod=00, Reg=dst, RM=101) | disp32_le
                // RM=101 with Mod=00 selects RIP-relative addressing in 64-bit mode.
                let hw_reg = tmp_reg.hw_enc();
                // REX.W=1 (64-bit operand), REX.R=1 if destination register is R8-R15
                let rex = 0x48u8 | ((hw_reg >> 3) << 2);
                // ModRM: Mod=00, Reg=lower 3 bits of hw_reg, RM=101 (RIP-relative)
                let modrm = ((hw_reg & 7) << 3) | 0x05;
                let [d0, d1, d2, d3] = offset.as_i32().to_le_bytes();
                out.extend_from_slice(&[rex, 0x8D, modrm, d0, d1, d2, d3]);
            }
            X86NativeOp::Mov_Reg_Mem(reg, offset) => {
                // MOV r32/r64, [mem]: load. Opcode 0x8B, Reg=dst in ModRM.
                encode_mov_reg_mem(out, *reg, 0x8B, offset);
            }
            X86NativeOp::Mov_Reg_MemTmp(reg, tmp_reg) => {
                let hw_reg = reg.register().hw_enc();
                let rex_r = reg.register().is_rex();
                let rex: Option<u8> = rex_r.then_some(0x40 | (u8::from(rex_r) << 2));
                let hw_base = tmp_reg.hw_enc();
                // Mod=00: no displacement
                let modrm = ((hw_reg & 7) << 3) | hw_base;
                out.extend_opt(rex);
                out.extend_from_slice(&[0x8B, modrm]);
            }
            X86NativeOp::Mov_Mem_Reg(offset, reg) => {
                // MOV [mem], r32/r64: store. Opcode 0x89, Reg=src in ModRM.
                encode_mov_reg_mem(out, *reg, 0x89, offset);
            }
            X86NativeOp::Mov_Mem_Imm(offset, imm) => {
                // MOV [mem], imm32: store immediate (C7 /0), always 32-bit, no REX.
                // C7 05 disp32 imm32 (Mod=00, /0, RM=5 = RIP-relative): 10 bytes
                let [i0, i1, i2, i3] = imm.as_i32().to_le_bytes();
                let [d0, d1, d2, d3] = offset.as_i32().to_le_bytes();
                out.extend_from_slice(&[0xC7, 0x05, d0, d1, d2, d3, i0, i1, i2, i3]);
            }
            X86NativeOp::Mov_Reg_RoI(dst, src) => {
                let hw_dst = dst.register().hw_enc();
                match src {
                    X86Reg32OrImm::Reg(src_reg) => {
                        // MOV r/m32, r32: [REX] 0x89 /r (Reg=src, RM=dst), the form GNU as picks
                        let hw_src = src_reg.register().hw_enc();
                        let modrm = 0xC0 | ((hw_src & 7) << 3) | (hw_dst & 7);
                        let rex_r = src_reg.register().is_rex(); // REX.R extends ModRM.Reg (src)
                        let rex_b = dst.register().is_rex(); // REX.B extends ModRM.RM (dst)
                        let rex = if rex_r || rex_b {
                            Some(0x40 | (u8::from(rex_r) << 2) | u8::from(rex_b))
                        } else {
                            None
                        };
                        out.extend_opt(rex);
                        out.extend_from_slice(&[0x89, modrm]);
                    }
                    X86Reg32OrImm::Imm(imm) => {
                        // MOV r32, imm32: [REX.B] (0xB8+rd) id, short form with no ModRM.
                        let [d0, d1, d2, d3] = imm.as_i32().to_le_bytes();
                        if dst.register().is_rex() {
                            out.extend_from_slice(&[
                                0x41,
                                0xB8u8.wrapping_add(hw_dst & 7),
                                d0,
                                d1,
                                d2,
                                d3,
                            ]);
                        } else {
                            out.extend_from_slice(&[0xB8u8.wrapping_add(hw_dst), d0, d1, d2, d3]);
                        }
                    }
                }
            }
            X86NativeOp::Add_Reg_RoI(dst, src) => encode_alu_reg_roi(out, 0, dst, src),
            X86NativeOp::Or_Reg_RoI(dst, src) => encode_alu_reg_roi(out, 1, dst, src),
            X86NativeOp::And_Reg_RoI(dst, src) => encode_alu_reg_roi(out, 4, dst, src),
            X86NativeOp::Sub_Reg_RoI(dst, src) => encode_alu_reg_roi(out, 5, dst, src),
            X86NativeOp::Xor_Reg_RoI(dst, src) => encode_alu_reg_roi(out, 6, dst, src),
            X86NativeOp::Cmp_Reg_RoI(dst, src) => encode_alu_reg_roi(out, 7, dst, src),
            X86NativeOp::Add_Tmp64_Reg64(tmp_reg, r) => {
                // add r64, r64: REX.W[.R] 0x01 ModRM (always 3 bytes)
                // Encoding: ADD r/m64, r64 — Reg=src(r), RM=dst(tmp)
                let hw_tmp = tmp_reg.hw_enc();
                let hw_r = r.hw_enc();
                let modrm = 0xC0 | ((hw_r & 7) << 3) | hw_tmp;
                // REX.W=1 always; REX.R=1 if r is extended (R8-R15)
                let rex = 0x48 | (u8::from(r.register().is_rex()) << 2);
                out.extend_from_slice(&[rex, 0x01, modrm]);
            }
            X86NativeOp::Xor_Self(dst) => {
                // xor r, r (zero-idiom): 0x31 /r. Reg and RM are the same register.
                let hw = match dst {
                    X86Reg32Or64::Reg32(r) => r.register().hw_enc(),
                    X86Reg32Or64::Reg64(r) => r.register().hw_enc(),
                };
                let modrm = 0xC0 | ((hw & 7) << 3) | (hw & 7);
                let rex_rb = hw >= 8; // both REX.R and REX.B when same extended reg in Reg+RM
                let rex_w = dst.is_64_bit();
                let rex = if rex_w || rex_rb {
                    Some(0x40 | (u8::from(rex_w) << 3) | (u8::from(rex_rb) << 2) | u8::from(rex_rb))
                } else {
                    None
                };
                out.extend_opt(rex);
                out.extend_from_slice(&[0x31, modrm]);
            }
            X86NativeOp::Test_Reg_RoI(reg, roi) => {
                let hw_reg = reg.register().hw_enc();
                match roi {
                    X86Reg32OrImm::Reg(roi_reg) => {
                        // test r/m32, r32: [REX] 0x85 ModRM
                        // roi_reg goes in ModRM.Reg, reg goes in ModRM.RM
                        let hw_roi = roi_reg.register().hw_enc();
                        let modrm = 0xC0 | ((hw_roi & 7) << 3) | (hw_reg & 7);
                        let rex_r = roi_reg.register().is_rex(); // REX.R extends ModRM.Reg (roi_reg)
                        let rex_b = reg.register().is_rex(); // REX.B extends ModRM.RM (reg)
                        let rex = if rex_r || rex_b {
                            Some(0x40 | (u8::from(rex_r) << 2) | u8::from(rex_b))
                        } else {
                            None
                        };
                        out.extend_opt(rex);
                        out.extend_from_slice(&[0x85, modrm]);
                    }
                    X86Reg32OrImm::Imm(imm) => {
                        // test r/m32, imm32: [REX] 0xF7 /0 imm32
                        // No imm8 form for TEST; sign-extend Imm8 to 32 bits.
                        let imm32 = match imm {
                            X86Immediate::Imm8(i) => i.get() as i32,
                            X86Immediate::Imm32(i) => i.get(),
                        };
                        let [d0, d1, d2, d3] = imm32.to_le_bytes();
                        if reg == &REG_EAX {
                            // EAX short form: 0xA9 imm32 (5 bytes, no REX)
                            out.extend_from_slice(&[0xA9, d0, d1, d2, d3]);
                        } else {
                            let modrm = 0xC0 | (hw_reg & 7); // /0: ModRM.Reg = 0
                            if reg.register().is_rex() {
                                out.extend_from_slice(&[0x41, 0xF7, modrm, d0, d1, d2, d3]);
                            } else {
                                out.extend_from_slice(&[0xF7, modrm, d0, d1, d2, d3]);
                            }
                        }
                    }
                }
            }
            X86NativeOp::Mul_Reg_RoI(dst, src) => {
                let hw_dst = dst.register().hw_enc();
                match src {
                    X86Reg32OrImm::Reg(src_reg) => {
                        // imul r32, r32: [REX] 0F AF /r
                        let hw_src = src_reg.register().hw_enc();
                        let modrm = 0xC0 | ((hw_dst & 7) << 3) | (hw_src & 7);
                        let rex_r = dst.register().is_rex();
                        let rex_b = src_reg.register().is_rex();
                        let rex = if rex_r || rex_b {
                            Some(0x40 | (u8::from(rex_r) << 2) | u8::from(rex_b))
                        } else {
                            None
                        };
                        out.extend_opt(rex);
                        out.extend_from_slice(&[0x0F, 0xAF, modrm]);
                    }
                    X86Reg32OrImm::Imm(imm) => {
                        // imul r32, imm: dst is also the r/m operand, so REX.R=REX.B if extended
                        let modrm = 0xC0 | ((hw_dst & 7) << 3) | (hw_dst & 7);
                        let rex: Option<u8> = dst.register().is_rex().then_some(0x45); // REX.R+REX.B
                        out.extend_opt(rex);
                        match imm {
                            // imul r32, r32, imm8: [REX] 6B /r ib
                            X86Immediate::Imm8(imm8) => {
                                out.extend_from_slice(&[0x6B, modrm, imm8.get() as u8]);
                            }
                            // imul r32, r32, imm32: [REX] 69 /r id
                            X86Immediate::Imm32(imm32) => {
                                let [d0, d1, d2, d3] = imm32.get().to_le_bytes();
                                out.extend_from_slice(&[0x69, modrm, d0, d1, d2, d3]);
                            }
                        }
                    }
                }
            }
            X86NativeOp::MulClobber(sign, r) => {
                // mul/imul r32 (clobbers edx:eax): 0xF7 /4 unsigned, /5 signed
                encode_f7_ext(
                    out,
                    if matches!(sign, SignType::Unsigned) {
                        4
                    } else {
                        5
                    },
                    r,
                );
            }
            X86NativeOp::DivClobber(sign, r) => {
                // div/idiv r32 (clobbers edx:eax): 0xF7 /6 unsigned, /7 signed
                encode_f7_ext(
                    out,
                    if matches!(sign, SignType::Unsigned) {
                        6
                    } else {
                        7
                    },
                    r,
                );
            }
            X86NativeOp::Shl_Reg_Sft(dst, shift) => encode_shift(out, 4, dst, shift), // /4
            X86NativeOp::Shr_Reg_Sft(dst, shift) => encode_shift(out, 5, dst, shift), // /5
            X86NativeOp::Sar_Reg_Sft(dst, shift) => encode_shift(out, 7, dst, shift), // /7
            X86NativeOp::Neg_Reg(r) => {
                // neg r32: 0xF7 /3
                encode_f7_ext(out, 3, r);
            }
            X86NativeOp::Jmp(offset) => {
                // Near JMP: 0xE9 + imm32_le (5 bytes)
                let [d0, d1, d2, d3] = offset.as_i32().to_le_bytes();
                out.extend_from_slice(&[0xE9, d0, d1, d2, d3]);
            }
            X86NativeOp::Jcc(cond, offset) => {
                // Near Jcc: 0x0F op2 imm32_le (6 bytes).
                let cond_byte = cond.encoding();
                let [d0, d1, d2, d3] = offset.as_i32().to_le_bytes();
                out.extend_from_slice(&[0x0F, cond_byte, d0, d1, d2, d3]);
            }
            X86NativeOp::Jmp_Tmp(tmp_reg) => {
                // jmp r64: 0xFF /4. ModRM: Mod=11, Reg=4(/4), RM=hw(tmp).
                // RAX=hw0, RBX=hw3 — both < 8, no REX needed.
                out.extend_from_slice(&[0xFF, 0xC0 | (4 << 3) | tmp_reg.hw_enc()]);
            }
            X86NativeOp::SetLow8(sign, r) => {
                // setl r8 (signed) or setb r8 (unsigned): 0F 9C/92 /0
                // ModRM: Mod=11, Reg=0(/0), RM=hw&7.
                // REX.B=0x41 for R8-R15; REX marker 0x40 for sil/dil/bpl.
                let hw = r.register().hw_enc();
                let modrm = 0xC0 | (hw & 7);
                let op2: u8 = match sign {
                    SignType::Signed => 0x9C,   // setl
                    SignType::Unsigned => 0x92, // setb
                };
                let rex = get_reg_rex_esi_edi(&r.register(), 0x41, 0x40);
                out.extend_opt(rex);
                out.extend_from_slice(&[0x0F, op2, modrm]);
            }
            X86NativeOp::Movzx_Self(r) => {
                // movzx r32, r8 (same register, zero-extend byte to dword)
                // Opcode: 0F B6 /r
                // REX.R and REX.B both needed for R8D-R15D (same reg in Reg and RM fields).
                // REX marker (0x40) needed for RSI/RDI/RBP to access sil/dil/bpl vs ah/ch/dh/bh.
                let hw = r.register().hw_enc();
                let modrm = 0xC0 | ((hw & 7) << 3) | (hw & 7);
                let rex = get_reg_rex_esi_edi(&r.register(), 0x45, 0x40);
                out.extend_opt(rex);
                out.extend_from_slice(&[0x0F, 0xB6, modrm]);
            }
            X86NativeOp::Movsxd_Tmp(tmp_reg) => {
                // movsxd r64, r32 (sign-extend 32-bit to 64-bit, same register)
                // REX.W | 0x63 | ModRM(Mod=11, Reg=hw, RM=hw)
                let hw = tmp_reg.hw_enc();
                let modrm = 0xC0 | (hw << 3) | hw;
                out.extend_from_slice(&[0x48, 0x63, modrm]);
            }
            X86NativeOp::Cdq => out.push(0x99),
            X86NativeOp::Load_Reg_MemTmp(size, dst, src) => {
                // Load from [src_tmp + 0] into dst r32 with sign/zero extension.
                // ModRM: Mod=00 (indirect, no disp), Reg=hw(dst)&7, RM=hw(src_tmp).
                // RAX=0 and RBX=3 are safe as RM: neither triggers SIB(4) or RIP-rel(5).
                let hw_dst = dst.register().hw_enc();
                let hw_src = src.hw_enc();
                let modrm = ((hw_dst & 7) << 3) | hw_src;
                // REX.R=1 only when dst is R8D-R15D (extends ModRM.Reg)
                let rex: Option<u8> = dst.register().is_rex().then_some(0x44);
                out.extend_opt(rex);
                match size {
                    LoadSize::OneByte(sign) | LoadSize::HalfWord(sign) => {
                        // Byte: 0xBE (sx) / 0xB6 (zx); HalfWord: 0xBF (sx) / 0xB7 (zx)
                        let base = if matches!(size, LoadSize::HalfWord(_)) {
                            0xB7u8
                        } else {
                            0xB6u8
                        };
                        // Signed variants set bit 3 of the opcode (0xB6→0xBE, 0xB7→0xBF).
                        let op2 = if matches!(sign, SignType::Signed) {
                            base | 0x08
                        } else {
                            base
                        };
                        out.extend_from_slice(&[0x0F, op2, modrm]);
                    }
                    LoadSize::OneWord => out.extend_from_slice(&[0x8B, modrm]),
                }
            }
            X86NativeOp::Store_MemTmp_Reg(size, dst, src) => {
                // Store from src r8/r16/r32 to [dst_tmp + 0].
                // ModRM: Mod=00, Reg=hw(src)&7, RM=hw(dst_tmp).
                let hw_src = src.register().hw_enc();
                let hw_dst = dst.hw_enc();
                let modrm = ((hw_src & 7) << 3) | hw_dst;
                match size {
                    StoreSize::OneByte => {
                        // mov byte ptr [tmp], r8: 0x88 /r
                        // REX.R=1 for R8B-R15B; REX marker (0x40) for sil/dil/bpl.
                        let rex = get_reg_rex_esi_edi(&src.register(), 0x44, 0x40);
                        out.extend_opt(rex);
                        out.extend_from_slice(&[0x88, modrm]);
                    }
                    StoreSize::HalfWord => {
                        // mov word ptr [tmp], r16: 0x66 0x89 /r (0x66 REX 0x89 if extended)
                        if src.register().is_rex() {
                            out.extend_from_slice(&[0x66, 0x44, 0x89, modrm]);
                        } else {
                            out.extend_from_slice(&[0x66, 0x89, modrm]);
                        }
                    }
                    StoreSize::OneWord => {
                        // mov dword ptr [tmp], r32: 0x89 /r (REX.R if extended)
                        if src.register().is_rex() {
                            out.extend_from_slice(&[0x44, 0x89, modrm]);
                        } else {
                            out.extend_from_slice(&[0x89, modrm]);
                        }
                    }
                }
            }
            X86NativeOp::Push(r) => encode_push_pop(out, 0x50, r), // PUSH r64: 0x50+rd
            X86NativeOp::Pop(r) => encode_push_pop(out, 0x58, r),  // POP  r64: 0x58+rd
            X86NativeOp::Sub_Gas_Imm(imm) => {
                let gas_hw = X86_REG_GAS.hw_enc();
                let rex = 0x48 | u8::from(X86_REG_GAS.register().is_rex());
                let modrm = 0xC0 | (5 << 3) | (gas_hw & 7); // /5 for sub
                match imm {
                    // sub rsi, imm8 (sign-extended): REX.W 0x83 ModRM(Mod=11, /5, rsi=6) imm8
                    X86Immediate::Imm8(imm8) => {
                        out.extend_from_slice(&[rex, 0x83, modrm, imm8.get() as u8]);
                    }
                    // sub rsi, imm32 (sign-extended): REX.W 0x81 ModRM(Mod=11, /5, rsi=6) imm32_le
                    X86Immediate::Imm32(imm32) => {
                        let [d0, d1, d2, d3] = imm32.get().to_le_bytes();
                        out.extend_from_slice(&[rex, 0x81, modrm, d0, d1, d2, d3]);
                    }
                }
            }
            X86NativeOp::Sub_Gas_Tmp(tmp_reg) => {
                // sub rsi, rax/rbx: REX.W | 0x29 | ModRM(Mod=11, Reg=hw_tmp, RM=rsi=6)
                // Opcode 0x29 (SUB r/m64, r64): Reg=src(tmp), RM=dst(rsi/gas).
                let hw_tmp = tmp_reg.hw_enc();
                let gas_hw = X86_REG_GAS.hw_enc();
                let rex = 0x48 | u8::from(X86_REG_GAS.register().is_rex());
                let modrm = 0xC0 | (hw_tmp << 3) | (gas_hw & 7);
                out.extend_from_slice(&[rex, 0x29, modrm]);
            }
            X86NativeOp::Ret() => out.push(0xC3), // RET opcode
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "File operations are required for the assembler comparison tests."
)]
#[cfg(test)]
mod bytecode_tests {
    use crate::test_support::seeded_rng;
    use crate::{
        traits::NativeOp as _,
        traits::PcRelativeOffset,
        x86_64::{
            ops::{
                X86ConditionCode, X86Imm8Bit, X86Imm32Bit, X86Immediate, X86NativeOp,
                X86Reg32OrImm, X86ShiftAmount,
            },
            registers::{
                REG_EAX, REG_EBP, REG_EBX, REG_ECX, REG_EDI, REG_EDX, REG_ESI, REG_R8, REG_R8D,
                REG_R9, REG_R9D, REG_R10, REG_R10D, REG_R11, REG_R11D, REG_R12, REG_R12D, REG_R13,
                REG_R13D, REG_R14, REG_R14D, REG_R15, REG_R15D, REG_RAX, REG_RBP, REG_RBX, REG_RCX,
                REG_RDI, REG_RDX, REG_RSI, X86Reg32Bit, X86Reg32Or64, X86Reg64Bit, X86TempReg,
            },
        },
    };
    use rand::Rng;
    use riscv32::common::SignType;
    use riscv32::ops::{LoadSize, StoreSize};
    use riscv32_elf::consts::TEXT_SECTION_NAME;
    use std::io::Write;
    use std::process::Command;

    /// Assembles multiple x86-64 instructions (Intel syntax) and returns the raw bytecode.
    fn assemble_instructions(instructions: &[String]) -> Vec<u8> {
        let temp_dir = tempfile::tempdir().expect("Failed to create a temporary directory");
        let asm_file = temp_dir.path().join("x86_64_test.s");
        let obj_file = temp_dir.path().join("x86_64_test.o");
        let bin_file = temp_dir.path().join("x86_64_test.bin");

        let mut file = std::fs::File::create(&asm_file).expect("Failed to create asm file");
        writeln!(file, ".intel_syntax noprefix").expect("Failed to write .intel_syntax");
        writeln!(file, "{TEXT_SECTION_NAME}").expect("Failed to write .text");
        for instr in instructions {
            writeln!(file, "{instr}").expect("Failed to write instruction");
        }
        drop(file);

        let as_output = Command::new("x86_64-linux-gnu-as")
            .arg("-o")
            .arg(&obj_file)
            .arg(&asm_file)
            .output()
            .expect("Failed to run x86_64-linux-gnu-as. Is it installed?");

        if !as_output.status.success() {
            panic!(
                "Assembly failed: {}",
                String::from_utf8_lossy(&as_output.stderr)
            );
        }

        println!("Assembled instructions:\n{}", instructions.join("\n"));

        let objcopy_output = Command::new("x86_64-linux-gnu-objcopy")
            .args(["-O", "binary", "-j", TEXT_SECTION_NAME])
            .arg(&obj_file)
            .arg(&bin_file)
            .output()
            .expect("Failed to run x86_64-linux-gnu-objcopy");

        if !objcopy_output.status.success() {
            panic!(
                "objcopy failed: {}",
                String::from_utf8_lossy(&objcopy_output.stderr)
            );
        }

        let bytes = std::fs::read(&bin_file).expect("Failed to read binary");
        println!("Extracted bytecode bytes: {bytes:02X?}");
        bytes
    }

    // All general-purpose 32-bit registers (rsp excluded — not used as general-purpose).
    const REGS32: [X86Reg32Bit; 15] = [
        REG_EAX, REG_EBX, REG_ECX, REG_EDX, REG_ESI, REG_EDI, REG_EBP, REG_R8D, REG_R9D, REG_R10D,
        REG_R11D, REG_R12D, REG_R13D, REG_R14D, REG_R15D,
    ];

    // All general-purpose 64-bit registers (rsp excluded).
    const REGS64: [X86Reg64Bit; 15] = [
        REG_RAX, REG_RBX, REG_RCX, REG_RDX, REG_RSI, REG_RDI, REG_RBP, REG_R8, REG_R9, REG_R10,
        REG_R11, REG_R12, REG_R13, REG_R14, REG_R15,
    ];

    fn random_reg32(rng: &mut impl Rng) -> X86Reg32Bit {
        REGS32[rng.random_range(0..REGS32.len())]
    }

    fn random_reg64(rng: &mut impl Rng) -> X86Reg64Bit {
        REGS64[rng.random_range(0..REGS64.len())]
    }

    fn random_tmp(rng: &mut impl Rng) -> X86TempReg {
        if rng.random::<bool>() {
            X86TempReg::Tmp1
        } else {
            X86TempReg::Tmp2
        }
    }

    /// Returns an immediate that fits in a signed byte (Imm8 variant).
    fn random_imm8(rng: &mut impl Rng) -> X86Immediate {
        X86Immediate::Imm8(X86Imm8Bit::new(rng.random::<i8>()))
    }

    /// Returns an immediate that does NOT fit in a signed byte (Imm32 variant).
    fn random_imm32(rng: &mut impl Rng) -> X86Immediate {
        let v = loop {
            let v: i32 = rng.random::<i32>();
            if !(-128..=127).contains(&v) {
                break v;
            }
        };
        X86Immediate::Imm32(X86Imm32Bit::new(v))
    }

    fn random_condition(rng: &mut impl Rng) -> X86ConditionCode {
        const CONDITIONS: [X86ConditionCode; 10] = [
            X86ConditionCode::Equal,
            X86ConditionCode::NotEqual,
            X86ConditionCode::Less,
            X86ConditionCode::Greater,
            X86ConditionCode::GreaterEqual,
            X86ConditionCode::Below,
            X86ConditionCode::Above,
            X86ConditionCode::AboveEqual,
            X86ConditionCode::Sign,
            X86ConditionCode::NotSign,
        ];
        CONDITIONS[rng.random_range(0..CONDITIONS.len())]
    }

    fn random_pc_offset(rng: &mut impl Rng) -> PcRelativeOffset {
        PcRelativeOffset::new(rng.random::<i32>())
    }

    fn zero_pc_offset() -> PcRelativeOffset {
        PcRelativeOffset::new(0)
    }

    /// Generates one round of test cases, covering all directly-encodable X86Op variants.
    fn generate_test_cases(rng: &mut impl Rng) -> Vec<X86NativeOp> {
        let r1 = random_reg32(rng);
        let r2 = random_reg32(rng);
        let r3 = random_reg32(rng);
        let r64_1 = random_reg64(rng);
        let r64_2 = random_reg64(rng);
        let tmp1 = random_tmp(rng);
        let tmp2 = random_tmp(rng);
        let cond = random_condition(rng);
        let imm8 = random_imm8(rng);
        let imm32 = random_imm32(rng);
        // Shift amount of 1 triggers the `D1` (shift-by-1) opcode, so test both 1 and >1.
        let shift_gt1 = X86Imm8Bit::new(rng.random_range(2i8..=31));

        vec![
            // Address calculation
            X86NativeOp::Lea_Tmp_Mem(tmp1, random_pc_offset(rng)),
            X86NativeOp::Lea_Tmp_Mem(tmp1, zero_pc_offset()),
            X86NativeOp::Lea_Tmp_Mem(tmp2, random_pc_offset(rng)),
            X86NativeOp::Lea_Tmp_Mem(tmp2, zero_pc_offset()),
            X86NativeOp::Add_Tmp64_Reg64(tmp1, r64_1),
            X86NativeOp::Add_Tmp64_Reg64(tmp2, r64_2),
            // Data movement — RIP-relative
            X86NativeOp::Mov_Reg_Mem(X86Reg32Or64::Reg32(r1), random_pc_offset(rng)),
            X86NativeOp::Mov_Reg_Mem(X86Reg32Or64::Reg32(r1), zero_pc_offset()),
            X86NativeOp::Mov_Reg_Mem(X86Reg32Or64::Reg64(r64_1), random_pc_offset(rng)),
            X86NativeOp::Mov_Reg_Mem(X86Reg32Or64::Reg64(r64_1), zero_pc_offset()),
            X86NativeOp::Mov_Mem_Reg(random_pc_offset(rng), X86Reg32Or64::Reg32(r2)),
            X86NativeOp::Mov_Mem_Reg(zero_pc_offset(), X86Reg32Or64::Reg32(r2)),
            X86NativeOp::Mov_Mem_Reg(random_pc_offset(rng), X86Reg32Or64::Reg64(r64_2)),
            X86NativeOp::Mov_Mem_Reg(zero_pc_offset(), X86Reg32Or64::Reg64(r64_2)),
            X86NativeOp::Mov_Mem_Imm(random_pc_offset(rng), imm8),
            X86NativeOp::Mov_Mem_Imm(zero_pc_offset(), imm8),
            X86NativeOp::Mov_Mem_Imm(random_pc_offset(rng), imm32),
            X86NativeOp::Mov_Mem_Imm(zero_pc_offset(), imm32),
            X86NativeOp::Mov_Reg_MemTmp(r1, tmp1),
            // Register/immediate moves
            X86NativeOp::Mov_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Mov_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Mov_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            // Arithmetic
            X86NativeOp::Add_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Add_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Add_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            X86NativeOp::Sub_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Sub_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Sub_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            X86NativeOp::Neg_Reg(r1),
            X86NativeOp::Mul_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Mul_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Mul_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            X86NativeOp::MulClobber(SignType::Signed, r1),
            X86NativeOp::MulClobber(SignType::Unsigned, r2),
            X86NativeOp::DivClobber(SignType::Signed, r1),
            X86NativeOp::DivClobber(SignType::Unsigned, r2),
            // Bitwise
            X86NativeOp::And_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::And_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::And_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            X86NativeOp::Or_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Or_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Or_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            X86NativeOp::Xor_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Xor_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Xor_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            X86NativeOp::Xor_Self(X86Reg32Or64::Reg32(r1)),
            X86NativeOp::Xor_Self(X86Reg32Or64::Reg64(r64_1)),
            // Shifts — test shift-by-1 (D1 opcode), shift-by-imm (C1 opcode), and shift-by-CL (D3)
            X86NativeOp::Shl_Reg_Sft(r1, X86ShiftAmount::Imm(X86Imm8Bit::new(1))),
            X86NativeOp::Shl_Reg_Sft(r2, X86ShiftAmount::Imm(shift_gt1)),
            X86NativeOp::Shl_Reg_Sft(r3, X86ShiftAmount::Cl),
            X86NativeOp::Shr_Reg_Sft(r1, X86ShiftAmount::Imm(X86Imm8Bit::new(1))),
            X86NativeOp::Shr_Reg_Sft(r2, X86ShiftAmount::Imm(shift_gt1)),
            X86NativeOp::Shr_Reg_Sft(r3, X86ShiftAmount::Cl),
            X86NativeOp::Sar_Reg_Sft(r1, X86ShiftAmount::Imm(X86Imm8Bit::new(1))),
            X86NativeOp::Sar_Reg_Sft(r2, X86ShiftAmount::Imm(shift_gt1)),
            X86NativeOp::Sar_Reg_Sft(r3, X86ShiftAmount::Cl),
            // Comparison
            X86NativeOp::Cmp_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Cmp_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Cmp_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            X86NativeOp::Test_Reg_RoI(r1, X86Reg32OrImm::Reg(r2)),
            X86NativeOp::Test_Reg_RoI(r1, X86Reg32OrImm::Imm(imm8)),
            X86NativeOp::Test_Reg_RoI(r1, X86Reg32OrImm::Imm(imm32)),
            // Conditional jumps and unconditional jump (PcRelative only — Symbolic requires relocation)
            X86NativeOp::Jmp(random_pc_offset(rng)),
            X86NativeOp::Jcc(cond, random_pc_offset(rng)),
            X86NativeOp::Jmp_Tmp(tmp1),
            // Set byte on condition and zero-extension
            X86NativeOp::SetLow8(SignType::Signed, r1.b()),
            X86NativeOp::SetLow8(SignType::Unsigned, r2.b()),
            X86NativeOp::Movzx_Self(r1),
            X86NativeOp::Movsxd_Tmp(tmp1),
            X86NativeOp::Cdq,
            // Loads from [tmp + 0]
            X86NativeOp::Load_Reg_MemTmp(LoadSize::OneByte(SignType::Signed), r1, tmp1),
            X86NativeOp::Load_Reg_MemTmp(LoadSize::OneByte(SignType::Unsigned), r2, tmp2),
            X86NativeOp::Load_Reg_MemTmp(LoadSize::HalfWord(SignType::Signed), r1, tmp1),
            X86NativeOp::Load_Reg_MemTmp(LoadSize::HalfWord(SignType::Unsigned), r2, tmp2),
            X86NativeOp::Load_Reg_MemTmp(LoadSize::OneWord, r1, tmp1),
            // Stores to [tmp + 0]
            X86NativeOp::Store_MemTmp_Reg(StoreSize::OneByte, tmp1, r1),
            X86NativeOp::Store_MemTmp_Reg(StoreSize::HalfWord, tmp2, r2),
            X86NativeOp::Store_MemTmp_Reg(StoreSize::OneWord, tmp1, r3),
            // Stack
            X86NativeOp::Push(r64_1),
            X86NativeOp::Pop(r64_2),
            // Gas metering
            X86NativeOp::Sub_Gas_Imm(imm8),
            X86NativeOp::Sub_Gas_Imm(imm32),
            X86NativeOp::Sub_Gas_Tmp(tmp1),
            // Return
            X86NativeOp::Ret(),
        ]
    }

    #[test]
    fn all_x86_opcodes_match_assembler() {
        let mut rng = seeded_rng("all_x86_opcodes_match_assembler");
        let test_cases: Vec<X86NativeOp> = (0..500)
            .flat_map(|_| generate_test_cases(&mut rng))
            .collect();

        // Collect asm strings and our bytecode bytes for all directly-encodable ops.
        let mut asm_lines: Vec<String> = Vec::new();
        let mut our_codes: Vec<Vec<u8>> = Vec::new();
        let mut descriptions: Vec<String> = Vec::new();

        for op in &test_cases {
            let mut machine_code = Vec::new();
            op.encode(&mut machine_code);
            if machine_code.len() as u32 != op.size_in_bytes() {
                panic!(
                    "Encoded bytecode length {} does not match expected size {} for op {:?}",
                    machine_code.len(),
                    op.size_in_bytes(),
                    op
                );
            }
            asm_lines.push(op.as_assembly());
            our_codes.push(machine_code);
            descriptions.push(format!("{op:?}"));
        }

        // Assemble all instructions at once.
        let assembled = assemble_instructions(&asm_lines);

        // x86 instructions are variable-length. Use the byte count from our encoding to
        // determine instruction boundaries in the assembled binary. A size disagreement
        // will manifest as a cascade of mismatches starting at the first divergence.
        let mut offset = 0usize;
        let mut mismatches: Vec<String> = Vec::new();

        for (i, (ours, desc)) in our_codes.iter().zip(descriptions.iter()).enumerate() {
            let size = ours.len();
            if offset + size > assembled.len() {
                mismatches.push(format!(
                    "[{i}] {desc}\n    ran past end of assembled binary at offset {offset}"
                ));
                break;
            }
            let theirs = &assembled[offset..offset + size];
            if ours.as_slice() != theirs {
                mismatches.push(format!(
                    "[{i}] {desc}\n    asm: {}\n    ours:   {ours:02x?}\n    theirs: {theirs:02x?}",
                    asm_lines[i]
                ));
            }
            offset += size;
        }

        if offset != assembled.len() && mismatches.is_empty() {
            mismatches.push(format!(
                "Byte count mismatch: consumed {offset} bytes but assembled {} bytes total",
                assembled.len()
            ));
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
}
