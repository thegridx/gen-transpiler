.section .text
    .align 2
.global _test_start
_test_start:
    .align 2
    .globl _start

_start:
    # skipping x1 == ra
    li x2, 3
    li x3, 5
    li x4, 7
    li x5, 11
    li x6, 13
    li x7, 17
    li x8, 19
    li x9, 23

    li x10, 29    # (a0)
    li x11, 31    # (a1)

    li x18, 61    # (s2)
    li x19, 67    # (s3)
    li x20, 71    # (s4)
    li x21, 73    # (s5)
    li x22, 79    # (s6)
    li x23, 83    # (s7)
    li x24, 89    # (s8)
    li x25, 97    # (s9)
    li x26, 101   # (s10)
    li x27, 103   # (s11)
    li x28, 107   # (t3)
    li x29, 109   # (t4)
    li x30, 113   # (t5)
    li x31, 127   # (t6)

    # Load 1 into a7 (x17)
    li a7, 1

    # Issue ecall
    ecall
        
    addi x2, x2, -3
    addi x3, x3, -5
    addi x4, x4, -7
    addi x5, x5, -11
    addi x6, x6, -13
    addi x7, x7, -17
    addi x8, x8, -19
    addi x9, x9, -23

    # a0 should be 29 + 31 (not 29)
    addi x10, x10, -60
    # a7 should still be 1: an ecall keeps it, as the RISC-V ABI requires
    addi x17, x17, -1

    addi x18, x18, -61
    addi x19, x19, -67
    addi x20, x20, -71
    addi x21, x21, -73
    addi x22, x22, -79
    addi x23, x23, -83
    addi x24, x24, -89
    addi x25, x25, -97
    addi x26, x26, -101
    addi x27, x27, -103
    addi x28, x28, -107
    addi x29, x29, -109
    addi x30, x30, -113
    addi x31, x31, -127
    bnez x2, fail
    bnez x3, fail
    bnez x4, fail
    bnez x5, fail
    bnez x6, fail
    bnez x7, fail
    bnez x8, fail
    bnez x9, fail
        
    bnez x10, fail
    bnez x17, fail
        
    bnez x18, fail
    bnez x19, fail
    bnez x20, fail
    bnez x21, fail
    bnez x22, fail
    bnez x23, fail
    bnez x24, fail
    bnez x25, fail
    bnez x26, fail
    bnez x27, fail
    bnez x28, fail
    bnez x29, fail
    bnez x30, fail
    bnez x31, fail

success:
    li a0, 0
    li a7, 0
    ret

fail:
    li a0, -1
    li a7, 0
    ret
