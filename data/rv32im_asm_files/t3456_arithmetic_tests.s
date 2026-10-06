.section .text
.globl _test_start
_test_start:
    addi sp, sp, -16
    sw ra, 12(sp)

    # Scenario 1: ADD with t1, t2
    li t1, 100
    li t2, 200
    add a1, t1, t2
    li a2, 300
    bne a1, a2, fail_arith_1

    # Scenario 2: ADD with t3, t4
    li t3, 0x12345678
    li t4, 0x11111111
    add a1, t3, t4
    li a2, 0x23456789
    bne a1, a2, fail_arith_2

    # Scenario 3: ADD with t5, t6
    li t5, -100
    li t6, 50
    add a1, t5, t6
    li a2, -50
    bne a1, a2, fail_arith_3

    # Scenario 4: SUB with t1, t2
    li t1, 300
    li t2, 100
    sub a1, t1, t2
    li a2, 200
    bne a1, a2, fail_arith_4

    # Scenario 5: SUB with t3, t4
    li t3, 0x23456789
    li t4, 0x11111111
    sub a1, t3, t4
    li a2, 0x12345678
    bne a1, a2, fail_arith_5

    # Scenario 6: SUB with t5, t6 (negative result)
    li t5, 50
    li t6, 100
    sub a1, t5, t6
    li a2, -50
    bne a1, a2, fail_arith_6

    # Scenario 7: ADDI with t3
    li t3, 100
    addi t3, t3, 50
    li a1, 150
    bne t3, a1, fail_arith_7

    # Scenario 8: ADDI with negative immediate t4
    li t4, 100
    addi t4, t4, -50
    li a1, 50
    bne t4, a1, fail_arith_8

    # Scenario 9: SLT with t1, t2 (true)
    li t1, 50
    li t2, 100
    slt a1, t1, t2
    li a2, 1
    bne a1, a2, fail_arith_9

    # Scenario 10: SLT with t3, t4 (false)
    li t3, 100
    li t4, 50
    slt a1, t3, t4
    li a2, 0
    bne a1, a2, fail_arith_10

    # Scenario 11: SLTU with t5, t6 (unsigned)
    li t5, 0x7FFFFFFF
    li t6, 0x80000000
    sltu a1, t5, t6
    li a2, 1
    bne a1, a2, fail_arith_11

    # Scenario 12: SLTI with t3
    li t3, 50
    slti a1, t3, 100
    li a2, 1
    bne a1, a2, fail_arith_12

    # Scenario 13: SLTIU with t4
    li t4, 0x7FFFFFFF
    sltiu a1, t4, -1  # -1 as immediate = 0xFFFFFFFF
    li a2, 1
    bne a1, a2, fail_arith_13

    # Scenario 14: Overflow test with t5, t6
    li t5, 0x7FFFFFFF
    li t6, 1
    add a1, t5, t6
    li a2, 0x80000000
    bne a1, a2, fail_arith_14

    # Scenario 15: Chained operations with all special regs
    li t3, 10
    li t4, 20
    li t5, 30
    li t6, 40
    add a1, t3, t4
    add a1, a1, t5
    add a1, a1, t6
    li a2, 100
    bne a1, a2, fail_arith_15

    # Scenario 16: ADD with both operands in t4/t5, dst=t6
    li t4, 11
    li t5, 22
    add t6, t4, t5
    li a1, 33
    bne t6, a1, fail_arith_16

    # Scenario 17: SUB with operands t6,t4, dst=t5
    # 33 - 11 = 22
    sub t5, t6, t4
    li a1, 22
    bne t5, a1, fail_arith_17

    # Scenario 18: ADDI 12-bit boundaries on spilled reg (dst=t4)
    li t4, 0
    addi t4, t4, 2047      # max positive 12-bit
    addi t4, t4, -2048     # min negative 12-bit
    li a1, -1
    bne t4, a1, fail_arith_18


    # Success
    li a0, 0
    j test_arithmetic_end

fail_arith_1:
    li a0, 1
    j test_arithmetic_end
fail_arith_2:
    li a0, 2
    j test_arithmetic_end
fail_arith_3:
    li a0, 3
    j test_arithmetic_end
fail_arith_4:
    li a0, 4
    j test_arithmetic_end
fail_arith_5:
    li a0, 5
    j test_arithmetic_end
fail_arith_6:
    li a0, 6
    j test_arithmetic_end
fail_arith_7:
    li a0, 7
    j test_arithmetic_end
fail_arith_8:
    li a0, 8
    j test_arithmetic_end
fail_arith_9:
    li a0, 9
    j test_arithmetic_end
fail_arith_10:
    li a0, 10
    j test_arithmetic_end
fail_arith_11:
    li a0, 11
    j test_arithmetic_end
fail_arith_12:
    li a0, 12
    j test_arithmetic_end
fail_arith_13:
    li a0, 13
    j test_arithmetic_end
fail_arith_14:
    li a0, 14
    j test_arithmetic_end
fail_arith_15:
    li a0, 15
    j test_arithmetic_end
fail_arith_16:
    li a0, 16
    j test_arithmetic_end
fail_arith_17:
    li a0, 17
    j test_arithmetic_end
fail_arith_18:
    li a0, 18
    j test_arithmetic_end

test_arithmetic_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
