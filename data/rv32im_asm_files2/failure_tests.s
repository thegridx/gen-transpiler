.section .text
        .align 2
# Entry points must be global symbols.
        .globl illegal_opcode
        .globl out_of_gas
        .globl illegal_read
        .globl illegal_jump
        .globl load_past_end
        .globl store_past_end
        .globl amo_past_end
        .globl taken_branch_misaligned
        .globl taken_branch_past_end
        .globl jal_misaligned
        .globl lr_misaligned
        .globl sc_misaligned
        .globl store_to_code
        .globl amo_to_code
        .globl load_wraparound
        .globl jalr_to_data
        .globl branch_to_end
        .globl div_at_end

illegal_opcode:
        li a0, 0
        .word 0x01010101
        ret

out_of_gas:
        li a0, -10
        li a7, 5
        ecall
        addi a0, a0, 1
        ret

illegal_read:
        li a0, 0x7FFFFFFF
        lbu a1, 0(a0)
        ret

illegal_jump:
        li a0, 0x7FFFFFFF
        jalr a0
        ret

# Guest memory ends at `_data_end_value`; the register store, which holds gas, starts there.
load_past_end:
        lui a0, %hi(_data_end_value)
        addi a0, a0, %lo(_data_end_value)
        lw a1, -3(a0)
        ret

store_past_end:
        lui a0, %hi(_data_end_value)
        addi a0, a0, %lo(_data_end_value)
        sb zero, 0(a0)
        ret

amo_past_end:
        lui a0, %hi(_data_end_value)
        addi a0, a0, %lo(_data_end_value)
        amoadd.w a1, zero, (a0)
        ret

# A taken branch or a `jal` to an invalid target faults: a panic, as for `jalr`.
taken_branch_misaligned:
        beq zero, zero, . + 6
        ret

taken_branch_past_end:
        beq zero, zero, . + 0x800
        ret

jal_misaligned:
        jal . + 6
        ret

# LR and SC need 4-byte-aligned addresses, like the AMOs. `sp - 6` is writable but misaligned.
lr_misaligned:
        addi a0, sp, -6
        lr.w a1, (a0)
        ret

sc_misaligned:
        addi a0, sp, -6
        sc.w a1, zero, (a0)
        ret

# Stores may only write [_data_start_value, _data_end_value): not code or read-only data.
store_to_code:
        sw zero, 0(zero)
        ret

amo_to_code:
        amoadd.w a1, zero, (zero)
        ret

# Addresses wrap around in 32 bits: -4 is far past the end of guest memory.
load_wraparound:
        lw a1, -4(zero)
        ret

# A `jalr` may only target the code: an aligned address in the stack is rejected.
jalr_to_data:
        mv a0, sp
        jalr a0
        ret

# These two must stay last: the `div` is the final instruction of `.text`. Reaching the end of
# the code by a branch, or by running past a final `div` (whose transpilation branches to the
# next instruction), is like falling off the end: an illegal-instruction exit.
branch_to_end:
        beq zero, zero, end_of_text

div_at_end:
        li a1, 3
        div a0, a0, a1
end_of_text:
