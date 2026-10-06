.section .text
.globl _test_start

_test_start:
    li a7, 0                     # return code    

# ============================================================================
# AMOSWAP.W Test Suite - 22 cases (tests 1**)
# ============================================================================

# --- GROUP A: 5 cases ---

# A1: (a0, a1, a2)
test_swap_a1:
    li a1, 0x8F8F8F8F
    la a2, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a2)
    mv t1, a1
    amoswap.w a0, a1, (a2)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_a1_failed
    bne a1, t1, test_swap_a1_failed
    lw t3, 0(a2)
    bne t3, t1, test_swap_a1_failed

# A2: (a1, a0, a0)
test_swap_a2:
    li a0, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoswap.w a1, a0, (a0)
    li t2, 0x0F0F0F0F
    bne a1, t2, test_swap_a2_failed
    lw t3, 0(t1)
    bne t3, t1, test_swap_a2_failed

# A3: (a0, a1, a0)
test_swap_a3:
    li a1, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoswap.w a0, a1, (a0)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_a3_failed
    lw t3, 0(t1)
    bne t3, a1, test_swap_a3_failed

# A4: (a0, a0, a1)
test_swap_a4:
    li a0, 0x8F8F8F8F
    la a1, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a1)
    mv t1, a0
    amoswap.w a0, a0, (a1)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_a4_failed
    lw t3, 0(a1)
    bne t3, t1, test_swap_a4_failed

# A5: (a0, a0, a0)
test_swap_a5:
    li a0, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoswap.w a0, a0, (a0)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_a5_failed
    lw t3, 0(t1)
    bne t3, t1, test_swap_a5_failed

# --- GROUP B: 10 cases ---

# B1: (t3, a0, a1)
test_swap_b1:
    li a0, 0x8F8F8F8F
    la a1, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a1)
    mv t1, a0
    amoswap.w t3, a0, (a1)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_b1_failed
    bne a0, t1, test_swap_b1_failed
    lw t4, 0(a1)
    bne t4, t1, test_swap_b1_failed

# B2: (t3, a0, a0)
test_swap_b2:
    li a0, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoswap.w t3, a0, (a0)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_b2_failed
    lw t4, 0(t1)
    bne t4, t1, test_swap_b2_failed

# B3: (a0, t3, a1)
test_swap_b3:
    li t3, 0x8F8F8F8F
    la a1, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a1)
    mv t1, t3
    amoswap.w a0, t3, (a1)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_b3_failed
    bne t3, t1, test_swap_b3_failed
    lw t4, 0(a1)
    bne t4, t1, test_swap_b3_failed

# B4: (a0, t3, a0)
test_swap_b4:
    li t3, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoswap.w a0, t3, (a0)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_b4_failed
    lw t4, 0(t1)
    bne t4, t3, test_swap_b4_failed

# B5: (a0, a1, t3)
test_swap_b5:
    li a1, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, a1
    amoswap.w a0, a1, (t3)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_b5_failed
    bne a1, t1, test_swap_b5_failed
    lw t4, 0(t3)
    bne t4, t1, test_swap_b5_failed

# B6: (a0, a0, t3)
test_swap_b6:
    li a0, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, a0
    amoswap.w a0, a0, (t3)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_b6_failed
    lw t4, 0(t3)
    bne t4, t1, test_swap_b6_failed

# B7: (a0, t3, t3)
test_swap_b7:
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoswap.w a0, t3, (t3)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_b7_failed
    lw t4, 0(t1)
    bne t4, t1, test_swap_b7_failed

# B8: (t3, a0, t3)
test_swap_b8:
    li a0, 0x8F8F8F8F
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoswap.w t3, a0, (t3)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_b8_failed
    lw t4, 0(t1)
    bne t4, a0, test_swap_b8_failed

# B9: (t3, t3, a0)
test_swap_b9:
    li t3, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, t3
    amoswap.w t3, t3, (a0)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_b9_failed
    lw t4, 0(a0)
    bne t4, t1, test_swap_b9_failed

# B10: (t3, t3, t3)
test_swap_b10:
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoswap.w t3, t3, (t3)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_b10_failed
    lw t4, 0(t1)
    bne t4, t1, test_swap_b10_failed

# --- GROUP C: 6 cases ---

# C1: (t3, t4, a0)
test_swap_c1:
    li t4, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, t4
    amoswap.w t3, t4, (a0)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_c1_failed
    bne t4, t1, test_swap_c1_failed
    lw t5, 0(a0)
    bne t5, t1, test_swap_c1_failed

# C2: (t3, a0, t4)
test_swap_c2:
    li a0, 0x8F8F8F8F
    la t4, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t4)
    mv t1, a0
    amoswap.w t3, a0, (t4)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_c2_failed
    bne a0, t1, test_swap_c2_failed
    lw t5, 0(t4)
    bne t5, t1, test_swap_c2_failed

# C3: (a0, t3, t4)
test_swap_c3:
    li t3, 0x8F8F8F8F
    la t4, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t4)
    mv t1, t3
    amoswap.w a0, t3, (t4)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_swap_c3_failed
    bne t3, t1, test_swap_c3_failed
    lw t5, 0(t4)
    bne t5, t1, test_swap_c3_failed

# C4: (t3, t3, t4)
test_swap_c4:
    li t3, 0x8F8F8F8F
    la t4, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t4)
    mv t1, t3
    amoswap.w t3, t3, (t4)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_c4_failed
    lw t5, 0(t4)
    bne t5, t1, test_swap_c4_failed

# C5: (t3, t4, t3)
test_swap_c5:
    li t4, 0x8F8F8F8F
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoswap.w t3, t4, (t3)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_c5_failed
    lw t5, 0(t1)
    bne t5, t4, test_swap_c5_failed

# C6: (t4, t3, t3)
test_swap_c6:
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoswap.w t4, t3, (t3)
    li t2, 0x0F0F0F0F
    bne t4, t2, test_swap_c6_failed
    lw t5, 0(t1)
    bne t5, t1, test_swap_c6_failed

# --- GROUP D: 1 case ---

# D1: (t3, t4, t5)
test_swap_d1:
    li t4, 0x8F8F8F8F
    la t5, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t5)
    mv t1, t4
    amoswap.w t3, t4, (t5)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_swap_d1_failed
    bne t4, t1, test_swap_d1_failed
    lw t0, 0(t5)
    bne t0, t1, test_swap_d1_failed

# ============================================================================
# AMOADD.W Test Suite - 22 cases (tests 2**)
# ============================================================================

# --- GROUP A: 5 cases ---

# A1: (a0, a1, a2)
test_add_a1:
    li a1, 0x8F8F8F8F
    la a2, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a2)
    mv t1, a1
    amoadd.w a0, a1, (a2)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_a1_failed
    bne a1, t1, test_add_a1_failed
    add t3, t2, t1
    lw t4, 0(a2)
    bne t4, t3, test_add_a1_failed

# A2: (a1, a0, a0)
test_add_a2:
    li a0, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoadd.w a1, a0, (a0)
    li t2, 0x0F0F0F0F
    bne a1, t2, test_add_a2_failed
    add t3, t2, t1
    lw t4, 0(t1)
    bne t4, t3, test_add_a2_failed

# A3: (a0, a1, a0)
test_add_a3:
    li a1, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoadd.w a0, a1, (a0)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_a3_failed
    add t3, t2, a1
    lw t4, 0(t1)
    bne t4, t3, test_add_a3_failed

# A4: (a0, a0, a1)
test_add_a4:
    li a0, 0x8F8F8F8F
    la a1, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a1)
    mv t1, a0
    amoadd.w a0, a0, (a1)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_a4_failed
    add t3, t2, t1
    lw t4, 0(a1)
    bne t4, t3, test_add_a4_failed

# A5: (a0, a0, a0)
test_add_a5:
    li a0, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoadd.w a0, a0, (a0)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_a5_failed
    add t3, t2, t1
    lw t4, 0(t1)
    bne t4, t3, test_add_a5_failed

# --- GROUP B: 10 cases ---

# B1: (t3, a0, a1)
test_add_b1:
    li a0, 0x8F8F8F8F
    la a1, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a1)
    mv t1, a0
    amoadd.w t3, a0, (a1)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_b1_failed
    bne a0, t1, test_add_b1_failed
    add t4, t2, t1
    lw t5, 0(a1)
    bne t5, t4, test_add_b1_failed

# B2: (t3, a0, a0)
test_add_b2:
    li a0, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoadd.w t3, a0, (a0)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_b2_failed
    add t4, t2, t1
    lw t5, 0(t1)
    bne t5, t4, test_add_b2_failed

# B3: (a0, t3, a1)
test_add_b3:
    li t3, 0x8F8F8F8F
    la a1, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a1)
    mv t1, t3
    amoadd.w a0, t3, (a1)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_b3_failed
    bne t3, t1, test_add_b3_failed
    add t4, t2, t1
    lw t5, 0(a1)
    bne t5, t4, test_add_b3_failed

# B4: (a0, t3, a0)
test_add_b4:
    li t3, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, a0
    amoadd.w a0, t3, (a0)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_b4_failed
    add t4, t2, t3
    lw t5, 0(t1)
    bne t5, t4, test_add_b4_failed

# B5: (a0, a1, t3)
test_add_b5:
    li a1, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, a1
    amoadd.w a0, a1, (t3)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_b5_failed
    bne a1, t1, test_add_b5_failed
    add t4, t2, t1
    lw t5, 0(t3)
    bne t5, t4, test_add_b5_failed

# B6: (a0, a0, t3)
test_add_b6:
    li a0, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, a0
    amoadd.w a0, a0, (t3)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_b6_failed
    add t4, t2, t1
    lw t5, 0(t3)
    bne t5, t4, test_add_b6_failed

# B7: (a0, t3, t3)
test_add_b7:
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoadd.w a0, t3, (t3)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_b7_failed
    add t4, t2, t1
    lw t5, 0(t1)
    bne t5, t4, test_add_b7_failed

# B8: (t3, a0, t3)
test_add_b8:
    li a0, 0x8F8F8F8F
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoadd.w t3, a0, (t3)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_b8_failed
    add t4, t2, a0
    lw t5, 0(t1)
    bne t5, t4, test_add_b8_failed

# B9: (t3, t3, a0)
test_add_b9:
    li t3, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, t3
    amoadd.w t3, t3, (a0)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_b9_failed
    add t4, t2, t1
    lw t5, 0(a0)
    bne t5, t4, test_add_b9_failed

# B10: (t3, t3, t3)
test_add_b10:
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoadd.w t3, t3, (t3)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_b10_failed
    add t4, t2, t1
    lw t5, 0(t1)
    bne t5, t4, test_add_b10_failed

# --- GROUP C: 6 cases ---

# C1: (t3, t4, a0)
test_add_c1:
    li t4, 0x8F8F8F8F
    la a0, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(a0)
    mv t1, t4
    amoadd.w t3, t4, (a0)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_c1_failed
    bne t4, t1, test_add_c1_failed
    add t5, t2, t1
    lw t6, 0(a0)
    bne t6, t5, test_add_c1_failed

# C2: (t3, a0, t4)
test_add_c2:
    li a0, 0x8F8F8F8F
    la t4, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t4)
    mv t1, a0
    amoadd.w t3, a0, (t4)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_c2_failed
    bne a0, t1, test_add_c2_failed
    add t5, t2, t1
    lw t6, 0(t4)
    bne t6, t5, test_add_c2_failed

# C3: (a0, t3, t4)
test_add_c3:
    li t3, 0x8F8F8F8F
    la t4, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t4)
    mv t1, t3
    amoadd.w a0, t3, (t4)
    li t2, 0x0F0F0F0F
    bne a0, t2, test_add_c3_failed
    bne t3, t1, test_add_c3_failed
    add t5, t2, t1
    lw t6, 0(t4)
    bne t6, t5, test_add_c3_failed

# C4: (t3, t3, t4)
test_add_c4:
    li t3, 0x8F8F8F8F
    la t4, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t4)
    mv t1, t3
    amoadd.w t3, t3, (t4)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_c4_failed
    add t5, t2, t1
    lw t6, 0(t4)
    bne t6, t5, test_add_c4_failed

# C5: (t3, t4, t3)
test_add_c5:
    li t4, 0x8F8F8F8F
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoadd.w t3, t4, (t3)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_c5_failed
    add t5, t2, t4
    lw t6, 0(t1)
    bne t6, t5, test_add_c5_failed

# C6: (t4, t3, t3)
test_add_c6:
    li t3, 0x8F8F8F8F
    la t3, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t3)
    mv t1, t3
    amoadd.w t4, t3, (t3)
    li t2, 0x0F0F0F0F
    bne t4, t2, test_add_c6_failed
    add t5, t2, t1
    lw t6, 0(t1)
    bne t6, t5, test_add_c6_failed

# --- GROUP D: 1 case ---

# D1: (t3, t4, t5)
test_add_d1:
    li t4, 0x8F8F8F8F
    la t5, memory_loc
    li t0, 0x0F0F0F0F
    sw t0, 0(t5)
    mv t1, t4
    amoadd.w t3, t4, (t5)
    li t2, 0x0F0F0F0F
    bne t3, t2, test_add_d1_failed
    bne t4, t1, test_add_d1_failed
    add t0, t2, t1
    lw t6, 0(t5)
    bne t6, t0, test_add_d1_failed

### go over the other amo operations, but only with one combination each
        
    la a0, memory_loc            # Setup: Load base addresses

    li t1, 0x0F0F0F0F
    li t2, 0x8F8F8F8F

    # Test 1001: AMOXOR.W
    sw t2, 0(a0)                 # Mem[a0] = t2 
    amoxor.w a4, t1, (a0)        # a4 = old Mem[a0], Mem[a0] = t1 ^ t2
    bne a4, t2, test_xor_failed  # Verify: a4 == t2
    xor t4, t2, t1
    lw t3, 0(a0)
    bne t3, t4, test_xor_failed  # Verify: Mem[a0] == t1 ^ t2

    # Test 1002: AMOAND.W
    sw t2, 0(a0)                 # Mem[a0] = t2
    amoand.w a5, t1, (a0)        # a5 = old Mem[a0], Mem[a0] = t1 & t2
    bne a5, t2, test_and_failed  # Verify: a5 == t2
    and t4, t2, t1
    lw t3, 0(a0)
    bne t3, t4, test_and_failed  # Verify: Mem[a0] == t1 & t2
                
    # Test 1003: AMOOR.W
    sw t2, 0(a0)                 # Mem[a0] = t2
    amoor.w a6, t1, (a0)         # a6 = old Mem[a0], Mem[a0] = t1 | t2
    bne a6, t2, test_or_failed   # Verify: a6 == t2
    or t4, t2, t1
    lw t3, 0(a0)
    bne t3, t4, test_or_failed   # Verify: Mem[a0] == t1 | t2

    # Test 1004: AMOMIN.W
    sw t2, 0(a0)                 # Mem[a0] = t2
    amomin.w t5, t1, (a0)        # t5, Mem[a0] = min(t1, t2)
    bne t5, t2, test_min_failed  # Verify: t5 = t2
    lw t3, 0(a0)
    bne t3, t2, test_min_failed  # Verify: Mem[a0] == min(t1, t2) = t2

    # Test 1005: AMOMAX.W
    sw t2, 0(a0)                 # Mem[a0] = t2
    amomax.w t6, t1, (a0)        # t6 = old Mem[a0], Mem[a0] = max(t1, t2)
    bne t6, t2, test_max_failed  # Verify: t6 == t2
    lw t3, 0(a0)
    bne t3, t1, test_max_failed  # Verify: Mem[a0] == max(t1, t2) == t1

    # Test 1006: AMOMINU.W
    sw t2, 0(a0)                 # Mem[a0] = t2
    amominu.w a1, t1, (a0)       # a1 = old Mem[a0], Mem[a0] = minu(t1, t2)
    bne a1, t2, test_minu_failed # Verify: a1 == t2
    lw t3, 0(a0)
    bne t3, t1, test_minu_failed # Verify: Mem[a0] == minu(t1, t2) == t1

    # Test 1007: AMOMAXU.W
    sw t2, 0(a0)                 # Mem[a0] = t2
    amomaxu.w a2, t1, (a0)       # a2 = old Mem[a0], Mem[a0] = maxu(t1, t2)
    bne a2, t2, test_maxu_failed # Verify: a2 == t2
    lw t3, 0(a0)
    bne t3, t2, test_maxu_failed # Verify: Mem[a0] == maxu(t1, t2) == t2

        
        
###  Test 10000: LR.W / SC.W pair
    li t0, 500
    sw t0, 0(a0)                 # Mem[a0] = 500
    lr.w t1, (a0)                # t1 = Mem[a0]
    bne t1, t0, test_lrsc_failed # Verify Mem[a0] = 500
    addi t3, t1, 100             # t3 = t1 + 100
    sc.w t4, t3, (a0)            # Supposedly, Mem[a0] = t3, t4 = 0 (for success)
    bnez t4, test_lrsc_failed    # Verify t4 = 0
    lw t5, 0(a0)
    bne t5, t3, test_lrsc_failed # Verify: Mem[a0] == t3



    li a0, 0                     # SUCCESS !
    ret

#######swap test cases #################
test_swap_a1_failed:
    li a0, -101
    ret
test_swap_a2_failed:
    li a0, -102
    ret
test_swap_a3_failed:
    li a0, -103
    ret
test_swap_a4_failed:
    li a0, -104
    ret
test_swap_a5_failed:
    li a0, -105
    ret
test_swap_b1_failed:
    li a0, -111
    ret
test_swap_b2_failed:
    li a0, -112
    ret
test_swap_b3_failed:
    li a0, -113
    ret
test_swap_b4_failed:
    li a0, -114
    ret
test_swap_b5_failed:
    li a0, -115
    ret
test_swap_b6_failed:
    li a0, -116
    ret
test_swap_b7_failed:
    li a0, -117
    ret
test_swap_b8_failed:
    li a0, -118
    ret
test_swap_b9_failed:
    li a0, -119
    ret
test_swap_b10_failed:
    li a0, -120
    ret
test_swap_c1_failed:
    li a0, -121
    ret
test_swap_c2_failed:
    li a0, -122
    ret
test_swap_c3_failed:
    li a0, -123
    ret
test_swap_c4_failed:
    li a0, -124
    ret
test_swap_c5_failed:
    li a0, -125
    ret
test_swap_c6_failed:
    li a0, -126
    ret
test_swap_d1_failed:
    li a0, -131
    ret
######### add test cases ##############
test_add_a1_failed:
    li a0, -201
    ret
test_add_a2_failed:
    li a0, -202
    ret
test_add_a3_failed:
    li a0, -203
    ret
test_add_a4_failed:
    li a0, -204
    ret
test_add_a5_failed:
    li a0, -205
    ret
test_add_b1_failed:
    li a0, -211
    ret
test_add_b2_failed:
    li a0, -212
    ret
test_add_b3_failed:
    li a0, -213
    ret
test_add_b4_failed:
    li a0, -214
    ret
test_add_b5_failed:
    li a0, -215
    ret
test_add_b6_failed:
    li a0, -216
    ret
test_add_b7_failed:
    li a0, -217
    ret
test_add_b8_failed:
    li a0, -218
    ret
test_add_b9_failed:
    li a0, -219
    ret
test_add_b10_failed:
    li a0, -220
    ret
test_add_c1_failed:
    li a0, -221
    ret
test_add_c2_failed:
    li a0, -222
    ret
test_add_c3_failed:
    li a0, -223
    ret
test_add_c4_failed:
    li a0, -224
    ret
test_add_c5_failed:
    li a0, -225
    ret
test_add_c6_failed:
    li a0, -226
    ret
test_add_d1_failed:
    li a0, -231
    ret
        
####################        
test_xor_failed:
    li a0, -1001
    ret
test_and_failed:
    li a0, -1002
    ret
test_or_failed:
    li a0, -1003
    ret
test_min_failed:
    li a0, -1004
    ret
test_max_failed:
    li a0, -1005
    ret
test_minu_failed:
    li a0, -1006
    ret
test_maxu_failed:
    li a0, -1007
    ret

######################
test_lrsc_failed:
    li a0, -10000
    ret

.section .data
    .align 4
memory_loc:
    .word 100

        
