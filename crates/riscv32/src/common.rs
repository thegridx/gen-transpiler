//! Generic, crate-agnostic utilities.
use strum::EnumIter;
use thiserror::Error;

#[derive(Clone, Copy, Debug, EnumIter, Default)]
pub enum SignType {
    #[default]
    Signed,
    Unsigned,
}

#[derive(Error, Debug)]
pub enum ValueError {
    #[error("bad alignment: {align}, context: {context}")]
    BadAlignment { align: u32, context: String },
    #[error("value above max u32: {value}, context: {context}")]
    ValueAboveMaxU32 { value: u64, context: String },
    #[error("overflow: base: {base}, add: {add}, context: {context}")]
    Overflow {
        base: u64,
        add: u64,
        context: String,
    },
    #[error("underflow: base: {base}, sub: {sub}, context: {context}")]
    Underflow {
        base: u64,
        sub: u64,
        context: String,
    },
    #[error("mul overflow: base: {base}, mul: {mul}, context: {context}")]
    MulOverflow {
        base: u64,
        mul: u64,
        context: String,
    },
}

/// Alignment of `PT_LOAD` segments in input RISC-V executables and transpiled ELFs: `-z max-page-size`
/// `ALIGN(...)` in `riscv_elf_mem_layout.x`, intra-segment padding inside `compute_layout`,
/// and the per-target `elf_segment_alignment()` returned by each transpiler envelope.
///
/// Fixed at 16 KiB on any host. The choice satisfies `SEGMENT_BYTE_ALIGN >= host_page_size` for every supported host
/// (4 KiB Linux x86_64 / aarch64, 16 KiB Apple Silicon), so per-segment `mprotect` works at runtime, and is
/// host-independent so RISC-V executables are byte-identical regardless of where they were built.
/// 64 KiB-page Linux ARM64 is intentionally out of scope; see `HOST_PAGE_SIZE_BYTES` in the transpiler's `execute/loader.rs`.
pub const SEGMENT_BYTE_ALIGN: u32 = 0x4000;

pub fn check_alignment(alignment: u32, context: &str) -> Result<(), ValueError> {
    if alignment == 0 || !alignment.is_power_of_two() {
        Err(ValueError::BadAlignment {
            align: alignment,
            context: context.to_string(),
        })
    } else {
        Ok(())
    }
}

/// Attempts to align up to the next multiple of `alignment`.
/// `alignment` must be a power of two.
pub fn checked_align_to(addr: u32, alignment: u32, context: &str) -> Result<u32, ValueError> {
    check_alignment(alignment, context)?;
    addr.checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
        .ok_or(ValueError::Overflow {
            base: addr as u64,
            add: (alignment - 1) as u64,
            context: context.to_string(),
        })
}

/// Align `addr` upward to the next [`SEGMENT_BYTE_ALIGN`] boundary.
/// Used for intra-segment paddings in produced ELFs; do not use for runtime page-aligned `mmap` math!
/// (that's the host page size, see `HOST_PAGE_SIZE_BYTES` in the transpiler's `execute/loader.rs`).
pub fn checked_align_segment(addr: u32, context: &str) -> Result<u32, ValueError> {
    checked_align_to(addr, SEGMENT_BYTE_ALIGN, context)
}

pub fn checked_usize_to_u32(u: usize, context: &str) -> Result<u32, ValueError> {
    u32::try_from(u).map_err(|_| ValueError::ValueAboveMaxU32 {
        value: u as u64,
        context: context.to_string(),
    })
}

pub fn checked_u64_to_u32(u: u64, context: &str) -> Result<u32, ValueError> {
    u32::try_from(u).map_err(|_| ValueError::ValueAboveMaxU32 {
        value: u,
        context: context.to_string(),
    })
}

pub(crate) fn checked_add_u16(a: u16, b: u16, context: &str) -> Result<u16, ValueError> {
    a.checked_add(b).ok_or_else(|| ValueError::Overflow {
        base: a as u64,
        add: b as u64,
        context: context.to_string(),
    })
}

pub fn checked_add_u32(a: u32, b: u32, context: &str) -> Result<u32, ValueError> {
    a.checked_add(b).ok_or_else(|| ValueError::Overflow {
        base: a as u64,
        add: b as u64,
        context: context.to_string(),
    })
}

pub fn checked_add_usize(a: usize, b: usize, context: &str) -> Result<usize, ValueError> {
    a.checked_add(b).ok_or_else(|| ValueError::Overflow {
        base: a as u64,
        add: b as u64,
        context: context.to_string(),
    })
}

pub fn checked_sub_u32(a: u32, b: u32, context: &str) -> Result<u32, ValueError> {
    a.checked_sub(b).ok_or_else(|| ValueError::Underflow {
        base: a as u64,
        sub: b as u64,
        context: context.to_string(),
    })
}

pub fn checked_sub_usize(a: usize, b: usize, context: &str) -> Result<usize, ValueError> {
    a.checked_sub(b).ok_or_else(|| ValueError::Underflow {
        base: a as u64,
        sub: b as u64,
        context: context.to_string(),
    })
}

pub fn checked_mul_u32(a: u32, b: u32, context: &str) -> Result<u32, ValueError> {
    a.checked_mul(b).ok_or_else(|| ValueError::MulOverflow {
        base: a as u64,
        mul: b as u64,
        context: context.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // The two tests below exercise `checked_align_to` with a fixed 4 KiB alignment; they're algorithm tests,
    // not page-size tests, so they intentionally pass an explicit constant rather than `SEGMENT_BYTE_ALIGN`.
    const TEST_ALIGN: u32 = 0x1000;

    #[test]
    fn align_to_next_page_already_aligned_is_unchanged() {
        assert_eq!(checked_align_to(0, TEST_ALIGN, "test").unwrap(), 0);
        assert_eq!(
            checked_align_to(0x1000, TEST_ALIGN, "test").unwrap(),
            0x1000
        );
        assert_eq!(
            checked_align_to(0x2000, TEST_ALIGN, "test").unwrap(),
            0x2000
        );
        assert_eq!(
            checked_align_to(0xFF000, TEST_ALIGN, "test").unwrap(),
            0xFF000
        );
    }

    #[test]
    fn align_to_next_page_unaligned_rounds_up_to_next_boundary() {
        assert_eq!(checked_align_to(1, TEST_ALIGN, "test").unwrap(), 0x1000);
        assert_eq!(checked_align_to(0xFFF, TEST_ALIGN, "test").unwrap(), 0x1000);
        assert_eq!(
            checked_align_to(0x1001, TEST_ALIGN, "test").unwrap(),
            0x2000
        );
        assert_eq!(
            checked_align_to(0x1FFF, TEST_ALIGN, "test").unwrap(),
            0x2000
        );
    }

    #[test]
    fn align_up_correctly_aligns_values_to_boundaries() {
        assert_eq!(checked_align_to(0, SEGMENT_BYTE_ALIGN, "test").unwrap(), 0);
        assert_eq!(
            checked_align_to(1, SEGMENT_BYTE_ALIGN, "test").unwrap(),
            SEGMENT_BYTE_ALIGN
        );
        assert_eq!(
            checked_align_to(SEGMENT_BYTE_ALIGN, SEGMENT_BYTE_ALIGN, "test").unwrap(),
            SEGMENT_BYTE_ALIGN
        );
        assert_eq!(
            checked_align_to(SEGMENT_BYTE_ALIGN + 1, SEGMENT_BYTE_ALIGN, "test").unwrap(),
            SEGMENT_BYTE_ALIGN * 2
        );
    }

    #[test]
    fn align_up_small_alignments() {
        assert_eq!(checked_align_to(0, 4, "test").unwrap(), 0);
        assert_eq!(checked_align_to(1, 4, "test").unwrap(), 4);
        assert_eq!(checked_align_to(3, 4, "test").unwrap(), 4);
        assert_eq!(checked_align_to(4, 4, "test").unwrap(), 4);
        assert_eq!(checked_align_to(5, 4, "test").unwrap(), 8);
    }

    #[rstest]
    #[case(1)]
    #[case(2)]
    #[case(4)]
    #[case(8)]
    #[case(SEGMENT_BYTE_ALIGN)]
    #[case(1 << 31)]
    fn check_alignment_accepts_powers_of_two(#[case] align: u32) {
        let result = check_alignment(align, "test");
        assert!(
            result.is_ok(),
            "alignment {align} should be accepted, got {result:?}"
        );
    }

    #[test]
    fn check_alignment_rejects_zero_and_non_powers_of_two() {
        let result_zero = check_alignment(0, "ctx");
        assert!(
            matches!(result_zero, Err(ValueError::BadAlignment { align: 0, .. })),
            "expected BadAlignment {{ align: 0 }}, got {result_zero:?}"
        );
        let result_three = check_alignment(3, "ctx");
        assert!(
            matches!(result_three, Err(ValueError::BadAlignment { align: 3, .. })),
            "expected BadAlignment {{ align: 3 }}, got {result_three:?}"
        );
        let result_six = check_alignment(6, "ctx");
        assert!(
            matches!(result_six, Err(ValueError::BadAlignment { align: 6, .. })),
            "expected BadAlignment {{ align: 6 }}, got {result_six:?}"
        );
        let result_unaligned = check_alignment(0x1001, "ctx");
        assert!(
            matches!(result_unaligned, Err(ValueError::BadAlignment { .. })),
            "expected BadAlignment, got {result_unaligned:?}"
        );
    }

    #[test]
    fn checked_align_to_propagates_bad_alignment() {
        let result_align_zero = checked_align_to(0x100, 0, "test");
        assert!(
            matches!(result_align_zero, Err(ValueError::BadAlignment { .. })),
            "expected BadAlignment, got {result_align_zero:?}"
        );
        let result_align_three = checked_align_to(0x100, 3, "test");
        assert!(
            matches!(result_align_three, Err(ValueError::BadAlignment { .. })),
            "expected BadAlignment, got {result_align_three:?}"
        );
    }

    #[test]
    fn checked_align_to_errors_on_overflow() {
        // addr + (alignment - 1) must not overflow
        let result_max = checked_align_to(u32::MAX, SEGMENT_BYTE_ALIGN, "test");
        assert!(
            matches!(result_max, Err(ValueError::Overflow { .. })),
            "expected Overflow, got {result_max:?}"
        );
        let result_near_max = checked_align_to(u32::MAX - 1, 4, "test");
        assert!(
            matches!(result_near_max, Err(ValueError::Overflow { .. })),
            "expected Overflow, got {result_near_max:?}"
        );
    }

    #[test]
    fn checked_align_segment_wraps_checked_align_to() {
        assert_eq!(checked_align_segment(0, "t").unwrap(), 0);
        assert_eq!(checked_align_segment(1, "t").unwrap(), SEGMENT_BYTE_ALIGN);
        assert_eq!(
            checked_align_segment(SEGMENT_BYTE_ALIGN, "t").unwrap(),
            SEGMENT_BYTE_ALIGN
        );
        assert_eq!(
            checked_align_segment(SEGMENT_BYTE_ALIGN + 1, "t").unwrap(),
            SEGMENT_BYTE_ALIGN * 2
        );
        let result = checked_align_segment(u32::MAX, "t");
        assert!(
            matches!(result, Err(ValueError::Overflow { .. })),
            "expected Overflow, got {result:?}"
        );
    }

    #[test]
    fn checked_usize_to_u32_ok_and_err() {
        assert_eq!(checked_usize_to_u32(0, "t").unwrap(), 0);
        assert_eq!(checked_usize_to_u32(42, "t").unwrap(), 42);
        assert_eq!(
            checked_usize_to_u32(u32::MAX as usize, "t").unwrap(),
            u32::MAX
        );
        // Only meaningful on 64-bit platforms where usize > u32.
        #[cfg(target_pointer_width = "64")]
        {
            let err = checked_usize_to_u32(u32::MAX as usize + 1, "ctx").unwrap_err();
            assert!(
                matches!(
                    err,
                    ValueError::ValueAboveMaxU32 {
                        value,
                        ..
                    } if value == (u32::MAX as u64) + 1
                ),
                "expected ValueAboveMaxU32 {{ value: u32::MAX + 1 }}, got {err:?}"
            );
        }
    }

    #[test]
    fn checked_u64_to_u32_ok_and_err() {
        assert_eq!(checked_u64_to_u32(0, "t").unwrap(), 0);
        assert_eq!(checked_u64_to_u32(1234, "t").unwrap(), 1234);
        assert_eq!(checked_u64_to_u32(u32::MAX as u64, "t").unwrap(), u32::MAX);
        let err = checked_u64_to_u32(u32::MAX as u64 + 1, "ctx").unwrap_err();
        assert!(
            matches!(
                err,
                ValueError::ValueAboveMaxU32 { value, .. } if value == (u32::MAX as u64) + 1
            ),
            "expected ValueAboveMaxU32 {{ value: u32::MAX + 1 }}, got {err:?}"
        );
    }

    #[test]
    fn checked_add_u32_ok_and_overflow() {
        assert_eq!(checked_add_u32(0, 0, "t").unwrap(), 0);
        assert_eq!(checked_add_u32(10, 32, "t").unwrap(), 42);
        assert_eq!(checked_add_u32(u32::MAX - 1, 1, "t").unwrap(), u32::MAX);
        let err = checked_add_u32(u32::MAX, 1, "ctx").unwrap_err();
        assert!(
            matches!(
                err,
                ValueError::Overflow { base, add, .. }
                    if base == u32::MAX as u64 && add == 1
            ),
            "expected Overflow {{ base: u32::MAX, add: 1 }}, got {err:?}"
        );
    }

    #[test]
    fn checked_add_usize_ok_and_overflow() {
        assert_eq!(checked_add_usize(0, 0, "t").unwrap(), 0);
        assert_eq!(checked_add_usize(100, 200, "t").unwrap(), 300);
        let err = checked_add_usize(usize::MAX, 1, "ctx").unwrap_err();
        assert!(
            matches!(err, ValueError::Overflow { .. }),
            "expected Overflow, got {err:?}"
        );
    }

    #[test]
    fn checked_sub_u32_ok_and_underflow() {
        assert_eq!(checked_sub_u32(10, 0, "t").unwrap(), 10);
        assert_eq!(checked_sub_u32(10, 10, "t").unwrap(), 0);
        assert_eq!(checked_sub_u32(0x1000, 0x1, "t").unwrap(), 0xFFF);
        let err = checked_sub_u32(0, 1, "ctx").unwrap_err();
        assert!(
            matches!(
                err,
                ValueError::Underflow {
                    base: 0,
                    sub: 1,
                    ..
                }
            ),
            "expected Underflow {{ base: 0, sub: 1 }}, got {err:?}"
        );
    }

    #[test]
    fn checked_sub_usize_ok_and_underflow() {
        assert_eq!(checked_sub_usize(100, 1, "t").unwrap(), 99);
        assert_eq!(checked_sub_usize(0, 0, "t").unwrap(), 0);
        let err = checked_sub_usize(0, 1, "ctx").unwrap_err();
        assert!(
            matches!(err, ValueError::Underflow { .. }),
            "expected Underflow, got {err:?}"
        );
    }

    #[test]
    fn checked_mul_u32_ok_and_overflow() {
        assert_eq!(checked_mul_u32(0, 123, "t").unwrap(), 0);
        assert_eq!(checked_mul_u32(6, 7, "t").unwrap(), 42);
        assert_eq!(checked_mul_u32(u32::MAX, 1, "t").unwrap(), u32::MAX);
        let result_times_two = checked_mul_u32(u32::MAX, 2, "ctx");
        assert!(
            matches!(result_times_two, Err(ValueError::MulOverflow { .. })),
            "expected MulOverflow, got {result_times_two:?}"
        );
        let result_squared = checked_mul_u32(1 << 16, 1 << 16, "ctx");
        assert!(
            matches!(result_squared, Err(ValueError::MulOverflow { .. })),
            "expected MulOverflow, got {result_squared:?}"
        );
    }

    #[test]
    fn error_context_is_preserved() {
        match checked_add_u32(u32::MAX, 1, "my_ctx") {
            Err(ValueError::Overflow { context, .. }) => assert_eq!(context, "my_ctx"),
            other => panic!("expected Overflow, got {other:?}"),
        }
        match checked_sub_u32(0, 1, "sub_ctx") {
            Err(ValueError::Underflow { context, .. }) => assert_eq!(context, "sub_ctx"),
            other => panic!("expected Underflow, got {other:?}"),
        }
        match check_alignment(3, "align_ctx") {
            Err(ValueError::BadAlignment { context, .. }) => {
                assert_eq!(context, "align_ctx")
            }
            other => panic!("expected BadAlignment, got {other:?}"),
        }
        match checked_u64_to_u32(u64::MAX, "u64_ctx") {
            Err(ValueError::ValueAboveMaxU32 { context, .. }) => {
                assert_eq!(context, "u64_ctx")
            }
            other => panic!("expected ValueAboveMaxU32, got {other:?}"),
        }
        match checked_mul_u32(u32::MAX, 2, "mul_ctx") {
            Err(ValueError::MulOverflow { context, .. }) => assert_eq!(context, "mul_ctx"),
            other => panic!("expected MulOverflow, got {other:?}"),
        }
    }
}
