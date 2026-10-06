//! Instruction words for fuzz inputs, biased toward ones that decode.

use arbitrary::Arbitrary;

/// The RV32IMA major opcodes. Forcing one onto an arbitrary word makes most words decode, so
/// programs run far enough to reach interesting states.
const OPCODES: [u32; 12] = [
    0x13, 0x33, 0x37, 0x17, 0x03, 0x23, 0x63, 0x6F, 0x67, 0x73, 0x2F, 0x0F,
];

/// `jalr x0, 0(ra)`: return from `main`.
pub(crate) const RET: u32 = 0x0000_8067;

/// One instruction: `bits` with the opcode `OPCODES[opcode % 12]`, or `bits` unchanged when
/// `opcode` is 255.
#[derive(Arbitrary, Debug)]
pub(crate) struct Word {
    opcode: u8,
    bits: u32,
}

impl Word {
    pub(crate) fn encode(&self) -> u32 {
        match OPCODES.get(usize::from(self.opcode) % OPCODES.len()) {
            Some(opcode) if self.opcode != u8::MAX => (self.bits & !0x7F) | opcode,
            _ => self.bits,
        }
    }
}

/// At most `max` words, then a `ret`.
pub(crate) fn program(words: &[Word], max: usize) -> Vec<u32> {
    let mut code: Vec<u32> = words.iter().take(max).map(Word::encode).collect();
    code.push(RET);
    code
}
