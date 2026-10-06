# Gas edge cases. Each case sets its gas with ecall 5 (the test loader's SetGasToW0) and then
# runs. Gas is charged per pseudo basic block, up front: resuming after the ecall charges the
# rest of the block it resumes in, and a `jalr` into the middle of a block charges the rest of
# that block. A case with exactly enough gas returns normally; one with one less runs out.
.section .text
        .align 2
# Entry points must be global symbols.
        .globl block_exact
        .globl block_one_short
        .globl mid_block_exact
        .globl mid_block_one_short
        .globl loop_runs_out

# The block after the ecall is `addi, addi, ret`: 3 gas.
block_exact:
        li a0, 3
        li a7, 5
        ecall
        addi a0, zero, 7
        addi a0, a0, 0
        ret

block_one_short:
        li a0, 2
        li a7, 5
        ecall
        addi a0, zero, 7
        addi a0, a0, 0
        ret

# After the ecall: the `jalr` block (1 gas), then the rest of the target's block from the
# target on, `addi, ret` (2 gas). The target is reached only indirectly, so it does not start a
# block of its own.
mid_block_exact:
        la t0, mid_block_exact_target
        li a0, 3
        li a7, 5
        ecall
        jalr zero, 0(t0)
        addi a0, zero, 1
mid_block_exact_target:
        addi a0, zero, 9
        ret

mid_block_one_short:
        la t0, mid_block_one_short_target
        li a0, 2
        li a7, 5
        ecall
        jalr zero, 0(t0)
        addi a0, zero, 1
mid_block_one_short_target:
        addi a0, zero, 9
        ret

# 100 iterations of a 2-gas loop body with 50 gas: runs out part-way.
loop_runs_out:
        li a0, 50
        li a7, 5
        ecall
        li t0, 100
loop:
        addi t0, t0, -1
        bnez t0, loop
        ret
