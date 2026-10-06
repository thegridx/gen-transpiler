# test branches whose target is invalid (misaligned, or past the end of .text) but that are not
# taken. The spec faults only when such a branch is taken, so each one must fall through.
# Returns 0 on success in a0
.section .text
.globl _test_start

_test_start:
    li a0, 1
    li a1, 2
    # misaligned targets
    bne zero, zero, . + 6
    beq a0, a1, . + 6
    blt a1, a0, . + 6
    bge a0, a1, . + 6
    bltu a1, a0, . + 6
    bgeu a0, a1, . + 6
    # targets past the end of .text (within branch range, so the assembler keeps the branch)
    bne zero, zero, . + 0x800
    beq a0, a1, . + 0x800
    blt a1, a0, . + 0x800

    li a7, 0    # no panic
    li a0, 0    # success
    ret
