.section .text
.globl _test_start
_test_start:
    addi sp, sp, -16
    sw ra, 12(sp)

    # Scenario 1: MUL with t1, t2
    li t1, 100
    li t2, 200
    mul a1, t1, t2
    li a2, 20000
    bne a1, a2, fail_muldiv_1

    # Scenario 2: MUL with t3, t4
    li t3, 0x00010000
    li t4, 0x00000100
    mul a1, t3, t4
    li a2, 0x01000000
    bne a1, a2, fail_muldiv_2

    # Scenario 3: MUL with t5, t6 (negative)
    li t5, -10
    li t6, 20
    mul a1, t5, t6
    li a2, -200
    bne a1, a2, fail_muldiv_3

    # Scenario 4: MULH with t1, t2 (high bits signed)
    li t1, 0x40000000
    li t2, 4
    mulh a1, t1, t2
    li a2, 1
    bne a1, a2, fail_muldiv_4

    # Scenario 5: MULHU with t3, t4 (high bits unsigned)
    li t3, 0x80000000
    li t4, 4
    mulhu a1, t3, t4
    li a2, 2
    bne a1, a2, fail_muldiv_5

    # Scenario 6: MULHSU with t5, t6
    li t5, 0x80000000  # Negative when signed
    li t6, 2
    mulhsu a1, t5, t6
    li a2, 0xFFFFFFFF
    bne a1, a2, fail_muldiv_6

    # Scenario 7: DIV with t1, t2
    li t1, 1000
    li t2, 10
    div a1, t1, t2
    li a2, 100
    bne a1, a2, fail_muldiv_7

    # Scenario 8: DIV with t3, t4 (negative dividend)
    li t3, -1000
    li t4, 10
    div a1, t3, t4
    li a2, -100
    bne a1, a2, fail_muldiv_8

    # Scenario 9: DIV with t5, t6 (negative divisor)
    li t5, 1000
    li t6, -10
    div a1, t5, t6
    li a2, -100
    bne a1, a2, fail_muldiv_9

    # Scenario 10: DIVU with t1, t2 (unsigned)
    li t1, 0x80000000
    li t2, 2
    divu a1, t1, t2
    li a2, 0x40000000
    bne a1, a2, fail_muldiv_10

    # Scenario 11: REM with t3, t4
    li t3, 1003
    li t4, 10
    rem a1, t3, t4
    li a2, 3
    bne a1, a2, fail_muldiv_11

    # Scenario 12: REM with t5, t6 (negative dividend)
    li t5, -1003
    li t6, 10
    rem a1, t5, t6
    li a2, -3
    bne a1, a2, fail_muldiv_12

    # Scenario 13: REMU with t1, t2 (unsigned)
    li t1, 0x80000005
    li t2, 0x10
    remu a1, t1, t2
    li a2, 5
    bne a1, a2, fail_muldiv_13

    # Scenario 14: DIV by zero with t3, t4 (result should be -1)
    li t3, 100
    li t4, 0
    div a1, t3, t4
    li a2, -1
    bne a1, a2, fail_muldiv_14

    # Scenario 15: REM by zero with t5, t6 (result should be dividend)
    li t5, 100
    li t6, 0
    rem a1, t5, t6
    li a2, 100
    bne a1, a2, fail_muldiv_15

    # Scenario 16: Multiply-accumulate pattern with t3, t4, t5
    li t3, 10
    li t4, 20
    li t5, 300
    mul a1, t3, t4
    add a1, a1, t5
    li a2, 500
    bne a1, a2, fail_muldiv_16

    # Scenario 17: MUL with t3, t4 into t3
    li t3, 0x00010001
    li t4, 0x00000100
    mul t3, t3, t4
    li a2, 0x01000100
    bne t3, a2, fail_muldiv_17

    # Scenario 18: MUL with t5, t6 (negative) into t5
    li t5, -10
    li t6, 20
    mul t5, t5, t6
    li a2, -200
    bne t5, a2, fail_muldiv_18

    # Scenario 19: MULHU with t3, t4 (high bits unsigned) into t3
    li t3, 0x80000000
    li t4, 4
    mulhu t3, t3, t4
    li a2, 2
    bne t3, a2, fail_muldiv_19

    # Scenario 20: MULHSU with t5, t6 into t5
    li t5, 0x80000000  # Negative when signed
    li t6, 2
    mulhsu t5, t5, t6
    li a2, 0xFFFFFFFF
    bne t5, a2, fail_muldiv_20

    # Scenario 21: DIV with t3, t4 (negative dividend) into t3
    li t3, -1000
    li t4, 10
    div t3, t3, t4
    li a2, -100
    bne t3, a2, fail_muldiv_21

    # Scenario 22: DIV with t5, t6 (negative divisor) into t5
    li t5, 1000
    li t6, -10
    div t5, t5, t6
    li a2, -100
    bne t5, a2, fail_muldiv_22

    # Scenario 23: REM with t3, t4 into t3
    li t3, 1003
    li t4, 10
    rem t3, t3, t4
    li a2, 3
    bne t3, a2, fail_muldiv_23

    # Scenario 24: REM with t5, t6 (negative dividend) into t5
    li t5, -1003
    li t6, 10
    rem t5, t5, t6
    li a2, -3
    bne t5, a2, fail_muldiv_24

    # Scenario 25: DIV by zero with t3, t4 (result should be -1) into t3
    li t3, 100
    li t4, 0
    div t3, t3, t4
    li a2, -1
    bne t3, a2, fail_muldiv_25

    # Scenario 26: REM by zero with t5, t6 (result should be dividend) into t5
    li t5, 100
    li t6, 0
    rem t5, t5, t6
    li a2, 100
    bne t5, a2, fail_muldiv_26

    # Scenario 27: MUL using only t3
    li t3, 123
    mul t3, t3, t3
    li a2, 15129
    bne t3, a2, fail_muldiv_27

    # Scenario 28: DIV using only t4
    li t4, 10
    div t4, t4, t4
    li a2, 1
    bne t4, a2, fail_muldiv_28

    # Scenario 29: REM using only t5
    li t5, 1234
    rem t5, t5, t5
    li a2, 0
    bne t5, a2, fail_muldiv_29

    # Scenario 30: the most negative number divided by -1
    li t3, 0x80000000
    li t4, -1
    div a1, t3, t4
    li a2, 0x80000000
    bne a1, a2, fail_muldiv_30

    # Scenario 31: DIV overflow (INT_MIN / -1) -> INT_MIN
    li t3, 0x80000000
    li t4, -1
    div t6, t3, t4
    li t5, 0x80000000
    bne t6, t5, fail_muldiv_31

    # Scenario 32: REM overflow pair -> 0
    rem a1, t3, t4
    li a2, 0
    bne a1, a2, fail_muldiv_32

    # Scenario 33: DIVU by zero → 0xFFFFFFFF
    li t3, 12345
    li t4, 0
    divu a1, t3, t4
    li a2, 0xFFFFFFFF
    bne a1, a2, fail_muldiv_33

    # Scenario 34: REMU by zero -> dividend (unsigned semantics)
    remu a1, t3, t4
    li a2, 12345
    bne a1, a2, fail_muldiv_34

    # Success
    li a0, 0
    j test_multiply_divide_end

fail_muldiv_1:
    li a0, 1
    j test_multiply_divide_end
fail_muldiv_2:
    li a0, 2
    j test_multiply_divide_end
fail_muldiv_3:
    li a0, 3
    j test_multiply_divide_end
fail_muldiv_4:
    li a0, 4
    j test_multiply_divide_end
fail_muldiv_5:
    li a0, 5
    j test_multiply_divide_end
fail_muldiv_6:
    li a0, 6
    j test_multiply_divide_end
fail_muldiv_7:
    li a0, 7
    j test_multiply_divide_end
fail_muldiv_8:
    li a0, 8
    j test_multiply_divide_end
fail_muldiv_9:
    li a0, 9
    j test_multiply_divide_end
fail_muldiv_10:
    li a0, 10
    j test_multiply_divide_end
fail_muldiv_11:
    li a0, 11
    j test_multiply_divide_end
fail_muldiv_12:
    li a0, 12
    j test_multiply_divide_end
fail_muldiv_13:
    li a0, 13
    j test_multiply_divide_end
fail_muldiv_14:
    li a0, 14
    j test_multiply_divide_end
fail_muldiv_15:
    li a0, 15
    j test_multiply_divide_end
fail_muldiv_16:
    li a0, 16
    j test_multiply_divide_end
fail_muldiv_17:
    li a0, 17
    j test_multiply_divide_end
fail_muldiv_18:
    li a0, 18
    j test_multiply_divide_end
fail_muldiv_19:
    li a0, 19
    j test_multiply_divide_end
fail_muldiv_20:
    li a0, 20
    j test_multiply_divide_end
fail_muldiv_21:
    li a0, 21
    j test_multiply_divide_end
fail_muldiv_22:
    li a0, 22
    j test_multiply_divide_end
fail_muldiv_23:
    li a0, 23
    j test_multiply_divide_end
fail_muldiv_24:
    li a0, 24
    j test_multiply_divide_end
fail_muldiv_25:
    li a0, 25
    j test_multiply_divide_end
fail_muldiv_26:
    li a0, 26
    j test_multiply_divide_end
fail_muldiv_27:
    li a0, 27
    j test_multiply_divide_end
fail_muldiv_28:
    li a0, 28
    j test_multiply_divide_end
fail_muldiv_29:
    li a0, 29
    j test_multiply_divide_end
fail_muldiv_30:
    li a0, 30
    j test_multiply_divide_end
fail_muldiv_31:
    li a0, 31
    j test_multiply_divide_end
fail_muldiv_32:
    li a0, 32
    j test_multiply_divide_end
fail_muldiv_33:
    li a0, 33
    j test_multiply_divide_end
fail_muldiv_34:
    li a0, 34

test_multiply_divide_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
