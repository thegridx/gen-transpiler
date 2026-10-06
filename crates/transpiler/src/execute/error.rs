//! Errors returned while loading and running a transpiled program.
use riscv32::common::ValueError;
use thiserror::Error;
use user_code_runner::runner::UserCodeRunnerError;

use crate::layout::LayoutValidationError;

/// Why loading a transpiled ELF, or accessing the memory of a loaded one, failed.
#[derive(Error, Debug)]
pub enum LoaderError {
    #[error("value error: {0}")]
    ValueError(#[from] ValueError),
    #[error("layout validation error: {0}")]
    LayoutValidation(#[from] LayoutValidationError),
    #[error("compiled with wrong architecture")]
    CompiledWithWrongArchitecture,
    #[error("transpiled code cannot run on this host: it needs aarch64, or x86_64 on Linux")]
    UnsupportedHost,
    #[error("unknown ELF e_flags {e_flags:#x}: not a transpiler artifact")]
    UnknownElfFlags { e_flags: u32 },
    #[error(
        "host page size {page_size:#x} is not supported: segments are aligned to {max_page_size:#x}"
    )]
    UnsupportedPageSize {
        page_size: usize,
        max_page_size: usize,
    },
    #[error("section {section} at {addr:#x} is misaligned")]
    MisalignedSection { section: String, addr: usize },
    #[error("trusted artifact loaded in an untrusted context")]
    TrustedArtifactInUntrustedContext,
    #[error("entry points do not comply with range: {indices:?} != ({first}, {last})")]
    EntryPointsDoNotComplyWithRange {
        indices: Vec<u32>,
        first: usize,
        last: usize,
    },
    #[error("unaligned symbol {symbol}")]
    UnalignedSymbol { symbol: String },
    #[error("too many init functions: {count} exceeds max {max}")]
    TooManyInitFunctions { count: usize, max: usize },
    #[error("too many entry points: {count} exceeds max {max}")]
    TooManyEntryPoints { count: usize, max: usize },
    #[error("ELF parsing error: {message}")]
    ElfParsingError { message: String },
    #[error("mmap syscall failed ({syscall}): {error}")]
    MmapSyscallFailed { syscall: String, error: String },
    #[error("mmap segment at {vaddr:#x} failed: {error}")]
    MmapSegmentFailed { vaddr: usize, error: String },
    #[error("expected ELF64 format")]
    UnsupportedElfFormat,
    #[error("no PT_LOAD segments in ELF")]
    NoLoadSegments,
    #[error("ELF too large: {size_in_bytes} bytes exceeds max {max_in_bytes}")]
    ElfFileTooLarge {
        size_in_bytes: usize,
        max_in_bytes: usize,
    },
    #[error("mmap total span too large: {span_in_bytes:#x} exceeds max {max_in_bytes:#x}")]
    MmapSpanTooLarge {
        span_in_bytes: usize,
        max_in_bytes: usize,
    },
    #[error("invalid segment: filesz ({filesz:#x}) > memsz ({memsz:#x})")]
    InvalidSegmentFileszExceedsMemsz { filesz: usize, memsz: usize },
    #[error(
        "segment file range out of bounds: offset {offset:#x} + filesz {filesz:#x} > file_size {file_size_in_bytes:#x}"
    )]
    SegmentFileRangeOutOfBounds {
        offset: usize,
        filesz: usize,
        file_size_in_bytes: usize,
    },
    #[error("the jump table is in the writable segment at vaddr {vaddr:#x}")]
    WritableJumpTable { vaddr: usize },
    #[error("segment has forbidden W+X permissions at vaddr {vaddr:#x}")]
    WritableExecutableSegment { vaddr: usize },
    #[error("segment vaddr {vaddr:#x} is not page-aligned")]
    UnalignedSegmentVaddr { vaddr: usize },
    #[error("section not found: {section}")]
    SectionNotFound { section: String },
    #[error(
        "init array outside entry points: entry_points_begin={entry_points_begin:#x}, init_array_end={init_array_end:#x}, entry_points_end={entry_points_end:#x}"
    )]
    InitArrayOutsideEntryPoints {
        entry_points_begin: usize,
        init_array_end: usize,
        entry_points_end: usize,
    },
    #[error(
        "init array unaligned: init_array_size_in_bytes={init_array_size_in_bytes:#x}, branch_size_in_bytes={branch_size_in_bytes}"
    )]
    InitArrayUnaligned {
        init_array_size_in_bytes: usize,
        branch_size_in_bytes: usize,
    },
    #[error(
        "entry points unaligned: entry_points_byte_size={entry_points_size_in_bytes:#x}, branch_byte_size={branch_size_in_bytes}"
    )]
    EntryPointsUnaligned {
        entry_points_size_in_bytes: usize,
        branch_size_in_bytes: usize,
    },
    #[error(
        "{access} range [{start:#x}, {end:#x}) is not covered by PT_LOAD segments from {uncovered:#x}"
    )]
    RangeNotMapped {
        access: String,
        start: usize,
        end: usize,
        uncovered: usize,
    },
    #[error(
        "PT_LOAD segments overlap: [{first_start:#x}, {first_end:#x}) and one starting at {second_start:#x}"
    )]
    OverlappingSegments {
        first_start: usize,
        first_end: usize,
        second_start: usize,
    },
}

impl From<LoaderError> for UserCodeRunnerError {
    fn from(e: LoaderError) -> Self {
        UserCodeRunnerError::Runner(Box::new(e))
    }
}

#[cfg(test)]
mod tests {
    use user_code_runner::runner::UserCodeRunnerError;

    use super::LoaderError;

    #[test]
    fn loader_errors_stay_typed_inside_runner_errors() {
        let error = UserCodeRunnerError::from(LoaderError::NoLoadSegments);
        let UserCodeRunnerError::Runner(source) = error else {
            panic!("expected a runner error, got {error:?}");
        };
        assert!(matches!(
            source.downcast_ref::<LoaderError>(),
            Some(LoaderError::NoLoadSegments)
        ));
    }
}
