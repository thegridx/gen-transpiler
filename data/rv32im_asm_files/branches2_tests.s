.text
.globl _test_start

_test_start:
        li x5, -2
        li x6, -2
        li x7, -1
        li x11, -1
        li x12, 0
        li x13, 0
        li x14, 1
        li x15, 1
        li x16, 2
        li x17, 2
        beq x5, x7, fail_eq_ne_test
        beq x5, x12, fail_eq_ne_test
        beq x5, x14, fail_eq_ne_test
        beq x5, x16, fail_eq_ne_test
        beq x7, x12, fail_eq_ne_test
        beq x7, x14, fail_eq_ne_test
        beq x7, x16, fail_eq_ne_test
        beq x12, x14, fail_eq_ne_test
        beq x14, x16, fail_eq_ne_test
        beq x5, x6, eq_m2
        j fail_eq_ne_test
eq_m2:
        beq x7, x11, eq_m1
        j fail_eq_ne_test
eq_m1:
        beq x12, x13, eq_0
        j fail_eq_ne_test
eq_0:
        beq x14, x15, eq_1
        j fail_eq_ne_test
eq_1:
        beq x16, x17, eq_2
        j fail_eq_ne_test
eq_2:
        bne x5, x6, fail_eq_ne_test
        bne x7, x11, fail_eq_ne_test
        bne x12, x13, fail_eq_ne_test
        bne x14, x15, fail_eq_ne_test
        bne x16, x17, fail_eq_ne_test
        bne x5, x7, ne_m2_m1
        j fail_eq_ne_test
ne_m2_m1:
        bne x7, x12, ne_m1_0
        j fail_eq_ne_test
ne_m1_0:
        bne x12, x14, ne_0_1
        j fail_eq_ne_test
ne_0_1:
        bne x14, x16, blt_bge_test
fail_eq_ne_test:
        li x17, 0   # a7 = 0 (no panic)
        li x10, -1
        ret
blt_bge_test:
        bge x5, x7, fail_lt_ge_test        # !(-2 >= -1)
        bge x5, x12, fail_lt_ge_test       # !(-2 >= 0)
        bge x5, x14, fail_lt_ge_test       # !(-2 >= 1)
        bge x5, x16, fail_lt_ge_test       # !(-2 >= 2)
        bge x7, x12, fail_lt_ge_test       # !(-1 >= 0)
        bge x7, x14, fail_lt_ge_test       # !(-1 >= 1)
        bge x7, x16, fail_lt_ge_test       # !(-1 >= 2)
        bge x12, x14, fail_lt_ge_test      # !(0 >= 1)
        bge x14, x16, fail_lt_ge_test      # !(1 >= 2)

        bge x5, x6, ge_m2                  # -2 >= -2
        j fail_lt_ge_test

ge_m2:
        bge x7, x11, ge_m1                 # -1 >= -1
        j fail_lt_ge_test
ge_m1:
        bge x12, x13, ge_0                 # 0 >= 0
        j fail_lt_ge_test
ge_0:
        bge x14, x15, ge_1                 # 1 >= 1
        j fail_lt_ge_test
ge_1:
        bge x16, x17, ge_2                 # 2 >= 2
        j fail_lt_ge_test
ge_2:
        blt x5, x6, fail_lt_ge_test       # !(-2 < -2)
        blt x7, x11, fail_lt_ge_test      # !(-1 < -1)
        blt x12, x13, fail_lt_ge_test     # !(0 < 0)
        blt x14, x15, fail_lt_ge_test     # !(1 < 1)
        blt x16, x17, fail_lt_ge_test     # !(2 < 2)
        blt x5, x7, lt_m2_m1              # -2 < -1
        j fail_lt_ge_test

lt_m2_m1:
        blt x7, x12, lt_m1_0              # -1 < 0
        j fail_lt_ge_test
lt_m1_0:
        blt x12, x14, lt_0_1              # 0 < 1
        j fail_lt_ge_test
lt_0_1:
        blt x14, x16, bltu_bgeu_test      # 1 < 2
fail_lt_ge_test:
        li x17, 0   # a7 = 0 (no panic)
        li x10, -2
        ret
bltu_bgeu_test:
        bgeu x5, x7, fail_ltu_geu_test    # !(-2u >= -1u)
        bltu x5, x12, fail_ltu_geu_test   # !(-2u < 0u)
        bltu x5, x14, fail_ltu_geu_test   # !(-2u < 1u)
        bltu x5, x16, fail_ltu_geu_test   # !(-2u < 2u)
        bltu x7, x12, fail_ltu_geu_test   # !(-1u < 0u)
        bltu x7, x14, fail_ltu_geu_test   # !(-1u < 1u)
        bltu x7, x16, fail_ltu_geu_test   # !(-1u < 2u)
        bgeu x12, x14, fail_ltu_geu_test  # !(0u >= 1u)
        bgeu x14, x16, fail_ltu_geu_test  # !(1u >= 2u)
        bgeu x5, x6, geu_m2               # -2u >= -2u
        j fail_ltu_geu_test
geu_m2:
        bgeu x7, x11, geu_m1              # -1u >= -1u
        j fail_ltu_geu_test
geu_m1:
        bgeu x12, x13, geu_0              # 0u >= 0u
        j fail_ltu_geu_test
geu_0:
        bgeu x14, x15, geu_1              # 1u >= 1u
        j fail_ltu_geu_test
geu_1:
        bgeu x16, x17, geu_2              # 2u >= 2u
        j fail_ltu_geu_test
geu_2:
        bltu x5, x6, fail_ltu_geu_test    # !(-2u < -2u)
        bltu x7, x11, fail_ltu_geu_test   # !(-1u < -1u)
        bltu x12, x13, fail_ltu_geu_test  # !(0u < 0u)
        bltu x14, x15, fail_ltu_geu_test  # !(1u < 1u)
        bltu x16, x17, fail_ltu_geu_test  # !(2u < 2u)
        bltu x5, x7, ltu_m2_m1            # -2u < -1u
        j fail_ltu_geu_test
ltu_m2_m1:
        bgeu x5, x12, geu_m2_0            # -2u >= 0u
        j fail_ltu_geu_test
geu_m2_0:
        bgeu x7, x12, geu_m1_0            # -1u >= 0u
        j fail_ltu_geu_test
geu_m1_0:
        bltu x12, x14, ltu_0_1            # 0u < 1u
        j fail_ltu_geu_test
ltu_0_1:
        bltu x14, x16, done               # 1u < 2u
fail_ltu_geu_test:
        li x17, 0   # a7 = 0 (no panic)
        li x10, -3  # a0 = -3 (failure)
        ret
done:
        li x17, 0   # a7 = 0 (no panic)
        li x10, 0   # a0 = 0 (success)
        ret
