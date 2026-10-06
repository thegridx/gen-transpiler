# test fences and atomics with ordering bits, as compilers emit for sequentially consistent
# atomics. Transpiled code is single-threaded, so fences are no-ops and the aq/rl bits do not
# change what an atomic does.
# Returns 0 on success, or -N identifying the failing test, in a0
.section .text
.globl _test_start

_test_start:
    la a2, memory_loc

    # Test 1: fences of every common form fall through
    li t0, 5
    fence
    fence rw, rw
    fence r, r
    fence w, w
    fence iorw, iorw
    fence.tso
    addi t0, t0, 1
    li t1, 6
    li a4, -1
    bne t0, t1, fail

    # Test 2: amoadd.w.aqrl returns the old value and adds
    li t0, 40
    sw t0, 0(a2)
    li t1, 2
    amoadd.w.aqrl a0, t1, (a2)
    li a4, -2
    li t2, 40
    bne a0, t2, fail
    lw t3, 0(a2)
    li a4, -3
    li t2, 42
    bne t3, t2, fail

    # Test 3: amoswap.w.aq and amoswap.w.rl
    li t0, 7
    amoswap.w.aq a0, t0, (a2)
    li a4, -4
    li t2, 42
    bne a0, t2, fail
    li t0, 9
    amoswap.w.rl a0, t0, (a2)
    li a4, -5
    li t2, 7
    bne a0, t2, fail
    lw t3, 0(a2)
    li a4, -6
    li t2, 9
    bne t3, t2, fail

    # Test 4: lr.w.aq / sc.w.rl, the compare-and-swap loop compilers emit
    li t1, 100
retry:
    lr.w.aq t0, (a2)
    sc.w.rl t2, t1, (a2)
    bnez t2, retry
    lw t3, 0(a2)
    li a4, -7
    bne t3, t1, fail
    li a4, -8
    li t2, 9
    bne t0, t2, fail

    # Test 5: amomaxu.w.aqrl
    li t0, 200
    amomaxu.w.aqrl a0, t0, (a2)
    lw t3, 0(a2)
    li a4, -9
    bne t3, t0, fail

    # All tests passed
    li a7, 0    # no panic
    li a0, 0    # success
    ret

fail:
    li a7, 0    # no panic
    mv a0, a4   # the failing test's code
    ret

.section .data
    .align 4
memory_loc:
    .word 0
