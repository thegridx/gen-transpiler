.section .text
.globl _test_start
_test_start:
    addi sp, sp, -16
    sw ra, 12(sp)

    # Scenario 1: LUI with t1
    lui t1, 0x12345
    li a1, 0x12345000
    bne t1, a1, fail_lui_1

    # Scenario 2: LUI with t3
    lui t3, 0xABCDE
    li a1, 0xABCDE000
    bne t3, a1, fail_lui_2

    # Scenario 3: LUI with t4
    lui t4, 0x00001
    li a1, 0x00001000
    bne t4, a1, fail_lui_3

    # Scenario 4: LUI with t5
    lui t5, 0xFFFFF
    li a1, 0xFFFFF000
    bne t5, a1, fail_lui_4

    # Scenario 5: LUI with t6 (zero upper immediate)
    lui t6, 0x00000
    li a1, 0x00000000
    bne t6, a1, fail_lui_5

    # Scenario 6: AUIPC with t1 (basic - should be close)
    auipc t1, 0
    auipc t2, 0
    # t1 and t2 should have PC values 4 bytes apart
    sub a1, t2, t1
    li a2, 4
    bne a1, a2, fail_lui_6

    # Scenario 7: AUIPC with t3 (offset by 1 page)
    auipc t3, 1          # PC + 0x1000
    auipc a1, 0          # PC + 4 (next instruction)
    sub a2, t3, a1       # Should be 0x1000 - 4 = 0xFFC
    li a3, 0xFFC
    bne a2, a3, fail_lui_7

    # Scenario 8: AUIPC with t4 (larger offset)
    auipc t4, 0x100      # PC + 0x100000
    auipc a1, 0          # PC + 4
    sub a2, t4, a1       # Should be 0x100000 - 4
    li a3, 0x100000
    addi a3, a3, -4
    bne a2, a3, fail_lui_8

    # Scenario 9: AUIPC with t5 (verify actual PC capture)
    auipc t5, 0
    mv a1, t5
    # Just verify we got a reasonable address (non-zero)
    beqz a1, fail_lui_9

    # Scenario 10: LUI then ADDI pattern with t6
    lui t6, 0x12345
    addi t6, t6, 0x678
    li a1, 0x12345678
    bne t6, a1, fail_lui_10

    # Scenario 11: LUI with all special regs in sequence
    lui t3, 0x11111
    lui t4, 0x22222
    lui t5, 0x33333
    lui t6, 0x44444
    li a1, 0x11111000
    bne t3, a1, fail_lui_11
    li a1, 0x22222000
    bne t4, a1, fail_lui_11
    li a1, 0x33333000
    bne t5, a1, fail_lui_11
    li a1, 0x44444000
    bne t6, a1, fail_lui_11

    # Success
    li a0, 0
    j test_lui_auipc_end

fail_lui_1:
    li a0, 1
    j test_lui_auipc_end
fail_lui_2:
    li a0, 2
    j test_lui_auipc_end
fail_lui_3:
    li a0, 3
    j test_lui_auipc_end
fail_lui_4:
    li a0, 4
    j test_lui_auipc_end
fail_lui_5:
    li a0, 5
    j test_lui_auipc_end
fail_lui_6:
    li a0, 6
    j test_lui_auipc_end
fail_lui_7:
    li a0, 7
    j test_lui_auipc_end
fail_lui_8:
    li a0, 8
    j test_lui_auipc_end
fail_lui_9:
    li a0, 9
    j test_lui_auipc_end
fail_lui_10:
    li a0, 10
    j test_lui_auipc_end
fail_lui_11:
    li a0, 11
    j test_lui_auipc_end

test_lui_auipc_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
