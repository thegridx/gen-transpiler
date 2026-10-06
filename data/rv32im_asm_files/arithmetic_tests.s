# Tests: ADD, ADDI, SUB, SLT, SLTI, SLTU, SLTIU
# Returns: 0 on success, -1 on failure

.text
.globl _test_start

_test_start:
    # Test 1: Basic ADD operations
    li x10, 100
    li x2, 200
    add x3, x10, x2
    li x4, 300
    bne x3, x4, fail

    # Test 2: ADD with negative numbers
    li x10, -50
    li x2, 30
    add x3, x10, x2
    li x4, -20
    bne x3, x4, fail

    # Test 3: ADD overflow (should wrap around)
    li x10, 0x7FFF
    li x2, 1
    add x3, x10, x2
    li x4, 0x8000
    bne x3, x4, fail
    li x10, 0x7FFFFFFF
    li x2, 2
    add x3, x10, x2
    li x4, -0x7FFFFFFF
    bne x3, x4, fail

    # Test 4: ADDI with positive immediate
    li x10, 100
    addi x2, x10, 50
    li x3, 150
    bne x2, x3, fail

    # Test 5: ADDI with negative immediate
    li x10, 100
    addi x2, x10, -25
    li x3, 75
    bne x2, x3, fail

    # Test 6: ADDI with maximum positive immediate (2047)
    li x10, 100
    addi x2, x10, 2047
    li x3, 2147
    bne x2, x3, fail

    # Test 7: ADDI with minimum negative immediate (-2048)
    li x10, 100
    addi x2, x10, -2048
    li x3, -1948
    bne x2, x3, fail

    # Test 8: SUB basic operation
    li x10, 300
    li x2, 100
    sub x3, x10, x2
    li x4, 200
    bne x3, x4, fail

    # Test 9: SUB with negative result
    li x10, 100
    li x2, 200
    sub x3, x10, x2
    li x4, -100
    bne x3, x4, fail

    # Test 10: SUB underflow
    li x10, 0x8000
    li x2, 1
    sub x3, x10, x2
    li x4, 0x7FFF
    bne x3, x4, fail

    # Test 11: SLT (set less than) - true case
    li x10, 10
    li x2, 20
    slt x3, x10, x2
    li x4, 1
    bne x3, x4, fail

    # Test 12: SLT - false case
    li x10, 20
    li x2, 10
    slt x3, x10, x2
    li x4, 0
    bne x3, x4, fail

    # Test 13: SLT with negative numbers
    li x10, -10
    li x2, 5
    slt x3, x10, x2
    li x4, 1
    bne x3, x4, fail

    # Test 14: SLTI (set less than immediate)
    li x10, 10
    slti x2, x10, 20
    li x3, 1
    bne x2, x3, fail

    # Test 15: SLTI with negative immediate
    li x10, 10
    slti x2, x10, -5
    li x3, 0
    bne x2, x3, fail

    # Test 16: SLTU (unsigned comparison) - different from signed
    li x10, -1          # 0xFFFF (large unsigned)
    li x2, 1
    sltu x3, x10, x2    # Should be 0 (false) since -1 is large unsigned
    li x4, 0
    bne x3, x4, fail

    # Test 17: SLTU - true case
    li x10, 10
    li x2, 20
    sltu x3, x10, x2
    li x4, 1
    bne x3, x4, fail

    # Test 18: SLTIU (unsigned immediate comparison)
    li x10, 10
    sltiu x2, x10, 20
    li x3, 1
    bne x2, x3, fail

    # Test 19: Register x0 always reads as 0
    li x10, 100
    add x0, x10, x10     # Try to write to x0
    li x2, 0
    bne x0, x2, fail   # x0 should still be 0

    # Test 20: Using all registers x2-x28 (register coverage)
    li x2, 2
    li x3, 3
    li x4, 4
    li x5, 5
    li x6, 6
    li x7, 7
    li x8, 8
    li x9, 9
    li x10, 10
    li x11, 11
    li x12, 12
    li x13, 13
    li x14, 14
    li x15, 15
    li x16, 16
    li x17, 17
    li x18, 18
    li x19, 19
    li x20, 20
    li x21, 21
    li x22, 22
    li x23, 23
    li x24, 24
    li x25, 25
    li x26, 26
    li x27, 27
    li x28, 28

    # Verify a few of them
    li x10, 15
    bne x15, x10, fail
    li x10, 28
    bne x28, x10, fail

    # Success - return 0
    li x17, 0   # a7 = 0 (no panic)
    li x10, 0   # a0 = 0 (success)
    ret

fail:
    # Failure - return -1
    li x17, 0   # a7 = 0 (no panic)
    li x10, -1  # a0 = -1 (failure)
    ret
