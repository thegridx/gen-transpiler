.text
.globl _test_start

# This test makes sure both the user-code and the loader use the same "register store" scheme - 
# 23 preserved RISC-V in order (starting with RiscvRegister::Ra and RiscvRegister::Sp) followed by the 
# "virtual return address" (for ecalls), and then the 64bit gas (not tested here).
# See RvPreservedRegister in crates/riscv32/src/registers.rs.
# The test ensures the order is platform-agnostic - first it assigns the correct index to each of the
# non-ra/sp registers (starting with 2 up to 22), then invoke the test_loader's ReadWriteRegisterStore
# ecall (number 3) that verifies these indices on the loader side, then adds 100 to each to see that
# the data modifications are correct both ways. After the return from the ecall, the test verifies
# these added values.
# Finally, the test_loader also sets the current RiscvRegister::Sp in a1 and RiscvRegister::Ra in a2
# which are then compared by the test to the expected constants.

_test_start:
    li gp, 2
    li tp, 3
    li t0, 4
    li t1, 5
    li t2, 6
    li s0, 7
    li s1, 8
    li s2, 9
    li s3, 10
    li s4, 11
    li s5, 12
    li s6, 13
    li s7, 14
    li s8, 15
    li s9, 16
    li s10, 17
    li s11, 18
    li t3, 19
    li t4, 20
    li t5, 21
    li t6, 22
    li a7, 3 # Call test ecall number 3 (ReadWriteRegisterStore)
    ecall
    add gp, gp, -102
    bnez gp, fail
    add tp, tp, -103
    bnez tp, fail
    add t0, t0, -104
    bnez t0, fail
    add t1, t1, -105
    bnez t1, fail
    add t2, t2, -106
    bnez t2, fail
    add s0, s0, -107
    bnez s0, fail
    add s1, s1, -108
    bnez s1, fail
    add s2, s2, -109
    bnez s2, fail
    add s3, s3, -110
    bnez s3, fail
    add s4, s4, -111
    bnez s4, fail
    add s5, s5, -112
    bnez s5, fail
    add s6, s6, -113
    bnez s6, fail
    add s7, s7, -114
    bnez s7, fail
    add s8, s8, -115
    bnez s8, fail
    add s9, s9, -116
    bnez s9, fail
    add s10, s10, -117
    bnez s10, fail
    add s11, s11, -118
    bnez s11, fail
    add t3, t3, -119
    bnez t3, fail
    add t4, t4, -120
    bnez t4, fail
    add t5, t5, -121
    bnez t5, fail
    add t6, t6, -122
    bnez t6, fail
    # a1 is supposed to be sp. Expected SP = `_stack_top_value` = 0x14000
    # (16 KiB segment alignment from `SEGMENT_BYTE_ALIGN` in
    # `crates/riscv32/src/common.rs`, plus `_stack_size` = 64K).
    li a3, 0x14000
    sub a1, a1, a3
    bnez a1, fail
    # a2 is supposed to be ra - this the RISC ra
    li a3, 304
    sub a2, a2, a3
    bnez a2, fail
    li a7, 0
    ret
fail:
    li a0, -1
    li a7, 0
    ret
