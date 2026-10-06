.section .text
.globl _test_start
_test_start:
    addi sp, sp, -16
    sw ra, 12(sp)

    # Scenario 1: AND with t1, t2
    li t1, 0xFF00FF00
    li t2, 0x0FF00FF0
    and a1, t1, t2
    li a2, 0x0F000F00
    bne a1, a2, fail_logic_1

    # Scenario 2: AND with t3, t4
    li t3, 0xAAAAAAAA
    li t4, 0x55555555
    and a1, t3, t4
    li a2, 0
    bne a1, a2, fail_logic_2

    # Scenario 3: OR with t5, t6
    li t5, 0xF0F0F0F0
    li t6, 0x0F0F0F0F
    or a1, t5, t6
    li a2, 0xFFFFFFFF
    bne a1, a2, fail_logic_3

    # Scenario 4: OR with t1, t2
    li t1, 0x12340000
    li t2, 0x00005678
    or a1, t1, t2
    li a2, 0x12345678
    bne a1, a2, fail_logic_4

    # Scenario 5: XOR with t3, t4
    li t3, 0xFFFFFFFF
    li t4, 0xAAAAAAAA
    xor a1, t3, t4
    li a2, 0x55555555
    bne a1, a2, fail_logic_5

    # Scenario 6: XOR with t5, t6 (same values = 0)
    li t5, 0x12345678
    li t6, 0x12345678
    xor a1, t5, t6
    li a2, 0
    bne a1, a2, fail_logic_6

    # Scenario 7: ANDI with t3
    li t3, 0xFFFFFFFF
    andi t3, t3, 0xFF
    li a1, 0xFF
    bne t3, a1, fail_logic_7

    # Scenario 8: ORI with t4
    li t4, 0x12345000
    ori t4, t4, 0x678
    li a1, 0x12345678
    bne t4, a1, fail_logic_8

    # Scenario 9: XORI with t5
    li t5, 0xFFFFFFFF
    xori t5, t5, 0x7FF
    li a1, 0xFFFFF800
    bne t5, a1, fail_logic_9

    # Scenario 10: ANDI with negative immediate t6
    li t6, 0xFFFFFFFF
    andi t6, t6, -1
    li a1, 0xFFFFFFFF
    bne t6, a1, fail_logic_10

    # Scenario 11: Complex logical chain with t3-t6
    li t3, 0xFF00FF00
    li t4, 0x00FF00FF
    li t5, 0xF0F0F0F0
    li t6, 0x0F0F0F0F
    and t3, t3, t5
    and t6, t4, t6
    or t3, t6, t3
    li t4, 0xF00FF00F
    bne t3, t4, fail_logic_11

    # Scenario 12: NOT operation using XOR with -1 on t3
    li t3, 0x12345678
    xori a1, t3, -1
    li a2, 0xEDCBA987
    bne a1, a2, fail_logic_12

    # Success
    li a0, 0
    j test_logical_end

fail_logic_1:
    li a0, 1
    j test_logical_end
fail_logic_2:
    li a0, 2
    j test_logical_end
fail_logic_3:
    li a0, 3
    j test_logical_end
fail_logic_4:
    li a0, 4
    j test_logical_end
fail_logic_5:
    li a0, 5
    j test_logical_end
fail_logic_6:
    li a0, 6
    j test_logical_end
fail_logic_7:
    li a0, 7
    j test_logical_end
fail_logic_8:
    li a0, 8
    j test_logical_end
fail_logic_9:
    li a0, 9
    j test_logical_end
fail_logic_10:
    li a0, 10
    j test_logical_end
fail_logic_11:
    li a0, 11
    j test_logical_end
fail_logic_12:
    li a0, 12
    j test_logical_end

test_logical_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
