.section .text
.globl _test_start
_test_start:
    addi sp, sp, -64
    sw ra, 60(sp)

    # Setup test data area
    addi t0, sp, 0
    li t1, 0x12345678
    li t2, 0xABCDEF01
    li t3, 0x11111111
    li t4, 0x22222222
    li t5, 0x33333333
    li t6, 0x44444444

    # Scenario 1: SW/LW with regular registers (t1, t2)
    sw t1, 0(t0)
    lw t2, 0(t0)
    bne t1, t2, fail_scenario_1

    # Scenario 2: SW/LW with t3
    sw t3, 4(t0)
    lw a1, 4(t0)
    bne t3, a1, fail_scenario_2

    # Scenario 3: SW/LW with t4
    sw t4, 8(t0)
    lw a1, 8(t0)
    bne t4, a1, fail_scenario_3

    # Scenario 4: SW/LW with t5
    sw t5, 12(t0)
    lw a1, 12(t0)
    bne t5, a1, fail_scenario_4

    # Scenario 5: SW/LW with t6
    sw t6, 16(t0)
    lw a1, 16(t0)
    bne t6, a1, fail_scenario_5

    # Scenario 6: SH/LH with t3
    li t3, 0x00001234
    sh t3, 20(t0)
    lh a1, 20(t0)
    bne t3, a1, fail_scenario_6

    # Scenario 7: SH/LHU with t4
    li t4, 0x0000FFFF
    sh t4, 22(t0)
    lhu a1, 22(t0)
    bne t4, a1, fail_scenario_7

    # Scenario 8: SB/LB with t5
    li t5, 0x0000007F
    sb t5, 24(t0)
    lb a1, 24(t0)
    bne t5, a1, fail_scenario_8

    # Scenario 9: SB/LBU with t6
    li t6, 0x000000FF
    sb t6, 25(t0)
    lbu a1, 25(t0)
    bne t6, a1, fail_scenario_9

    # Scenario 10: Negative offset with t3
    addi t0, sp, 32
    li t3, 0x55555555
    sw t3, -4(t0)
    lw a1, -4(t0)
    bne t3, a1, fail_scenario_10

    # Scenario 11: Cross-register store/load (t4 -> t5)
    li t4, 0x66666666
    sw t4, 28(sp)
    lw t5, 28(sp)
    bne t4, t5, fail_scenario_11

    # Scenario 12: LW with t3 as base register
    li t3, 0x77777777
    addi t4, sp, 32
    sw t3, 0(t4)
    mv a1, t4
    lw a2, 0(a1)
    bne t3, a2, fail_scenario_12

    # Scenario 13: Multiple stores to same location
    li t3, 0xAAAAAAAA
    li t4, 0xBBBBBBBB
    sw t3, 36(sp)
    sw t4, 36(sp)
    lw a1, 36(sp)
    bne t4, a1, fail_scenario_13

    # Scenario 14: Sign extension test with LH and t5
    li t5, 0x00008000
    sh t5, 40(sp)
    lh a1, 40(sp)
    li a2, 0xFFFF8000
    bne a1, a2, fail_scenario_14

    # Scenario 15: Sign extension test with LB and t6
    li t6, 0x00000080
    sb t6, 42(sp)
    lb a1, 42(sp)
    li a2, 0xFFFFFF80
    bne a1, a2, fail_scenario_15

    # Success
    li a0, 0
    j test_load_store_end

fail_scenario_1:
    li a0, 1
    j test_load_store_end
fail_scenario_2:
    li a0, 2
    j test_load_store_end
fail_scenario_3:
    li a0, 3
    j test_load_store_end
fail_scenario_4:
    li a0, 4
    j test_load_store_end
fail_scenario_5:
    li a0, 5
    j test_load_store_end
fail_scenario_6:
    li a0, 6
    j test_load_store_end
fail_scenario_7:
    li a0, 7
    j test_load_store_end
fail_scenario_8:
    li a0, 8
    j test_load_store_end
fail_scenario_9:
    li a0, 9
    j test_load_store_end
fail_scenario_10:
    li a0, 10
    j test_load_store_end
fail_scenario_11:
    li a0, 11
    j test_load_store_end
fail_scenario_12:
    li a0, 12
    j test_load_store_end
fail_scenario_13:
    li a0, 13
    j test_load_store_end
fail_scenario_14:
    li a0, 14
    j test_load_store_end
fail_scenario_15:
    li a0, 15
    j test_load_store_end

test_load_store_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 60(sp)
    addi sp, sp, 64
    ret
