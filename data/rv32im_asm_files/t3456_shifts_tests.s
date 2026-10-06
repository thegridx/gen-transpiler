.section .text
.globl _test_start
_test_start:
    addi sp, sp, -16
    sw ra, 12(sp)

    # Scenario 1: SLL with t1, t2
    li t1, 0x00000001
    li t2, 4
    sll a1, t1, t2
    li a2, 0x00000010
    bne a1, a2, fail_shift_1

    # Scenario 2: SLL with t3, t4
    li t3, 0x12345678
    li t4, 8
    sll a1, t3, t4
    li a2, 0x34567800
    bne a1, a2, fail_shift_2

    # Scenario 3: SRL with t5, t6
    li t5, 0x80000000
    li t6, 4
    srl a1, t5, t6
    li a2, 0x08000000
    bne a1, a2, fail_shift_3

    # Scenario 4: SRL with t1, t2 (shift out bits)
    li t1, 0xFFFFFFFF
    li t2, 4
    srl a1, t1, t2
    li a2, 0x0FFFFFFF
    bne a1, a2, fail_shift_4

    # Scenario 5: SRA with t3, t4 (sign extend)
    li t3, 0x80000000
    li t4, 4
    sra a1, t3, t4
    li a2, 0xF8000000
    bne a1, a2, fail_shift_5

    # Scenario 6: SRA with t5, t6 (positive number)
    li t5, 0x7FFFFFFF
    li t6, 4
    sra a1, t5, t6
    li a2, 0x07FFFFFF
    bne a1, a2, fail_shift_6

    # Scenario 7: SLLI with t3
    li t3, 1
    slli t3, t3, 16
    li a1, 0x00010000
    bne t3, a1, fail_shift_7

    # Scenario 8: SRLI with t4
    li t4, 0x12340000
    srli t4, t4, 16
    li a1, 0x00001234
    bne t4, a1, fail_shift_8

    # Scenario 9: SRAI with t5 (negative)
    li t5, 0xFFFF0000
    srai t5, t5, 8
    li a1, 0xFFFFFF00
    bne t5, a1, fail_shift_9

    # Scenario 10: SLLI by 31 with t6
    li t6, 1
    slli t6, t6, 31
    li a1, 0x80000000
    bne t6, a1, fail_shift_10

    # Scenario 11: SRLI by 31 with t3
    li t3, 0x80000000
    srli t3, t3, 31
    li a1, 1
    bne t3, a1, fail_shift_11

    # Scenario 12: SLL with shift amount > 31 (should use only lower 5 bits)
    li t4, 1
    li t5, 33  # Only uses lower 5 bits = 1
    sll a1, t4, t5
    li a2, 2
    bne a1, a2, fail_shift_12

    # Scenario 13: Shift left then right with t6, t3
    li t6, 0x12345678
    li t3, 16
    sll a1, t6, t3
    srl a1, a1, t3
    li a2, 0x00005678
    bne a1, a2, fail_shift_13

    # Scenario 14: Combined shifts with all special regs
    li t3, 0x0000FF00
    li t4, 0x000000FF
    li t5, 8
    sll t6, t4, t5       # t6 = 0xFF << 8 = 0x0000FF00
    or a1, t3, t4        # a1 = 0x0000FF00 | 0x000000FF = 0x0000FFFF
    li a2, 0x0000FFFF
    bne a1, a2, fail_shift_14

    # Success
    li a0, 0
    j test_shifts_end

fail_shift_1:
    li a0, 1
    j test_shifts_end
fail_shift_2:
    li a0, 2
    j test_shifts_end
fail_shift_3:
    li a0, 3
    j test_shifts_end
fail_shift_4:
    li a0, 4
    j test_shifts_end
fail_shift_5:
    li a0, 5
    j test_shifts_end
fail_shift_6:
    li a0, 6
    j test_shifts_end
fail_shift_7:
    li a0, 7
    j test_shifts_end
fail_shift_8:
    li a0, 8
    j test_shifts_end
fail_shift_9:
    li a0, 9
    j test_shifts_end
fail_shift_10:
    li a0, 10
    j test_shifts_end
fail_shift_11:
    li a0, 11
    j test_shifts_end
fail_shift_12:
    li a0, 12
    j test_shifts_end
fail_shift_13:
    li a0, 13
    j test_shifts_end
fail_shift_14:
    li a0, 14
    j test_shifts_end

test_shifts_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
