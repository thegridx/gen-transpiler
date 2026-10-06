.section .text
.globl _test_start
_test_start:

    addi sp, sp, -16
    sw ra, 12(sp)

    # Scenario 1: JAL basic forward jump
    jal t1, jump_target_1
    li a0, 1  # Should be skipped
    ret
jump_target_1:
    # Verify return address in t1
    auipc t5, 0
    addi t5, t5, -8  # PC of JAL + 4
    bne t1, t5, fail_jump_1

    # Scenario 2: JAL with t3
    jal t3, jump_target_2
    li a0, 2
    ret
jump_target_2:
    auipc t5, 0
    addi t5, t5, -8
    bne t3, t5, fail_jump_2

# Scenario 3: JALR with t4 - verify jump and return address
    la t4, jump_target_3
    auipc t5, 0              # Get current PC (PC of auipc)
    addi t5, t5, 12          # Expected return: auipc PC + 12 = jalr PC + 4
    jalr t6, t4, 0           # Jump, save return in t6 (should be jalr PC + 4)
    li a0, 3                 # Should not execute if jump worked
    ret
jump_target_3:
    # Verify return address in t6 matches expected
    bne t5, t6, fail_jump_3

# Scenario 4: JALR with offset and t6
    la t6, jump_target_4_base
    auipc t1, 0
    addi t1, t1, 12          # Changed from 8 to 12 (jalr PC + 4)
    jalr t2, t6, 12          # Jump to base + 12 bytes (3 nops ahead)
    li a0, 4                 # Should not execute
    ret
jump_target_4_base:
    nop                  # offset 0
    nop                  # offset 4
    nop                  # offset 8
jump_target_4:           # offset 12 - lands here
        bne t1, t2, fail_jump_4   # Verify return address

# Scenario 5: JAL backward jump - verify correct return address
    j skip_backward_target

backward_target:
    # Calculate expected return address
    la a3, return_point_5
    bne t2, a3, fail_jump_5  # Verify return address is correct
    jalr zero, t2, 0

skip_backward_target:
    jal t2, backward_target
return_point_5:
    # If we get here, backward jump and return worked correctly
    nop

# Scenario 6: Function call pattern with t3
    jal t3, test_function_1
    bne t5, a2, fail_jump_6
    j continue_6

test_function_1:
    li t5, 42
    li a2, 42
    jalr zero, t3, 0

continue_6:

    # Scenario 7: Nested calls with t4, t5
    jal t4, outer_func
    li t5, 100
    bne t5, a2, fail_jump_7
    j continue_7

outer_func:
    addi sp, sp, -8
    sw t4, 4(sp)
    jal t5, inner_func
    lw t4, 4(sp)
    addi sp, sp, 8
    jalr zero, t4, 0

inner_func:
    li a2, 100
    jalr zero, t5, 0

continue_7:

# Scenario 8: JALR with zero register (tail call) - verify no return addr saved
    la t6, jump_target_8
    mv t3, ra                # Save original return address
    jalr zero, t6, 0         # Tail call - should not save return address
    li a0, 8                 # Should never execute
    ret

jump_target_8:
    # Verify ra wasn't modified (since we used zero as link reg)
    bne t3, ra, fail_jump_8

# Scenario 9: JAL with large offset - verify we can jump far and return
    jal t1, far_target
# After return, verify we got back
    li t5, 99
    li a2, 99
    bne t5, a2, fail_jump_9
    j continue_9

.align 4
.space 2048  # Create large distance
far_target:
    # Verify we actually jumped (by checking return address is far)
    auipc a3, 0
    sub a4, t1, a3       # Distance from here to return address
    li a5, 2000          # Should be > 2000 bytes away
    blt a4, zero, make_positive_9
    j check_distance_9
make_positive_9:
    sub a4, zero, a4
check_distance_9:
    blt a4, a5, fail_jump_9  # If distance < 2000, test failed
    jalr zero, t1, 0         # Return

continue_9:
# Success
    li a0, 0
    j test_jumps_end

fail_jump_1:
    li a0, 1
    j test_jumps_end
fail_jump_2:
    li a0, 2
    j test_jumps_end
fail_jump_3:
    li a0, 3
    j test_jumps_end
fail_jump_4:
    li a0, 4
    j test_jumps_end
fail_jump_5:
    li a0, 5
    j test_jumps_end
fail_jump_6:
    li a0, 6
    j test_jumps_end
fail_jump_7:
    li a0, 7
    j test_jumps_end
fail_jump_8:
    li a0, 7
    j test_jumps_end
fail_jump_9:
    li a0, 7
    j test_jumps_end

test_jumps_end:
    li a7, 0   # a7 = 0 (no panic)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
