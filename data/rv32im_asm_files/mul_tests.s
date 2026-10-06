# test multiplication and division (M-extension) instructions
# Returns 0 on success, -1 on failure in a7
.text
.globl _test_start

_test_start:
    # --------------------
    # Multiplication Tests
    # --------------------
    # Test 1: MUL (10 * 20 = 200)
    li x5, 10
    li x6, 20
    mul x7, x5, x6
    li x8, 200
    bne x7, x8, fail

    # Test 2: MUL with negative number (-5 * 3 = -15)
    li x5, -5
    li x6, 3
    mul x7, x5, x6
    li x8, -15
    bne x7, x8, fail

    # Test 3: MUL overflow (MIN_INT * -1 = MIN_INT)
    li x5, 0x80000000   # MIN_INT
    li x6, -1
    mul x7, x5, x6
    li x8, 0x80000000
    bne x7, x8, fail

    # Test 4: MUL overflow wrap (0x80000000 * 2 -> 0)
    li x5, 0x80000000
    li x6, 2
    mul x7, x5, x6
    li x8, 0
    bne x7, x8, fail

    # Test 5: MULH signed high bits (-5 * 3 -> high = -1)
    li x5, -5
    li x6, 3
    mulh x7, x5, x6
    li x8, -1
    bne x7, x8, fail

    # Test 6: MULH positive high bits (0x00010000 * 0x00010000 -> high = 1)
    li x5, 0x00010000
    li x6, 0x00010000
    mulh x7, x5, x6
    li x8, 1
    bne x7, x8, fail

    # Test 7: MULHU unsigned high bits (0xFFFFFFFF * 2 -> high = 1)
    li x5, 0xFFFFFFFF
    li x6, 2
    mulhu x7, x5, x6
    li x8, 1
    bne x7, x8, fail

    # Test 8: MULHSU mixed signed/unsigned (-1 * 2 -> high = -1)
    li x5, -1        # signed -1
    li x6, 2         # unsigned 2
    mulhsu x7, x5, x6
    li x8, -1
    bne x7, x8, fail

    # --------------------
    # Division Tests
    # --------------------
    # Test 9: DIV (10 / 3 = 3)
    li x5, 10
    li x6, 3
    div x7, x5, x6
    li x8, 3
    bne x7, x8, fail

    # Test 10: DIV negative (-15 / 4 = -3)
    li x5, -15
    li x6, 4
    div x7, x5, x6
    li x8, -3
    bne x7, x8, fail

    # Test 11: DIV by zero (10 / 0 = -1)
    li x5, 10
    li x6, 0
    div x7, x5, x6
    li x8, -1
    bne x7, x8, fail

    # Test 12: DIV overflow (-2^31 / -1 = -2^31)
    li x5, 0x80000000   # -2^31
    li x6, -1
    div x7, x5, x6
    li x8, 0x80000000
    bne x7, x8, fail

    # Test 13: DIVU (10 / 3 = 3)
    li x5, 10
    li x6, 3
    divu x7, x5, x6
    li x8, 3
    bne x7, x8, fail

    # Test 14: DIVU large unsigned (0x80000000 / 2 = 0x40000000)
    li x5, 0x80000000
    li x6, 2
    divu x7, x5, x6
    li x8, 0x40000000
    bne x7, x8, fail

    # Test 15: DIVU by zero (10 / 0 = 0xFFFFFFFF)
    li x5, 10
    li x6, 0
    divu x7, x5, x6
    li x8, 0xFFFFFFFF
    bne x7, x8, fail

    # --------------------
    # Remainder Tests
    # --------------------
    # Test 16: REM (10 %% 3 = 1)
    li x5, 10
    li x6, 3
    rem x7, x5, x6
    li x8, 1
    bne x7, x8, fail

    # Test 17: REM negative (-13 %% 4 = -1)
    li x5, -13
    li x6, 4
    rem x7, x5, x6
    li x8, -1
    bne x7, x8, fail

    # Test 18: REM by zero (10 %% 0 = 10)
    li x5, 10
    li x6, 0
    rem x7, x5, x6
    li x8, 10
    bne x7, x8, fail

    # Test 19: REM overflow (MIN_INT %% -1 = 0)
    li x5, 0x80000000   # MIN_INT
    li x6, -1
    rem x7, x5, x6
    li x8, 0
    bne x7, x8, fail

    # Test 20: REMU (10 %% 3 = 1)
    li x5, 10
    li x6, 3
    remu x7, x5, x6
    li x8, 1
    bne x7, x8, fail

    # Test 21: REMU large unsigned (0x80000005 %% 4 = 1)
    li x5, 0x80000005
    li x6, 4
    remu x7, x5, x6
    li x8, 1
    bne x7, x8, fail

    # Test 22: REMU by zero (0xABCDE %% 0 = 0xABCDE)
    li x5, 0xABCDE
    li x6, 0
    remu x7, x5, x6
    li x8, 0xABCDE
    bne x7, x8, fail

    # All tests passed
    li x17, 0   # a7 = 0 (no panic)
    li x10, 0   # a0 = 0 (success)
    ret

fail:
    li x17, 0   # a7 = 0 (no panic)
    li x10, -1  # a0 = -1 (failure)
    ret
