//! The emulator is a reference model for transpiled code: random programs must behave the same
//! in both. Each program runs in the emulator and transpiled for this host, as an init function
//! (sometimes) and two calls, resumed after ecalls the host halts in (sometimes). The two runs
//! must agree after every step (see `differential/`).
//!
//! The programs mix every kind of instruction, valid and not: accesses near both ends of the
//! writable range, out of bounds and misaligned; branches to misaligned targets, to the end of
//! the code and far ahead; indirect jumps into the middle of gas blocks; writes to ra, sp and the
//! base registers; ecalls with reserved numbers; and runs out of gas. Some programs are long
//! enough to split gas blocks on cost. Generated branches and jumps only go forward, and a gas
//! limit ends any loop a random word makes.
//!
//! The programs come from a fixed seed, printed by the test; set `TEST_SEED` to try others or to
//! replay a failure.

mod differential;
mod riscv_elf_spec;

use differential::{Start, assert_same_behavior};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use riscv_elf_spec::RiscvElfSpec;
use riscv32::registers::RvPreservedRegister;
use strum::IntoEnumIterator as _;

/// Programs per test run.
const N_PROGRAMS: usize = 2000;
/// The base registers for memory accesses, set by the host and rarely written: s0 (x8) at the
/// first writable address and s1 (x9) at the end of guest memory.
const BASE: u32 = 8;
const HIGH_BASE: u32 = 9;
/// x5 (t0): scratch for atomic addresses and indirect jumps.
const SCRATCH: u32 = 5;
/// `jalr x0, 0(ra)`.
const RET: u32 = 0x0000_8067;

fn seed() -> u64 {
    let seed = std::env::var("TEST_SEED").map_or(0x5EED_5EED, |value| {
        match value.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => value.parse().ok(),
        }
        .unwrap_or_else(|| panic!("TEST_SEED={value:?} is not a u64"))
    });
    println!("emulator_differential: seed {seed:#x} (rerun with TEST_SEED={seed:#x})");
    seed
}

fn r_type(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

fn i_type(imm: i32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

fn s_type(offset: i32, rs2: u32, rs1: u32, funct3: u32) -> u32 {
    let imm = offset as u32;
    (((imm >> 5) & 0x7F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | ((imm & 0x1F) << 7)
        | 0x23
}

fn b_type(offset: i32, rs2: u32, rs1: u32, funct3: u32) -> u32 {
    let imm = offset as u32;
    (((imm >> 12) & 1) << 31)
        | (((imm >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (((imm >> 1) & 0xF) << 8)
        | (((imm >> 11) & 1) << 7)
        | 0x63
}

fn j_type(offset: i32, rd: u32) -> u32 {
    let imm = offset as u32;
    (((imm >> 20) & 1) << 31)
        | (((imm >> 1) & 0x3FF) << 21)
        | (((imm >> 11) & 1) << 20)
        | (((imm >> 12) & 0xFF) << 12)
        | (rd << 7)
        | 0x6F
}

/// A destination register: mostly not ra, sp or a base register, so the final `ret` usually
/// returns and memory accesses usually keep a known base.
fn rd(rng: &mut StdRng) -> u32 {
    // Rarely any register at all: ra, sp and the bases have special mappings in the backends.
    if rng.random_ratio(1, 20) {
        return reg(rng);
    }
    loop {
        let reg = rng.random_range(0..32);
        if !matches!(reg, 1 | 2 | BASE | HIGH_BASE) {
            return reg;
        }
    }
}

fn reg(rng: &mut StdRng) -> u32 {
    rng.random_range(0..32)
}

/// Appends one random instruction, or a short sequence, to `code`. `remaining` is how many
/// instructions are left before the final `ret`, which bounds forward jumps.
fn push_random(rng: &mut StdRng, code: &mut Vec<u32>, remaining: i32) {
    // Mostly short hops; sometimes as far as a branch reaches (4 KiB).
    let forward = |r: &mut StdRng| {
        let reach = if r.random_ratio(1, 8) { 1000 } else { 8 };
        4 * r.random_range(1..=remaining.clamp(1, reach))
    };
    // A base register and an offset: mostly around the first writable address, sometimes just
    // below the end of memory, sometimes from any register.
    let based = |r: &mut StdRng| match r.random_range(0..20) {
        0..=11 => (BASE, r.random_range(-2048..2048)),
        12..=16 => (HIGH_BASE, r.random_range(-64..8)),
        _ => (reg(r), r.random_range(-2048..2048)),
    };
    let misalign = |r: &mut StdRng| if r.random_ratio(1, 16) { 2 } else { 0 };
    // Random words end most programs early, so they are rare.
    if rng.random_ratio(1, 100) {
        code.push(rng.random());
        return;
    }
    match rng.random_range(0..30) {
        // OP-IMM, including the shifts.
        0..=5 => {
            let funct3 = rng.random_range(0..8);
            let imm = match funct3 {
                1 => rng.random_range(0..32),
                5 => rng.random_range(0..32) | if rng.random() { 0x400 } else { 0 },
                _ => rng.random_range(-2048..2048),
            };
            code.push(i_type(imm, reg(rng), funct3, rd(rng), 0x13));
        }
        // OP, and RV32M.
        6..=12 => {
            let funct3 = rng.random_range(0..8);
            let funct7 = match rng.random_range(0..3) {
                0 => 0,
                1 if matches!(funct3, 0 | 5) => 0x20,
                _ => 1,
            };
            code.push(r_type(funct7, reg(rng), reg(rng), funct3, rd(rng), 0x33));
        }
        // LUI and AUIPC.
        13 | 14 => {
            let opcode = if rng.random() { 0x37 } else { 0x17 };
            code.push((rng.random::<u32>() & 0xFFFF_F000) | (rd(rng) << 7) | opcode);
        }
        // Loads near either end of the writable range, some out of bounds.
        15..=17 => {
            let funct3 = [0, 1, 2, 4, 5][rng.random_range(0..5)];
            let (base, offset) = based(rng);
            code.push(i_type(offset, base, funct3, rd(rng), 0x03));
        }
        // Stores near either end of the writable range; below it is read-only memory.
        18..=20 => {
            let funct3 = rng.random_range(0..3);
            let (base, offset) = based(rng);
            code.push(s_type(offset, reg(rng), base, funct3));
        }
        // Forward branches, to the end of the code at most, and sometimes misaligned.
        21 | 22 => {
            let funct3 = [0, 1, 4, 5, 6, 7][rng.random_range(0..6)];
            let offset = forward(rng) + misalign(rng);
            code.push(b_type(offset, reg(rng), reg(rng), funct3));
        }
        // A forward jump, sometimes misaligned.
        23 => {
            let target_rd = if rng.random() { 0 } else { rd(rng) };
            code.push(j_type(forward(rng) + misalign(rng), target_rd));
        }
        // An indirect jump forward, which can land in the middle of a gas block.
        24 => {
            let offset = 4 + forward(rng) + misalign(rng);
            code.push((SCRATCH << 7) | 0x17); // auipc t0, 0
            code.push(i_type(offset, SCRATCH, 0, rd(rng), 0x67));
        }
        // An ecall, with a number that may be reserved.
        25 | 26 => {
            code.push(i_type(rng.random_range(-3..6), 0, 0, 17, 0x13)); // li a7, n
            code.push(0x0000_0073);
        }
        // An atomic on an address near the base, sometimes misaligned.
        27 | 28 => {
            let (base, words) = if rng.random_ratio(1, 3) {
                (HIGH_BASE, rng.random_range(-16..2))
            } else {
                (BASE, rng.random_range(-16..256))
            };
            let offset = 4 * words + misalign(rng);
            code.push(i_type(offset, base, 0, SCRATCH, 0x13)); // addi t0, base, offset
            let funct5 = [
                0x00, 0x01, 0x02, 0x03, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1C,
            ][rng.random_range(0..11)];
            let rs2 = if funct5 == 0x02 { 0 } else { reg(rng) };
            code.push(r_type(funct5 << 2, rs2, SCRATCH, 2, rd(rng), 0x2F));
        }
        // A fence.
        _ => code.push(0x0FF0_000F),
    }
}

fn random_program(rng: &mut StdRng) -> Vec<u32> {
    // Some programs are long enough to split a gas block on cost (4095 per block).
    let len = if rng.random_ratio(1, 50) {
        rng.random_range(4200..5000)
    } else {
        rng.random_range(4..48)
    };
    let mut code = Vec::new();
    while code.len() < len {
        let remaining = i32::try_from(len - code.len()).unwrap_or(1);
        push_random(rng, &mut code, remaining);
    }
    code.push(RET);
    code
}

/// A random starting state.
fn random_start(rng: &mut StdRng) -> Start {
    // Random words can decode as backward branches, so gas also bounds every run.
    let gas = if rng.random_ratio(1, 3) {
        rng.random_range(0..64)
    } else {
        100_000
    };
    Start {
        arguments: rng.random(),
        saved_registers: RvPreservedRegister::iter().map(|_| rng.random()).collect(),
        gas,
        halt_on_ecall: rng.random_ratio(1, 4),
    }
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn random_programs_behave_the_same_in_the_emulator() {
    let mut rng = StdRng::seed_from_u64(seed());
    let mut outcomes = std::collections::BTreeMap::<String, usize>::new();
    for program in 0..N_PROGRAMS {
        let code = random_program(&mut rng);
        let start = random_start(&mut rng);
        // Sometimes `main` is also the init function, so it runs once more first.
        let init_array = if rng.random_ratio(1, 4) {
            vec![0]
        } else {
            Vec::new()
        };
        println!(
            "program {program} (gas {}, init {}, halts {}): {code:08x?}",
            start.gas,
            !init_array.is_empty(),
            start.halt_on_ecall
        );
        let elf = RiscvElfSpec {
            code,
            init_array,
            ..RiscvElfSpec::default()
        }
        .build();
        let outcome = assert_same_behavior(&elf, &start);
        let kind = outcome
            .split([' ', '{'])
            .next()
            .unwrap_or_default()
            .to_string();
        *outcomes.entry(kind).or_default() += 1;
    }
    println!("outcomes: {outcomes:?}");
}
