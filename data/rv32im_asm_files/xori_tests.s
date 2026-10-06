# test XORI : the range is 12 bits
.text
.globl _test_start
_test_start:
	li a0, 0x3
	xori a1, a0, 0x6
	li a0, 0x5
	bne a0, a1, .bad

	li a2, 0x6
	xori a3, a2, 0xc
	li a2, 0xa
	bne a2, a3, .bad

	li a4, 0xc
	xori a5, a4, 0x18
	li a4, 0x14
	bne a4, a5, .bad

	li a6, 0x18
	xori a7, a6, 0x30
	li a6, 0x28
	bne a6, a7, .bad

	li a0, 0x30
	xori a1, a0, 0x60
	li a0, 0x50
	bne a0, a1, .bad

	li a2, 0x60
	xori a3, a2, 0xc0
	li a2, 0xa0
	bne a2, a3, .bad

	li a4, 0xc0
	xori a5, a4, 0x180
	li a4, 0x140
	bne a4, a5, .bad

	li a6, 0x180
	xori a7, a6, 0x300
	li a6, 0x280
	bne a6, a7, .bad

	li a0, 0x300
	xori a1, a0, 0x600
	li a0, 0x500
	bne a0, a1, .bad

	li a2, 0x600
	xori a3, a2, -1024
	li a2, -1536
	bne a2, a3, .bad

	li a7, 0  /* a7 = 0 (no panic) */
	li a0, 0  /* a0 = 0 (success) */
	ret
.bad:
	li a7, 0  /* a7 = 0 (no panic) */
	li a0, -1  /* a0 = -1 (failure) */
	ret
