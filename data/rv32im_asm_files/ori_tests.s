# test ORI: the range is 12 bits
.text
.globl _test_start
_test_start:
	li a0, 0x1
	ori a1, a0, 0x2
	li a0, 0x3
	bne a0, a1, .bad
	li a2, 0x4
	ori a3, a2, 0x8
	li a2, 0xc
	bne a2, a3, .bad
	li a4, 0x10
	ori a5, a4, 0x20
	li a4, 0x30
	bne a4, a5, .bad
	li a6, 0x40
	ori a7, a6, 0x80
	li a6, 0xc0
	bne a6, a7, .bad
	li a0, 0x100
	ori a1, a0, 0x200
	li a0, 0x300
	bne a0, a1, .bad
	li a2, 0x400
	ori a3, a2, -2048
	li a2, -1024
	bne a2, a3, .bad
	li a7, 0  /* a7 = 0 (no panic) */
	li a0, 0  /* a0 = 0 (success) */
	ret
.bad:
	li a7, 0  /* a7 = 0 (no panic) */
	li a0, -1  /* a0 = -1 (failure) */
	ret
