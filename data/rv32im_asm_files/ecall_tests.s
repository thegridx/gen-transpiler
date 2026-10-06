# RISC-V 32I ecall test: ecall 1 = addition and ecall 2 = multiplication

.text
.global _test_start

_test_start:
    li   t0, 10              # Loop count for outer loop
outer:
    li   t1, 10              # Loop count for inner loop
inner:
    mv   a0, t0
    mv   a1, t1
    add  t2, a0, a1
    li   a7, 1
    ecall
    bne  a0, t2, test_failed
    mv   a0, t0
    mv   a1, t1
    mul  t2, a0, a1
    li   a7, 2
    ecall
    bne  a0, t2, test_failed
    addi t1, t1, -1
    bnez t1, inner
    addi t0, t0, -1
    bnez t0, outer
    # Success
    li   a7, 0   # a7 = 0 (no panic)
    li   a0, 0   # a0 = 0 (success)
    j    exit
test_failed:
    li   a7, 0   # a7 = 0 (no panic)
    li   a0, -1  # a0 = -1 (failure)
exit:
    ret
