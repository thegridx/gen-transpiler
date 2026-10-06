# Tests: JAL, JALR, J (pseudo-instruction), JR (pseudo-instruction)
# Returns: 0 on success, -1 on failure

.text
.globl _test_start

_test_start:
    # Test 1: JAL (jump and link) - verify return address is PC+4
    jal x11, jal_target1
jal_return_point1:              # This is where x11 should point (PC+4)
    j jal_test1_pass            # should return here after the subroutine
jal_target1:
    # Calculate expected return address using PC-relative addressing
    auipc x22, 0                  # Get current PC (address of this instruction)
    addi x22, x22, -4             # Subtract 4 to get address of jal_return_point1
                                  # (jal_target1 - 4 = jal_return_point1)
    bne x11, x22, fail
    jalr x0, x11, 0               # Return using saved address

jal_test1_pass:

    # Test 2: JAL with different link register
    jal x5, jal_target2
jal_return_point2:
    j jal_test2_pass
jal_target2:
    # Verify x5 contains correct return address
    auipc x6, 0                 # Get current PC (address of this auipc instruction)
    addi x6, x6, -4             # Point to jal_return_point2
    bne x5, x6, fail
    jalr x0, x5, 0              # Return

jal_test2_pass:

    # Test 3: JAL with x0 (discard return address)
    jal x0, jal_target3
    # Should continue here (no return expected)
    # x0 should still be 0
    li x10, 0
    bne x0, x10, fail
    j jal_test3_pass
jal_target3:
    # Just continue execution (can't return since x0 used as link reg)
    j jal_test3_pass

jal_test3_pass:

    # Test 4: JALR (jump and link register) - basic
    # First, get target address using PC-relative method
    auipc x10, 0                 # Get current PC
    addi x10, x10, 16             # Point to jalr_target1 (4 instructions ahead)
    jalr x22, x10, 0              # Jump to address in x10
jalr_return_point1:
    j jalr_test1_pass
jalr_target1:
    # Verify return address in x22
    auipc x3, 0                 # Get current PC (address of this auipc instruction)
    addi x3, x3, -4             # Point to jalr_return_point1
    bne x22, x3, fail
    jalr x0, x22, 0              # Return

jalr_test1_pass:

    # Test 5: JALR with non-zero offset - with verification
    auipc x10, 0                 # Get current PC
    addi x10, x10, 8              # Point to wrong_target
    jalr x22, x10, 4              # Jump to (x1 + 4), should skip wrong_target
    j jalr_test2_pass           # Return point
wrong_jalr_test2_target:               # x1 points here
    j fail                  # If offset ignored, we fail here
correct_jalr_test2_target:             # x1 + 4 points here
    jalr x0, x22, 0          # Return (only reachable if offset worked)

jalr_test2_pass:

    # Test 6: Function call/return simulation
    jal x10, simple_function
function_caller_return:
    # Verify function executed
    li x3, 42
    bne x22, x3, fail
    j function_test_pass
simple_function:
    li x22, 42                   # Set return value
    jalr x0, x10, 0              # Return

function_test_pass:

    # Test 7: J pseudo-instruction (JAL with x0)
    j j_target1
    j fail                      # Should not reach
j_target1:
    # Test backward jump
    j backward_jump_setup
backward_target:
    j backward_test_pass
backward_jump_setup:
    j backward_target
    j fail                      # Should not reach

backward_test_pass:

    # Test 8: Multiple JAL calls - verify each saves different return addresses
   jal x20, outer_function
first_return_point:
    # x3 should contain address of first_return_point
    auipc x4, 0                # x4 = address of this auipc instruction
    bne x3, x4, fail           # x3 should equal current PC (first_return_point)
    j test8_pass
outer_function:
    mv x10, x20                 # x10 = first return address
    jal x20, inner_function
second_return_point:
    # x11 should contain address of second_return_point
    auipc x12, 0               # x12 = address of this auipc instruction
    bne x11, x12, fail         # x11 should equal current PC (second_return_point)
    # Return first return address in x3 for main to verify
    mv x3, x10
    jalr x0, x10, 0            # Return to main
inner_function:
    # Return second return address in x11 for outer to verify
    mv x11, x20
    jalr x0, x20, 0

test8_pass:

    # Test 9: Jump table using relative addressing
    auipc x3, 0                # Get current PC
    addi x3, x3, 0x14          # Point to jump_table
    lw x4, 8(x3)               # Load relative offset of entry 2
    add x5, x3, x4             # Add base address to get absolute target
    jalr x0, x5, 0             # Jump to target
jump_table:
   .word 0x00000010
   .word 0x00000014
   .word 0x00000018
   .word 0x0000001c
cases:
    j fail
    j fail
    j case_2_reached
    j fail

case_2_reached:

    # Test 10: Register indirect jump with computation
    li x20, 0xABC             # Test marker value
    auipc x10, 0                    # Get current PC
    addi x10, x10, 36                # Point to computed_target
    jalr x22, x10, 0                 # Jump to computed address
computed_jump_return:
    li x21, 0xABD   # Verify the target function executed correctly
    bne x20, x21, fail          # Should have been incremented by target
    auipc x22, 0                # Verify we got correct return address
    addi x22, x22, -8           # Calculate computed_jump_return address
    bne x2, x22, fail           # Check return address is correct
    j computed_jump_pass
computed_target:
    # Increment the test value to prove we executed here
    addi x20, x20, 1            # 0xABC -> 0xABD
    jalr x0, x22, 0              # Return

computed_jump_pass:
    # Test 11: Return address manipulation
    jal x10, return_addr_test
    # These instructions should be skipped due to return address manipulation
    j fail
    j fail
return_addr_manipulated:       # This is where we should actually return
    j return_addr_pass
return_addr_test:
    # Modify return address to skip 2 instructions (8 bytes)
    addi x10, x10, 8             # Skip 2 instructions after original return point
    jalr x0, x10, 0             # Return to modified address

return_addr_pass:

    # Test 12: Chain of calls - verify return addresses don't get corrupted
    li x20, 0x123              # Test value to pass through chain
    jal x10, chain_start
main_return_point:
    # Verify we got correct return address back
    auipc x22, 0
    bne x19, x22, fail          # x19 should contain main return address
    # Verify test value made it through unchanged
    li x21, 0x123
    bne x20, x21, fail          # Check value preserved
    j test12_pass
chain_start:
    mv x19, x10                  # Save main's return address
    addi x20, x20, 1            # Modify test value: 0x123 -> 0x124
    jal x10, chain_middle
chain_start_return:
    # Verify middle function returned here correctly
    auipc x23, 0
    bne x18, x23, fail          # x18 should contain this address
    addi x20, x20, -1            # Restore test value: 0x124 -> 0x123
    jalr x0, x19, 0             # Return to main
chain_middle:
    mv x18, x10                  # Save chain_start's return address
    addi x20, x20, 1            # Modify test value: 0x124 -> 0x125
    jal x10, chain_end
chain_middle_return:
    # Verify end function returned here correctly
    auipc x24, 0
    bne x17, x24, fail          # x17 should contain this address
    addi x20, x20, -1            # Restore test value: 0x125 -> 0x124
    jalr x0, x18, 0             # Return to chain_start
chain_end:
    mv x17, x10                  # Save chain_middle's return address
    addi x20, x20, 1            # Modify test value: 0x125 -> 0x126
    # Verify we got the expected value
    li x25, 0x126
    bne x20, x25, fail          # Should be 0x126 at this point
    addi x20, x20, -1            # Restore: 0x126 -> 0x125
    jalr x0, x17, 0             # Return to chain_middle

test12_pass:

   # Test 13: JALR with same register as base and link - verify behavior
   li x20, 0x555                  # Test marker
   auipc x10, 0                    # Get current PC
   addi x10, x10, 32                # Point to jalr_self_target
   jalr x10, x10, 0                 # x10 is both base and link register
jalr_self_return:
    # Verify the target function executed
    li x21, 0x666
    bne x20, x21, fail          # Should have been incremented
    # Verify x10 now contains return address (not original target address)
    auipc x22, 0
    addi x22, x22, -12          # Calculate jalr_self_return address
    bne x10, x22, fail           # x10 should now be return address
    j jalr_self_pass
jalr_self_target:
    # Increment test value to prove we executed here
    addi x20, x20, 0x111        # 0x555 -> 0x666
    # x10 now contains return address, original value is lost
    jalr x0, x10, 0              # Return using x10 (which is now return address)

jalr_self_pass:

# Test 14: Maximum and minimum offsets for JALR - verify offset calculation
   li x20, 0x123                 # Test marker
   auipc x10, 0                    # Get current PC
   addi x10, x10, 40                # Point to offset_target
   addi x10, x10, 2047              # Add max positive offset
   jalr x3, x10, -2047             # Subtract max negative offset
offset_return:
    # Verify target executed correctly
    li x21, 0x124
    bne x20, x21, fail          # Should have been incremented
    # Verify return address is correct
    auipc x22, 0
    addi x22, x22, -8           # Calculate offset_return address
    bne x3, x22, fail           # Check return address
    j offset_test_pass
offset_target:
    # Increment test value to prove we executed here
    addi x20, x20, 1            # 0x123 -> 0x124
    jalr x0, x3, 0              # Return

offset_test_pass:

    # Test 15: JAL to get PC (common idiom)
    jal x10, next_instruction
next_instruction:
    auipc x11, 0
    bne x10, x11, fail

    # Success - return 0
    li x17, 0   # a7 = 0 (no panic)
    li x10, 0   # a0 = 0 (success)
    ret

fail:
    # Failure - return -1
    li x17, 0   # a7 = 0 (no panic)
    li x10, -1  # a0 = -1 (failure)
    ret
