.section .text
    .align 2

# Function 1: Return sum of 4 BSS elements
    .globl sum_bss
    .type sum_bss, @function
sum_bss:
    la t0, bss_data
    lbu t1, 0(t0)
    lbu t2, 1(t0)
    lbu t3, 2(t0)
    lbu t4, 3(t0)
    add a0, t1, t2
    add a0, a0, t3
    add a0, a0, t4    
    ret

# Function 2: Return product of 4 BSS elements
    .globl product_bss
    .type product_bss, @function
product_bss:
    la t0, bss_data
    lbu t1, 0(t0)
    lbu t2, 1(t0)
    lbu t3, 2(t0)
    lbu t4, 3(t0)
    mul a0, t1, t2
    mul a0, a0, t3
    mul a0, a0, t4
    ret

# Function 3: Increase all BSS elements by 1
    .globl increment_bss
    .type increment_bss, @function
increment_bss:
    la t0, bss_data
    lbu t1, 0(t0)
    addi t1, t1, 1
    sb t1, 0(t0)
    
    lbu t1, 1(t0)
    addi t1, t1, 1
    sb t1, 1(t0)
    
    lbu t1, 2(t0)
    addi t1, t1, 1
    sb t1, 2(t0)
    
    lbu t1, 3(t0)
    addi t1, t1, 1
    sb t1, 3(t0)
    li a0,0
    ret

# Function 4: move sp to a0
    .globl move_sp_to_a0
    .type move_sp_to_a0, @function
move_sp_to_a0:
    mv a0, sp
    ret

# Init functions
    .globl init_func_0
    .type init_func_0, @function
init_func_0:
    la t0, bss_data
    li t1, 2
    sb t1, 0(t0)
    ret

    .globl init_func_1
    .type init_func_1, @function
init_func_1:
    la t0, bss_data
    li t1, 3
    sb t1, 1(t0)
    ret

    .globl init_func_2
    .type init_func_2, @function
init_func_2:
    la t0, bss_data
    li t1, 5
    sb t1, 2(t0)
    ret

    .globl init_func_3
    .type init_func_3, @function
init_func_3:
    la t0, bss_data
    li t1, 7
    sb t1, 3(t0)
    ret

    .section .bss
    .align 2
    .globl bss_data
bss_data:
    .skip 16        # 4 words (16 bytes)

    .section .init_array, "aw"
    .align 2
    .word init_func_0
    .word init_func_1
    .word init_func_2
    .word init_func_3
