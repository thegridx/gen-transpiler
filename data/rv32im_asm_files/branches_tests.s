# Tests: BEQ, BNE, BLT, BGE, BLTU, BGEU
# Returns: 0 on success, -1 on failure

.text
.globl _test_start

_test_start:
    # Test 1: BEQ - branch when equal (should branch)
    li x10, 100
    li x2, 100
    beq x10, x2, beq_test1_pass
    j fail
beq_test1_pass:

    # Test 2: BEQ - branch when not equal (should not branch)
    li x10, 100
    li x2, 200
    beq x10, x2, fail    # Should not branch

    # Test 3: BNE - branch when not equal (should branch)
    li x10, 100
    li x2, 200
    bne x10, x2, bne_test1_pass
    j fail
bne_test1_pass:

    # Test 4: BNE - branch when equal (should not branch)
    li x10, 100
    li x2, 100
    bne x10, x2, fail    # Should not branch

    # Test 5: BLT - branch when less than (should branch)
    li x10, 50
    li x2, 100
    blt x10, x2, blt_test1_pass
    j fail
blt_test1_pass:

    # Test 6: BLT - branch when greater (should not branch)
    li x10, 100
    li x2, 50
    blt x10, x2, fail    # Should not branch

    # Test 7: BLT - branch when equal (should not branch)
    li x10, 100
    li x2, 100
    blt x10, x2, fail    # Should not branch

    # Test 8: BLT with negative numbers
    li x10, -100
    li x2, -50
    blt x10, x2, blt_test2_pass
    j fail
blt_test2_pass:

    # Test 9: BLT negative vs positive
    li x10, -10
    li x2, 10
    blt x10, x2, blt_test3_pass
    j fail
blt_test3_pass:

    # Test 10: BGE - branch when greater or equal (should branch - greater)
    li x10, 100
    li x2, 50
    bge x10, x2, bge_test1_pass
    j fail
bge_test1_pass:

    # Test 11: BGE - branch when equal (should branch)
    li x10, 100
    li x2, 100
    bge x10, x2, bge_test2_pass
    j fail
bge_test2_pass:

    # Test 12: BGE - should not branch when less
    li x10, 50
    li x2, 100
    bge x10, x2, fail    # Should not branch

    # Test 13: BLTU - unsigned comparison (different from signed)
    li x10, -1           # 0xFFFFFFFF (large positive in unsigned)
    li x2, 1
    bltu x10, x2, fail   # Should not branch (-1 is large unsigned)

    # Test 14: BLTU - normal unsigned comparison
    li x10, 50
    li x2, 100
    bltu x10, x2, bltu_test1_pass
    j fail
bltu_test1_pass:

    # Test 15: BLTU - edge case with large unsigned values
    li x10, 0x7FFFFFFF   # Max positive signed int
    li x2, 0x80000000   # Min negative signed int (large positive unsigned)
    bltu x10, x2, bltu_test2_pass
    j fail
bltu_test2_pass:

    # Test 16: BGEU - unsigned greater or equal
    li x10, 100
    li x2, 50
    bgeu x10, x2, bgeu_test1_pass
    j fail
bgeu_test1_pass:

    # Test 17: BGEU - equal case
    li x10, 100
    li x2, 100
    bgeu x10, x2, bgeu_test2_pass
    j fail
bgeu_test2_pass:

    # Test 18: BGEU - should not branch
    li x10, 50
    li x2, 100
    bgeu x10, x2, fail   # Should not branch

    # Test 19: Branch with x0 register (always 0)
    li x10, 0
    beq x0, x10, beq_x0_pass
    j fail
beq_x0_pass:

    # Test 20: Complex branch sequence - nested conditions
    li x10, 10
    li x2, 20
    li x3, 30

    blt x10, x2, nested_test1
    j fail
nested_test1:
    blt x2, x3, nested_test2
    j fail
nested_test2:
    bge x3, x2, nested_test3
    j fail
nested_test3:
    bge x2, x10, nested_pass
    j fail
nested_pass:

    # Test 21: Branch distance test - backward branch
    j forward_branch
backward_target:
    j branch_distance_pass
forward_branch:
    li x10, 100
    li x2, 100
    beq x10, x2, backward_target
    j fail
branch_distance_pass:

    # Test 22: Maximum branch distances (within ±4KB range)
    # Forward branch test
    li x10, 1
    li x2, 1
    beq x10, x2, far_forward
    j fail

    # Add some padding instructions to test branch distance
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop

far_forward:
    # Test 23: Branch condition with all register combinations
    li x10, 42
    li x28, 42
    beq x10, x28, reg_combo_pass
    j fail
reg_combo_pass:

    # Test 24: Edge values for signed comparison
    li x10, 0x7FFFFFFF   # Maximum positive
    li x2, 0x80000000   # Maximum negative
    blt x2, x10, signed_edge_pass  # Max negative < Max positive
    j fail
signed_edge_pass:

    # Test 25: Edge values for unsigned comparison
    li x10, 0xFFFFFFFF   # Maximum unsigned
    li x2, 0
    bgeu x10, x2, unsigned_edge_pass  # Max unsigned >= 0
    j fail
unsigned_edge_pass:

    # Test 26: Branch prediction stress test - alternating pattern
    li x5, 0            # Counter
    li x6, 10           # Limit
branch_loop:
    andi x7, x5, 1      # Check if odd/even
    beq x7, x0, even_case
    # Odd case
    addi x5, x5, 1
    j branch_loop_check
even_case:
    # Even case
    addi x5, x5, 1
branch_loop_check:
    blt x5, x6, branch_loop

    # Test 27: Zero register special cases
    li x10, 100
    bne x0, x10, zero_reg_pass1
    j fail
zero_reg_pass1:
    beq x0, x0, zero_reg_pass2
    j fail
zero_reg_pass2:

    # Success - return 0
    li x17, 0   # a7 = 0 (no panic)
    li x10, 0   # a0 = 0 (success)
    ret

fail:
    # Failure - return -1
    li x17, 0   # a7 = 0 (no panic)
    li x10, -1  # a0 = -1 (failure)
    ret
