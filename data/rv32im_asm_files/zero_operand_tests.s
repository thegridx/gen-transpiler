# test instructions that read x0 as a source operand
# On aarch64, register number 31 means WZR in some encodings and WSP in others, so every
# lowering that may put the guest's x0 into an operand slot needs coverage.
# Returns 0 on success, or -N identifying the failing test, in a0
.section .text
.globl _test_start

_test_start:
    la a2, memory_loc

    # --------------------
    # AMOs with rs2 = x0
    # --------------------
    # Test 1: AMOSWAP.W stores 0 and returns the old value
    li t0, 0x12345678
    sw t0, 0(a2)
    amoswap.w a0, zero, (a2)
    li a4, -1
    bne a0, t0, fail
    lw t1, 0(a2)
    li a4, -2
    bnez t1, fail

    # Test 2: AMOSWAP.W with rd = x0 still stores 0
    sw t0, 0(a2)
    amoswap.w zero, zero, (a2)
    lw t1, 0(a2)
    li a4, -3
    bnez t1, fail

    # Test 3: AMOADD.W with x0 leaves memory unchanged (atomic load idiom)
    sw t0, 0(a2)
    amoadd.w a0, zero, (a2)
    li a4, -4
    bne a0, t0, fail
    lw t1, 0(a2)
    li a4, -5
    bne t1, t0, fail

    # Test 4: AMOOR.W with x0 leaves memory unchanged (atomic load idiom)
    sw t0, 0(a2)
    amoor.w a0, zero, (a2)
    li a4, -6
    bne a0, t0, fail
    lw t1, 0(a2)
    li a4, -7
    bne t1, t0, fail

    # Test 5: AMOXOR.W with x0 leaves memory unchanged
    sw t0, 0(a2)
    amoxor.w a0, zero, (a2)
    lw t1, 0(a2)
    li a4, -8
    bne t1, t0, fail

    # Test 6: AMOAND.W with x0 clears memory
    sw t0, 0(a2)
    amoand.w a0, zero, (a2)
    lw t1, 0(a2)
    li a4, -9
    bnez t1, fail

    # Test 7: AMOMAX.W with x0 on a negative value stores 0
    li t0, -5
    sw t0, 0(a2)
    amomax.w a0, zero, (a2)
    lw t1, 0(a2)
    li a4, -10
    bnez t1, fail

    # Test 8: AMOMIN.W with x0 on a positive value stores 0
    li t0, 5
    sw t0, 0(a2)
    amomin.w a0, zero, (a2)
    lw t1, 0(a2)
    li a4, -11
    bnez t1, fail

    # Test 9: AMOMAXU.W with x0 leaves memory unchanged
    sw t0, 0(a2)
    amomaxu.w a0, zero, (a2)
    lw t1, 0(a2)
    li a4, -12
    bne t1, t0, fail

    # Test 10: AMOMINU.W with x0 stores 0
    sw t0, 0(a2)
    amominu.w a0, zero, (a2)
    lw t1, 0(a2)
    li a4, -13
    bnez t1, fail

    # --------------------
    # SLTI / SLTIU with rs1 = x0
    # --------------------
    # Test 11: 0 < 1 (signed)
    slti a0, zero, 1
    li a4, -14
    li t1, 1
    bne a0, t1, fail

    # Test 12: 0 < 0 (signed) is false
    slti a0, zero, 0
    li a4, -15
    bnez a0, fail

    # Test 13: 0 < -1 (signed) is false
    slti a0, zero, -1
    li a4, -16
    bnez a0, fail

    # Test 14: 0 < 2047 (signed)
    slti a0, zero, 2047
    li a4, -17
    li t1, 1
    bne a0, t1, fail

    # Test 15: 0 < 1 (unsigned)
    sltiu a0, zero, 1
    li a4, -18
    li t1, 1
    bne a0, t1, fail

    # Test 16: 0 < 0 (unsigned) is false
    sltiu a0, zero, 0
    li a4, -19
    bnez a0, fail

    # Test 17: 0 < 0xFFFFFFFF (unsigned, sign-extended -1)
    sltiu a0, zero, -1
    li a4, -20
    li t1, 1
    bne a0, t1, fail

    # Test 18: 0 < 0xFFFFF800 (unsigned, sign-extended -2048)
    sltiu a0, zero, -2048
    li a4, -21
    li t1, 1
    bne a0, t1, fail

    # --------------------
    # M extension with x0 operands
    # --------------------
    # Test 19: MULHSU with rs2 = x0
    li a1, -7
    mulhsu a0, a1, zero
    li a4, -22
    bnez a0, fail

    # Test 20: MULHSU with rs1 = x0
    li a1, -7
    mulhsu a0, zero, a1
    li a4, -23
    bnez a0, fail

    # Test 21: REM with rs1 = x0 and a zero divisor gives the dividend (0)
    li a1, 0
    rem a0, zero, a1
    li a4, -24
    bnez a0, fail

    # Test 22: REMU with rs1 = x0 and a zero divisor gives the dividend (0)
    li a1, 0
    remu a0, zero, a1
    li a4, -25
    bnez a0, fail

    # Test 23: REM with both operands x0
    rem a0, zero, zero
    li a4, -26
    bnez a0, fail

    # Test 24: REMU with both operands x0
    remu a0, zero, zero
    li a4, -27
    bnez a0, fail

    # Test 25: DIV with both operands x0 gives -1
    div a0, zero, zero
    li a4, -28
    li t1, -1
    bne a0, t1, fail

    # Test 26: DIVU with both operands x0 gives 0xFFFFFFFF
    divu a0, zero, zero
    li a4, -29
    li t1, -1
    bne a0, t1, fail

    # Test 27: REM with rs1 = x0 and a non-zero divisor
    li a1, 3
    rem a0, zero, a1
    li a4, -30
    bnez a0, fail

    # All tests passed
    li a7, 0    # no panic
    li a0, 0    # success
    ret

fail:
    li a7, 0    # no panic
    mv a0, a4   # the failing test's code
    ret

.section .data
    .align 4
memory_loc:
    .word 0
