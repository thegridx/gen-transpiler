# tests ADDI: the range is -(1<<11)...(1<<11) i.e. with the first one and without the last
.text
.globl _test_start
_test_start:
	li a0, 1
	addi a1, a0, -2
	addi a2, a1, 4
	addi a3, a2, -8
	addi a4, a3, 16
	addi a5, a4, -32
	addi a6, a5, 64
	addi a7, a6, -128
	addi a0, a7, 256
	addi a1, a0, -512
	addi a2, a1, 1024
	addi a3, a2, -2048
/* sum_{n=0}^5 (1<<2n)-(2<<2n)=-sum_{n=0}^5 1<<2n=-(1+4+16+64+256+1024)=-(5+80+1280)*/
	li a4, -1365
	li a7, 0  /* a7 = 0 (no panic) */
	li a0, 0  /* a0 = 0 (success) */
	beq a3, a4, .get_out
	li a7, 0  /* a7 = 0 (no panic) */
	li a0, -1  /* a0 = -1 (failure) */
.get_out:
	ret
