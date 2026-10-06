//! Minimal `no_std` RISC-V program with four arithmetic entry points, used by
//! the `rust-math` smoke-test suite. It has no dependencies: the functions use
//! the plain C calling convention, take one `u32` in `a0` and return one `u32`.
#![no_std]
#![no_main]

#[unsafe(link_section = ".text.init")]
#[unsafe(no_mangle)]
pub extern "C" fn sum_1_to_x(x: u32) -> u32 {
    let mut sum = 0;
    for i in 1..=x {
        sum += i;
    }
    sum
}

#[unsafe(link_section = ".text.init")]
#[unsafe(no_mangle)]
pub extern "C" fn sum_squares_1_to_x(x: u32) -> u32 {
    let mut sum_squares = 0;
    for i in 1..=x {
        sum_squares += i * i;
    }
    sum_squares
}

#[unsafe(link_section = ".text.init")]
#[unsafe(no_mangle)]
pub extern "C" fn sum_cubes_1_to_x(x: u32) -> u32 {
    let mut sum_cubes = 0;
    for i in 1..=x {
        sum_cubes += i * i * i;
    }
    sum_cubes
}

#[unsafe(link_section = ".text.init")]
#[unsafe(no_mangle)]
pub extern "C" fn factorial_x(x: u32) -> u32 {
    let mut factorial = 1;
    for i in 1..=x {
        factorial *= i;
    }
    factorial
}

// SAFETY: The function pointers are valid because the referenced functions are marked
// with #[unsafe(no_mangle)] ensuring their symbols are preserved in the binary.
// Keep entry points alive - prevents dead code elimination.
// Using transmute to cast different function signatures to a common type.
#[used]
#[unsafe(no_mangle)]
pub static ENTRY_POINTS: [unsafe extern "C" fn(); 4] = unsafe {
    [
        core::mem::transmute(sum_1_to_x as extern "C" fn(u32) -> u32),
        core::mem::transmute(sum_squares_1_to_x as extern "C" fn(u32) -> u32),
        core::mem::transmute(sum_cubes_1_to_x as extern "C" fn(u32) -> u32),
        core::mem::transmute(factorial_x as extern "C" fn(u32) -> u32),
    ]
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop()
    }
}
