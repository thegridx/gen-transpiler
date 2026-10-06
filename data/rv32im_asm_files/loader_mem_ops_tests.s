.section .text
.globl _test_start
_test_start:
    la a0, test_string              # a0 = &test_string
    la a1, test_string_end          # a1 = &test_string_end
    sub a1, a1, a0                  # a1 = len = end - start
    mv s0, a0                       # s0 = original_ptr (for verification)
    mv s3, a1                       # s3 = len (loop counter)
    la a2, copy_buffer              # a2 = &copy_buffer
    mv s2, a2                       # s2 = buffer_ptr (for verification)
    li a7, 4                        # ecall_code = 4 (copy string)
    ecall                           # Copy string to buffer and stack
    mv s1, sp                       # s1 = stack_ptr (string copied here)
    li a7, 0                        # Clear ecall code
verify_buffer_loop:
    lb t0, 0(s0)                    # t0 = original[i]
    lb t1, 0(s1)                    # t1 = stack_copy[i]
    lb t2, 0(s2)                    # t2 = buffer_copy[i]
    bne t0, t1, fail                # if (original != stack_copy) fail
    bne t0, t2, fail                # if (original != buffer_copy) fail
    addi s0, s0, 1                  # s0++
    addi s1, s1, 1                  # s1++
    addi s2, s2, 1                  # s2++
    addi s3, s3, -1                 # len--
    bnez s3, verify_buffer_loop     # while (len > 0) loop
success:        
    li a0, 0                        # return 0
    ret
fail:
    li a0, -1                       # return -1
    ret
.section .data
    .string "just a junk string"        
test_string:
    .string "the quick brown fox jumps over the lazy dog"
test_string_end:
    .string "yet another junk string"
copy_buffer:
    .space 64                       # 64-byte buffer
