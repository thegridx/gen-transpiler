# test ANDI: the range is 12 bits
.text
.globl _test_start
_test_start:
	li a0, 0x3
	andi a1, a0, 0x6
	li a0, 0x2
	bne a0, a1, .bad

	li a2, 0x6
	andi a3, a2, 0xc
	li a2, 0x4
	bne a2, a3, .bad

	li a4, 0xc
	andi a5, a4, 0x18
	li a4, 0x8
	bne a4, a5, .bad

	li a6, 0x18
	andi a7, a6, 0x30
	li a6, 0x10
	bne a6, a7, .bad

	li a0, 0x30
	andi a1, a0, 0x60
	li a0, 0x20
	bne a0, a1, .bad

	li a2, 0x60
	andi a3, a2, 0xc0
	li a2, 0x40
	bne a2, a3, .bad

	li a4, 0xc0
	andi a5, a4, 0x180
	li a4, 0x80
	bne a4, a5, .bad

	li a6, 0x180
	andi a7, a6, 0x300
	li a6, 0x100
	bne a6, a7, .bad

	li a0, 0x300
	andi a1, a0, 0x600
	li a0, 0x200
	bne a0, a1, .bad

	li a2, 0x600
	andi a3, a2, -1024
	li a2, 0x400
	bne a2, a3, .bad

	li a7, 0  /* a7 = 0 (no panic) */
	li a0, 0  /* a0 = 0 (success) */
	ret
.bad:
	li a7, 0  /* a7 = 0 (no panic) */
	li a0, -1  /* a0 = -1 (failure) */
	ret
