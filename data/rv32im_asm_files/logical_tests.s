# Tests: AND, ANDI, OR, ORI, XOR, XORI, SLL, SLLI, SRL, SRLI, SRA, SRAI
# Returns: 0 on success, -1 on failure

.text
.globl _test_start

_test_start:
    # Test 1: AND basic operation
    li x10, 0xF0F0F0F0
    li x2, 0x0F0F0F0F
    and x3, x10, x2
    li x4, 0
    bne x3, x4, fail

    # Test 2: AND with same values
    li x10, 0xAAAAAAAA
    li x2, 0xAAAAAAAA
    and x3, x10, x2
    bne x3, x10, fail

    # Test 3: ANDI with immediate
    li x10, 0xFFFFFFFF
    andi x2, x10, 0x0F0
    li x3, 0x0F0
    bne x2, x3, fail

    # Test 4: ANDI with all bits set immediate
    li x10, 0x12345678
    andi x2, x10, 0x7FF  # 11 bits set
    li x3, 0x678
    bne x2, x3, fail

    # Test 5: OR basic operation
    li x10, 0xF0F0F0F0
    li x2, 0x0F0F0F0F
    or x3, x10, x2
    li x4, 0xFFFFFFFF
    bne x3, x4, fail

    # Test 6: OR with zero
    li x10, 0x12345678
    li x2, 0
    or x3, x10, x2
    bne x3, x10, fail

    # Test 7: ORI with immediate
    li x10, 0x12340000
    ori x2, x10, 0x678
    li x3, 0x12340678
    bne x2, x3, fail

    # Test 8: XOR basic operation
    li x10, 0xAAAAAAAA
    li x2, 0x55555555
    xor x3, x10, x2
    li x4, 0xFFFFFFFF
    bne x3, x4, fail

    # Test 9: XOR with self (should be 0)
    li x10, 0x12345678
    xor x2, x10, x10
    li x3, 0
    bne x2, x3, fail

    # Test 10: XORI with immediate
    li x10, 0xAAAAA000
    xori x2, x10, 0x555
    li x3, 0xAAAAA555
    bne x2, x3, fail

    # Test 11: SLL (shift left logical) basic
    li x10, 0x12345678
    li x2, 4
    sll x3, x10, x2
    li x4, 0x23456780
    bne x3, x4, fail

    # Test 12: SLL with maximum shift (31 bits for RV32)
    li x10, 1
    li x2, 31
    sll x3, x10, x2
    li x4, 0x80000000
    bne x3, x4, fail

    # Test 13: SLLI (shift left logical immediate)
    li x10, 0x00012345
    slli x2, x10, 8
    li x3, 0x01234500
    bne x2, x3, fail

    # Test 14: SLLI with maximum immediate (31)
    li x10, 1
    slli x2, x10, 31
    li x3, 0x80000000
    bne x2, x3, fail

    # Test 15: SRL (shift right logical) basic
    li x10, 0x12345678
    li x2, 4
    srl x3, x10, x2
    li x4, 0x01234567
    bne x3, x4, fail

    # Test 16: SRL with maximum shift
    li x10, 0x80000000
    li x2, 31
    srl x3, x10, x2
    li x4, 1
    bne x3, x4, fail

    # Test 17: SRLI (shift right logical immediate)
    li x10, 0x12345678
    srli x2, x10, 8
    li x3, 0x00123456
    bne x2, x3, fail

    # Test 18: SRA (shift right arithmetic) - preserves sign
    li x10, 0x80000000  # Negative number
    li x2, 1
    sra x3, x10, x2
    li x4, 0xC0000000  # Sign extended
    bne x3, x4, fail

    # Test 19: SRA with positive number
    li x10, 0x12345678
    li x2, 4
    sra x3, x10, x2
    li x4, 0x01234567
    bne x3, x4, fail

    # Test 20: SRAI (shift right arithmetic immediate)
    li x10, 0xF0000000  # Negative
    srai x2, x10, 4
    li x3, 0xFF000000  # Sign extended
    bne x2, x3, fail

    # Test 21: Edge case - shift by 0
    li x10, 0x12345678
    li x2, 0
    sll x3, x10, x2
    bne x3, x10, fail    # Should be unchanged

    # Test 22: Edge case - shift amount masked to 5 bits
    li x10, 0x12345678
    li x2, 32          # Should be treated as 0 (32 & 0x1F = 0)
    sll x3, x10, x2
    bne x3, x10, fail

    # Test 23: Bit manipulation patterns
    li x10, 0x00000001
    li x2, 0
loop_shift:
    sll x10, x10, x2     # Shift left by x2
    addi x2, x2, 1     # Increment shift amount
    li x3, 8
    blt x2, x3, loop_shift
    # After loop, x10 should have bit 7 set (if we started with shifts)

    # Test 24: Complex bit manipulation
    li x10, 0xDEADBEEF
    li x2, 0xCAFEBABE
    and x3, x10, x2     # AND
    or x4, x10, x2      # OR
    xor x5, x10, x2     # XOR
    # Verify De Morgan's law: ~(A & B) = ~A | ~B
    # This is just a complex operation test

    # Test 25: All immediate values coverage for ANDI/ORI/XORI
    # Test with 0
    li x10, 0xFFFFFFFF
    andi x2, x10, 0
    li x3, 0
    bne x2, x3, fail

    # Test with all bits in 12-bit immediate
    li x10, 0
    ori x2, x10, 0x7FF
    li x3, 0x7FF
    bne x2, x3, fail

    # Success - return 0
    li x17, 0   # a7 = 0 (no panic)
    li x10, 0   # a0 = 0 (success)
    ret

fail:
    # Failure - return -1
    li x17, 0   # a7 = 0 (no panic)
    li x10, -1  # a0 = -1 (failure)
    ret
