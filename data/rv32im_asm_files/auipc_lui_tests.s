# Tests: LUI, AUIPC
# Returns: 0 on success, -1 on failure

.text
.globl _test_start

_test_start:
    # Test 1: LUI (load upper immediate) - basic operation
    lui x10, 0x12345
    li x2, 0x12345000
    bne x10, x2, fail

    # Test 2: LUI with maximum value (20 bits)
    lui x10, 0xFFFFF    # Maximum 20-bit value
    li x2, 0xFFFFF000
    bne x10, x2, fail

    # Test 3: LUI with zero
    lui x10, 0
    li x2, 0
    bne x10, x2, fail

    # Test 4: LUI with sign bit set (bit 19)
    lui x10, 0x80000    # Bit 19 set, should sign extend
    li x2, 0x80000000
    bne x10, x2, fail

    # Test 5: LUI followed by ADDI to create full 32-bit constant
    lui x10, 0x12345
    addi x10, x10, 0x678
    li x2, 0x12345678
    bne x10, x2, fail

    # Test 6: LUI with negative ADDI (handling sign extension)
    lui x10, 0x12345    # One more than target upper
    addi x10, x10, -0x523 # Negative immediate to compensate
    li x2, 0x12344ADD
    bne x10, x2, fail

    # Test 7: AUIPC (add upper immediate to PC) - basic
    auipc x10, 0         # Should give current PC
    # We can't easily test exact PC value, but we can test relative behavior
    auipc x2, 0         # Should be 4 bytes more than x10
    sub x3, x2, x10      # Difference should be 4
    li x4, 4
    bne x3, x4, fail

    # Test 8: AUIPC with non-zero immediate
    auipc x10, 1   # PC + 0x1000
    auipc x2, 0         # Current PC
    sub x3, x10, x2
    li x4, 0xFFC
    bne x3, x4, fail

    # Test 9: AUIPC with large immediate
    auipc x10, 0x12345
    auipc x2, 0
    addi x10, x10, 4
    sub x3, x10, x2      # Should be 0x12345000
    li x4, 0x12345000
    bne x3, x4, fail

    # Test 10: AUIPC with sign bit set
    auipc x10, 0x80000   # Sign bit set in immediate
    auipc x2, 0
    addi x10, x10, 4
    sub x3, x10, x2
    li x4, 0x80000000   # Should be sign extended
    bne x3, x4, fail

    # Test 11: Using AUIPC for position-independent code
    auipc x10, 0         # Get current PC
    addi x10, x10, 28    # Point to data_ref
    lw x2, 0(x10)        # Load the data
    li x3, 0xDEADBEEF    # this is two commands
    bne x2, x3, fail
    j skip_data

data_ref:
    .word 0xDEADBEEF

skip_data:
    # Test 12: LUI to x0 (should be ignored)
    lui x0, 0x12345     # Try to write to x0
    li x10, 0
    bne x0, x10, fail    # x0 should still be 0

    # Test 13: AUIPC to x0 (should be ignored)
    auipc x0, 0x12345   # Try to write to x0
    li x10, 0
    bne x0, x10, fail    # x0 should still be 0

    # Test 14: Building addresses with AUIPC + load
    auipc x10, 0         # Get PC
    addi x10, x10, 28    # 7 words: Point to test_value
    lw x2, 0(x10)        # Load the value
    li x3, 0xCAFEBABE    # this translates to two words = instructions
    bne x2, x3, fail
    j skip_test_value
test_value:
    .word 0xCAFEBABE
skip_test_value:

    # Test 15: Maximum and minimum immediate values
    lui x10, 0x7FFFF     # Maximum positive 20-bit value
    li x2, 0x7FFFF000
    bne x10, x2, fail

    lui x10, 0x80000     # Minimum value that sets sign bit
    li x2, 0x80000000
    bne x10, x2, fail

    # Test 16: AUIPC for computing relative addresses
    auipc x10, 0
    la x2, current_location
current_location:
    # x10 should be close to x2 (within a few instructions)
    sub x3, x2, x10
    li x4, 12      # la is translated to an auipc + add
    bne x3, x4, fail


    # Test 17: Combining LUI and AUIPC for large constants
    # Create a large address offset
    auipc x10, 0x10000   # Large PC offset
    lui x2, 0x20000     # Large constant
    add x3, x10, x2      # Combine them
    auipc x11, 0
    li x4, 0x2FFFFFF4
    add x4, x4, x11
    bne x4, x3, fail

    # Test 18: Edge case - immediate value 0x80000 (sign extension)
    lui x10, 0x80000
    # In 32-bit, this becomes 0x80000000 (negative)
    # Check if it's negative
    li x2, 0
    bge x10, x2, fail    # Should be negative

    # Test 19: Building function pointers with AUIPC
    auipc x10, 0
    addi x10, x10, 16     # Point to function
    jalr x2, x10, 0      # Call the function
    j func_call_pass

    # Simple function
    li x3, 99           # Set a value
    jalr x0, x2, 0      # Return

func_call_pass:
    li x4, 99
    bne x3, x4, fail    # Verify function executed

    # Test 20: All registers as destinations
    lui x21, 0x12345
    li x10, 0x12345000
    bne x21, x10, fail

    auipc x20, 1
    auipc x19, 0
    sub x28, x20, x19
    li x27, 0xFFC
    bne x28, x27, fail

    # Test 21: Immediate value patterns
    # Test all 1's in upper bits
    lui x10, 0xFFFFF
    li x2, 0xFFFFF000
    bne x10, x2, fail

    # Test alternating pattern
    lui x10, 0xAAAAA
    li x2, 0xAAAAA000
    bne x10, x2, fail

    lui x10, 0x55555
    li x2, 0x55555000
    bne x10, x2, fail

    # Test 22: Sequential AUIPC calls (PC advancement)
    auipc x10, 0         # First call
    auipc x2, 0         # Second call
    auipc x3, 0         # Third call

    # Verify PC advanced correctly
    sub x4, x2, x10      # Should be 4
    li x5, 4
    bne x4, x5, fail

    sub x4, x3, x2      # Should be 4
    bne x4, x5, fail


    # Test 23: AUIPC with immediate that causes overflow
    auipc x10, 0xFFFFF   # Maximum immediate
    auipc x11, 0
    li x12, 0xFFFFEFFC
    sub x10, x10, x11
    bne x12, x10, fail

    # Test 24: Using LUI to create masks
    lui x10, 0xFFFFF     # Create mask
    li x2, 0x12345678
    and x3, x2, x10      # Apply mask
    li x4, 0x12345000   # Expected result
    bne x3, x4, fail

    # Test 25: Position-independent data access
    auipc x10, 0         # Get current PC
    addi x10, x10, 28     # Point to data
    lw x2, 0(x10)        # Load data
    li x3, 0x11223344
    bne x2, x3, fail
    j skip_final_data
final_data:
    .word 0x11223344
skip_final_data:

    # Success - return 0
    li x17, 0   # a7 = 0 (no panic)
    li x10, 0   # a0 = 0 (success)
    ret

fail:
    # Failure - return -1
    li x17, 0   # a7 = 0 (no panic)
    li x10, -1  # a0 = -1 (failure)
    ret
