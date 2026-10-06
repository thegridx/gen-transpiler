# test that the whole guest memory range can be accessed, up to its last byte
# Guest loads may read anywhere in [0, _data_end_value). This loads one word from every 4 KiB
# page of that range, so an unmapped gap between output segments faults on a host with 4 KiB
# pages; the `.rodata` below makes the read-only segment end short of a 16 KiB boundary. It then
# loads, stores and runs an atomic on the last bytes before `_data_end_value`.
# Returns 0 on success in a0
.section .text
.globl _test_start

_test_start:
    lui t1, %hi(_data_end_value)
    addi t1, t1, %lo(_data_end_value)
    li t0, 0
    li t3, 4096

read_page:
    lw t2, 0(t0)
    add t0, t0, t3
    bltu t0, t1, read_page

    # Accesses that end exactly at the end of guest memory
    lw t2, -4(t1)
    lh t2, -2(t1)
    lb t2, -1(t1)
    sw zero, -4(t1)
    sh zero, -2(t1)
    sb zero, -1(t1)
    addi t4, t1, -4
    amoadd.w t2, zero, (t4)

    li a7, 0    # no panic
    li a0, 0    # success
    ret

.section .rodata
    .align 4
rodata_word:
    .word 0x12345678
