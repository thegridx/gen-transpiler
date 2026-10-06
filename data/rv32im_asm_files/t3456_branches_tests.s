.section .text
.globl _test_start
_test_start:
    addi sp, sp, -16
    sw ra, 12(sp)

    # Scenario 1: BEQ taken with t1, t2
    li t1, 100
    li t2, 100
    bne t1, t2, fail_branch_1

    # Scenario 2: BEQ not taken with t1, t2
    li t1, 100
    li t2, 200
    beq t1, t2, fail_branch_2

    # Scenario 3: BNE taken with t3, t4
    li t3, 100
    li t4, 200
    beq t3, t4, fail_branch_3

    # Scenario 4: BNE not taken with t3, t4
    li t3, 100
    li t4, 100
    bne t3, t4, fail_branch_4

    # Scenario 5: BLT taken with t5, t6
    li t5, 50
    li t6, 100
    bge t5, t6, fail_branch_5

    # Scenario 6: BLT not taken with t5, t6
    li t5, 100
    li t6, 50
    blt t5, t6, fail_branch_6

    # Scenario 7: BGE taken with t3, t5
    li t3, 100
    li t5, 100
    blt t3, t5, fail_branch_7

    # Scenario 8: BGE taken (greater) with t4, t6
    li t4, 100
    li t6, 50
    blt t4, t6, fail_branch_8

    # Scenario 9: BLTU taken with t1, t2 (unsigned)
    li t1, 0x00000001
    li t2, 0x80000000  # Large unsigned
    bgeu t1, t2, fail_branch_9

    # Scenario 10: BLTU not taken with t3, t4
    li t3, 0x80000000
    li t4, 0x00000001
    bltu t3, t4, fail_branch_10

    # Scenario 11: BGEU taken (equal) with t5, t6
    li t5, 0xFFFFFFFF
    li t6, 0xFFFFFFFF
    bltu t5, t6, fail_branch_11

    # Scenario 12: BGEU taken (greater) with t3, t4
    li t3, 0xFFFFFFFF
    li t4, 0x7FFFFFFF
    bltu t3, t4, fail_branch_12

    # Scenario 13: BLT with negative numbers t5, t6
    li t5, -100
    li t6, -50
    bge t5, t6, fail_branch_13

    # Scenario 14: BGE with negative and positive t3, t1
    li t3, -50
    li t1, 100
    bge t3, t1, fail_branch_14

    # Scenario 15: Complex branch chain with all special regs
    li t3, 10
    li t4, 20
    li t5, 30
    li t6, 40
    bge t3, t4, fail_branch_15
    bge t4, t5, fail_branch_15
    bge t5, t6, fail_branch_15
    blt t6, t5, fail_branch_15

    # Scenario 16: BEQ zero with t3
    li t3, 0
    bne t3, zero, fail_branch_16

    # Scenario 17: BNE zero with t4
    li t4, 1
    beq t4, zero, fail_branch_17

    # Success
    li a0, 0
    j test_branches_end

fail_branch_1:
    li a0, 1
    j test_branches_end
fail_branch_2:
    li a0, 2
    j test_branches_end
fail_branch_3:
    li a0, 3
    j test_branches_end
fail_branch_4:
    li a0, 4
    j test_branches_end
fail_branch_5:
    li a0, 5
    j test_branches_end
fail_branch_6:
    li a0, 6
    j test_branches_end
fail_branch_7:
    li a0, 7
    j test_branches_end
fail_branch_8:
    li a0, 8
    j test_branches_end
fail_branch_9:
    li a0, 9
    j test_branches_end
fail_branch_10:
    li a0, 10
    j test_branches_end
fail_branch_11:
    li a0, 11
    j test_branches_end
fail_branch_12:
    li a0, 12
    j test_branches_end
fail_branch_13:
    li a0, 13
    j test_branches_end
fail_branch_14:
    li a0, 14
    j test_branches_end
fail_branch_15:
    li a0, 15
    j test_branches_end
fail_branch_16:
    li a0, 16
    j test_branches_end
fail_branch_17:
    li a0, 17
    j test_branches_end

test_branches_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
