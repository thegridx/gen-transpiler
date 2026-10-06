.section .text
        .align 2
# Entry points must be global symbols.
        .globl misaligned_jump
        .globl tagged_jump

# `jalr` target with bit 1 set: RISC-V instruction-address-misaligned (IALIGN=32), must panic
# in both the regular and the trusted variant.
misaligned_jump:
        la a0, misaligned_jump
        addi a0, a0, 2
        jalr a0
        ret

# `jalr` target with bit 0 set: the hardware clears it, so this lands on `tagged_target`.
tagged_jump:
        la t0, tagged_target
        addi t0, t0, 1
        jr t0
        li a0, -1
        ret
tagged_target:
        li a0, 7
        ret
