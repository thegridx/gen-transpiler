PHDRS {
    text PT_LOAD FLAGS(5);  /* 5 = R+X (readable + executable) */
    rodata PT_LOAD FLAGS(4);    /* R only */
    /* `.stack` gets its own NOBITS-only R+W segment (p_filesz == 0). Keeping it out of the `data`
       segment lets `ld` compact `.data`'s file offset right after `.rodata` instead of baking the
       stack's address gap into the file as zero padding. Order matches `SEGMENT_FLAGS` in
       `elf_builder.rs` (text, rodata, stack, data). */
    stack PT_LOAD FLAGS(6);  /* 6 = R+W, NOBITS-only */
    data PT_LOAD FLAGS(6);  /* 6 = R+W (readable + writable) */
}
/* The stack and heap sizes default to 64K; override them with `--defsym`, for example
   `--defsym=_stack_size=128K`. The stack size must be a multiple of 16K. The ecall data size is
   fixed: the host's ecall buffer bound (`MAX_ECALL_INPUT_SIZE_IN_BYTES`) assumes 64K. */
PROVIDE(_stack_size = 64K);
PROVIDE(_heap_size = 64K);
PROVIDE(_ecall_data_size = 64K);

SECTIONS {
    . = 0;
    .text : {
        KEEP(*(.text.init))  /* Protect entry points from --gc-sections */
        *(.text .text.*)
        PROVIDE(_text_end = .);
    } : text
    .init_array : {
        PROVIDE_HIDDEN (__init_array_start = .);
        *(.init_array .init_array.*)
        PROVIDE_HIDDEN (__init_array_end = .);
    } : rodata
    . = ALIGN(16K);
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } : rodata
    . = ALIGN(16K);

    /* `.stack` is placed first in the writable region — as the lowest writable address — so a
       stack-pointer underflow past `_stack_bottom` crosses into the read-only rodata segment and
       faults instead of silently corrupting `.data`/`.bss`. Its size is a multiple of 16K, so
       `.data` abuts it on a 16K boundary and the writable region stays contiguous. */
    .stack (NOLOAD) : ALIGN(16) {
        _stack_bottom = .;
        . = . + _stack_size;
        _stack_top = .;
    } : stack
    . = ALIGN(16K);
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    } : data

    .bss : {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
    } : data

    .heap (NOLOAD) : ALIGN(16) {
        _heap_start = .;
        . = . + _heap_size;
        _heap_end = .;
    } : data
    /* NOLOAD: the program's ecall buffer may be declared here, but no contents are loaded, so
       it reads as zero. Do not put initialised data in this section. */
    .ecall_data (NOLOAD) : ALIGN(16) {
        _ecall_data_start = .;
        *(.ecall_data .ecall_data.*)
        ASSERT(. <= _ecall_data_start + _ecall_data_size, "input in .ecall_data exceeds _ecall_data_size");
        . = _ecall_data_start + _ecall_data_size;
        _ecall_data_end = .;
    } : data

   /* Discard .eh_frame section */
    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)  /* Also discard the header if present */
    }
}
/* Force absolute values for the transpiler (avoids relative linker magic). */
_text_end_value = ABSOLUTE(_text_end - 0x0);
/* `_data_start_value` is the start of the writable region — now `.stack`, the lowest writable
   address. The transpiler and loader use it as the writable-range lower bound, so it tracks the
   region start (the stack), not the `.data` section. */
_data_start_value = ABSOLUTE(_stack_bottom);
/* Top of `.stack`; `_data_end_value` marks the end of the R+W region (`.ecall_data` end). */
_stack_top_value = ABSOLUTE(_stack_top);
_data_end_value = ABSOLUTE(_ecall_data_end);

ASSERT(_stack_size % 16K == 0, "_stack_size must be a multiple of 16K");
/* The host's ecall buffer bound assumes 64K, so overriding the size is an error. */
ASSERT(_ecall_data_end - _ecall_data_start == 64K, "_ecall_data_size is fixed at 64K");
