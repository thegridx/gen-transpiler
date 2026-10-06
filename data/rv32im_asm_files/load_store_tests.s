# Tests: LB, LH, LW, LBU, LHU, SB, SH, SW
# Returns: 0 on success, -1 on failure

.section .text
.globl _test_start

_test_start:
    # Test 1: LW (load word) - basic load
    la x10, test_data
    lw x2, 0(x10)
    li x3, 0x12345678
    bne x2, x3, fail

    # Test 2: LW with positive offset
    la x10, test_data
    lw x2, 4(x10)        # Load second word
    li x3, 0xDEADBEEF
    bne x2, x3, fail

    # Test 3: LW with maximum positive offset
    la x10, test_data
    lw x2, 12(x10)       # Load fourth word (offset 12)
    li x3, 0xFEEDFACE
    bne x2, x3, fail

    # Test 4: LW with negative offset
    la x10, test_data
    addi x10, x10, 8      # Point to third word
    lw x2, -4(x10)       # Load second word using negative offset
    li x3, 0xDEADBEEF
    bne x2, x3, fail

    # Test 5: LH (load halfword) - basic load
    la x10, test_halfwords
    lh x2, 0(x10)
    li x3, 0x1234
    bne x2, x3, fail

    # Test 6: LH with sign extension (negative value)
    la x10, test_halfwords
    lh x2, 4(x10)        # Load 0xABCD (negative as halfword)
    li x3, 0xFFFFABCD   # Sign extended
    bne x2, x3, fail

    # Test 7: LHU (load halfword unsigned) - no sign extension
    la x10, test_halfwords
    lhu x2, 4(x10)       # Load 0xABCD
    li x3, 0x0000ABCD   # Zero extended
    bne x2, x3, fail

    # Test 8: LB (load byte) - basic load
    la x10, test_bytes
    lb x2, 0(x10)
    li x3, 0x12
    bne x2, x3, fail

    # Test 9: LB with sign extension
    la x10, test_bytes
    lb x2, 4(x10)        # Load 0x9A (negative as byte)
    li x3, 0xFFFFFF9A   # Sign extended
    bne x2, x3, fail

    # Test 10: LBU (load byte unsigned) - no sign extension
    la x10, test_bytes
    lbu x2, 4(x10)       # Load 0x9A
    li x3, 0x0000009A   # Zero extended
    bne x2, x3, fail

    # Test 11: SW (store word) - basic store
    la x10, work_area
    li x2, 0x87654321
    sw x2, 0(x10)
    lw x3, 0(x10)        # Load back to verify
    bne x2, x3, fail

    # Test 12: SW with offset
    la x10, work_area
    li x2, 0xABCDEF00
    sw x2, 8(x10)
    lw x3, 8(x10)
    bne x2, x3, fail

    # Test 13: SH (store halfword)
    la x10, work_area
    li x2, 0x12345678
    sh x2, 16(x10)       # Store lower 16 bits
    lhu x3, 16(x10)      # Load back as unsigned
    li x4, 0x5678
    bne x3, x4, fail

    # Test 14: SB (store byte)
    la x10, work_area
    li x2, 0x123456AB
    sb x2, 20(x10)       # Store lower 8 bits
    lbu x3, 20(x10)      # Load back as unsigned
    li x4, 0xAB
    bne x3, x4, fail

    # Test 15: Unaligned access simulation (accessing bytes of a word)
    la x10, test_data
    lbu x2, 0(x10)       # First byte of 0x12345678
    li x3, 0x78         # Little endian: LSB first
    bne x2, x3, fail

    lbu x2, 1(x10)       # Second byte
    li x3, 0x56
    bne x2, x3, fail

    lbu x2, 2(x10)       # Third byte
    li x3, 0x34
    bne x2, x3, fail

    lbu x2, 3(x10)       # Fourth byte (MSB)
    li x3, 0x12
    bne x2, x3, fail

    # Test 16: Store then load different sizes (aliasing test)
    la x10, work_area
    li x2, 0xDEADBEEF
    sw x2, 24(x10)       # Store as word

    # Load as halfwords
    lhu x3, 24(x10)      # Lower halfword
    li x4, 0xBEEF
    bne x3, x4, fail

    lhu x3, 26(x10)      # Upper halfword
    li x4, 0xDEAD
    bne x3, x4, fail

    # Test 17: Maximum offset values
    la x10, work_area
    li x2, 0x55AA55AA
    sw x2, 2047(x10)     # Maximum positive offset for I-type
    # Note: This might cause issues if work_area is too small
    # In real test, ensure work_area is large enough

    # Test 18: Minimum offset values
    la x10, work_area
    addi x10, x10, 2047   # Move pointer forward
    sw x2, -2047(x10)    # Minimum negative offset

    # Test 19: Register addressing modes
    la x10, work_area
    li x2, 32           # Offset in register
    li x3, 0x11223344
    add x4, x10, x2      # Calculate address
    sw x3, 0(x4)        # Store at calculated address
    lw x5, 32(x10)       # Load using immediate offset
    bne x3, x5, fail

    # Test 20: Zero register as address (should cause fault in real system)
    # Skip this test as it would cause segfault

    # Test 21: All registers as base address
    la x10, work_area
    mv x5, x10           # Copy address to x5
    li x6, 0x99887766
    sw x6, 0(x5)        # Use x5 as base
    lw x7, 0(x5)
    bne x6, x7, fail

    # Test 22: Endianness verification
    la x10, work_area
    li x2, 0x12345678
    sw x2, 40(x10)

    # Check byte order
    lbu x3, 40(x10)      # Should be 0x78 (LSB)
    li x4, 0x78
    bne x3, x4, fail

    lbu x3, 43(x10)      # Should be 0x12 (MSB)
    li x4, 0x12
    bne x3, x4, fail

    # Test 23: Store/load sequence with dependencies
    la x10, work_area
    li x2, 0xAAAAAAAA
    sw x2, 44(x10)       # Store
    lw x3, 44(x10)       # Load immediately after
    bne x2, x3, fail

    # Test 24: Multiple stores to same location
    la x10, work_area
    li x2, 0x11111111
    li x3, 0x22222222
    li x4, 0x33333333
    sw x2, 48(x10)
    sw x3, 48(x10)       # Overwrite
    sw x4, 48(x10)       # Overwrite again
    lw x5, 48(x10)
    bne x4, x5, fail    # Should have last stored value

    # Test 25: Sign extension edge cases
    la x10, work_area

    # Store 0x80 (negative byte)
    li x2, 0x80
    sb x2, 52(x10)
    lb x3, 52(x10)       # Load with sign extension
    li x4, 0xFFFFFF80
    bne x3, x4, fail

    # Store 0x8000 (negative halfword)
    li x2, 0x8000
    sh x2, 54(x10)
    lh x3, 54(x10)       # Load with sign extension
    li x4, 0xFFFF8000
    bne x3, x4, fail

    # Success - return 0
    li x17, 0   # a7 = 0 (no panic)
    li x10, 0   # a0 = 0 (success)
    ret

fail:
    # Failure - return -1
    li x17, 0   # a7 = 0 (no panic)
    li x10, -1  # a0 = -1 (failure)
    ret

.section .data
# Test data section
.align 4
test_data:
    .word 0x12345678
    .word 0xDEADBEEF
    .word 0xCAFEBABE
    .word 0xFEEDFACE

.align 2
test_halfwords:
    .half 0x1234
    .half 0x5678
    .half 0xABCD
    .half 0xEF00

.align 1
test_bytes:
    .byte 0x12
    .byte 0x34
    .byte 0x56
    .byte 0x78
    .byte 0x9A
    .byte 0xBC
    .byte 0xDE
    .byte 0xF0

# Work area for store tests
.align 4
work_area:
    .space 64
# We have to have this much memory for test #17
.section .bss
    .space 2048
