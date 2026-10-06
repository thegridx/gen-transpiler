//! The transpiler rejects malformed or unsupported RISC-V input with an error instead of
//! panicking (which aborts the host process) or silently miscompiling it.

mod riscv_elf_spec;

use std::collections::HashSet;

use riscv_elf_spec::{CODE, RiscvElfSpec, TestSection, TestSymbol};

use object::elf;
use riscv32::gas_cost::{RvGasCost, RvGasCostScheme};
use riscv32::immediate::ImmU12_0;
use riscv32::registers::RvPreservedRegister;
use riscv32_elf::reader::RvElfParseError;
use transpiler::ElfBuilderError;
use transpiler::execute::{LoaderContext, TranspiledCodeRunner, TranspiledVm};
use transpiler::{
    TranspilationMode, TranspilerError, TranspilerForArchitecture, TranspilerInput,
    TranspilerOutput,
};
use user_code_runner::gas::UserCodeGasAmount;
use user_code_runner::runner::{
    CodeRunner as _, EcallHandler, ILLEGAL_INSTRUCTION_EXIT_VALUE, MemoryAccessor as _,
    NORMAL_EXIT_VALUE, PANIC_EXIT_VALUE, RegistersInOut, RunOutcome, UserCodeRunnerError,
    call_entry_point, run_init_array,
};

fn transpile(
    elf: &[u8],
    mode: &TranspilationMode,
) -> Vec<Result<TranspilerOutput, TranspilerError>> {
    [
        TranspilerForArchitecture::Arm64,
        TranspilerForArchitecture::X86_64,
    ]
    .iter()
    .map(|transpiler| {
        transpiler.transpile_to_native_elf(
            TranspilerInput {
                riscv_elf_bytes: elf,
                entry_points: HashSet::from(["main".to_string()]),
            },
            mode,
        )
    })
    .collect()
}

/// Asserts that every target rejects `elf` with an error matching `expected`.
fn assert_rejected(
    elf: &[u8],
    mode: &TranspilationMode,
    expected: impl Fn(&TranspilerError) -> bool,
    what: &str,
) {
    for result in transpile(elf, mode) {
        match result {
            Err(e) if expected(&e) => {}
            Err(e) => panic!("{what}: unexpected error {e}"),
            Ok(_) => panic!("{what}: transpiled successfully"),
        }
    }
}

#[test]
fn well_formed_input_transpiles() {
    for result in transpile(
        &RiscvElfSpec::default().build(),
        &TranspilationMode::default(),
    ) {
        if let Err(e) = result {
            panic!("baseline input should transpile: {e}");
        }
    }
}

/// Undecodable words cost no gas, so they never close a block on cost. A long run of them
/// followed by a load must still transpile: on aarch64 the load's bounds check reaches its
/// block's panic branch with a `B.cond`, which spans only ±1 MiB.
#[test]
fn long_zero_cost_run_before_a_load_transpiles() {
    const ILLEGAL: u32 = 0xFFFF_FFFF;
    const LW_A0_0_A0: u32 = 0x0005_2503;
    let mut code = vec![ILLEGAL; 300_000];
    code.extend([LW_A0_0_A0, RET]);
    let elf = RiscvElfSpec {
        code,
        ..RiscvElfSpec::default()
    }
    .build();
    for result in transpile(&elf, &TranspilationMode::default()) {
        if let Err(e) = result {
            panic!("a long run of undecodable words should transpile: {e}");
        }
    }
}

/// Untrusted output must be metered, so a zero gas cost is rejected.
#[test]
fn untrusted_mode_rejects_a_zero_gas_cost() {
    let elf = RiscvElfSpec::default().build();
    let mode = TranspilationMode::Untrusted(RvGasCostScheme::Fixed(RvGasCost::zero()));
    for result in transpile(&elf, &mode) {
        assert!(
            matches!(result, Err(TranspilerError::ZeroGasCost)),
            "a zero gas cost should be rejected"
        );
    }
}

/// A `static` function in one file may share its name with another file's global entry point.
#[test]
fn local_symbol_named_like_an_entry_point_is_ignored() {
    let elf = RiscvElfSpec {
        local_entry_alias: true,
        ..RiscvElfSpec::default()
    }
    .build();
    for result in transpile(&elf, &TranspilationMode::default()) {
        if let Err(e) = result {
            panic!("a local `main` beside the global one should be ignored: {e}");
        }
    }
}

#[test]
fn code_not_at_address_zero_is_rejected() {
    let elf = RiscvElfSpec {
        text_addr: 0x1000,
        ..RiscvElfSpec::default()
    }
    .build();
    assert_rejected(
        &elf,
        &TranspilationMode::default(),
        |e| {
            matches!(
                e,
                TranspilerError::RiscvElfParse(RvElfParseError::CodeSectionNotAtZero {
                    address: 0x1000
                })
            )
        },
        ".text at 0x1000",
    );
}

#[test]
fn text_end_past_the_code_is_rejected() {
    let elf = RiscvElfSpec {
        text_end: Some(0x2000),
        ..RiscvElfSpec::default()
    }
    .build();
    assert_rejected(
        &elf,
        &TranspilationMode::default(),
        |e| {
            matches!(
                e,
                TranspilerError::RiscvElfParse(RvElfParseError::TextEndMismatch {
                    text_end: 0x2000,
                    code_end: 8,
                })
            )
        },
        "_text_end_value past the code",
    );
}

#[test]
fn gas_cost_above_a_block_is_rejected() {
    let mode = TranspilationMode::Untrusted(RvGasCostScheme::Fixed(RvGasCost::new(ImmU12_0(5000))));
    assert_rejected(
        &RiscvElfSpec::default().build(),
        &mode,
        |e| matches!(e, TranspilerError::GasCostTooLarge { cost: 5000, .. }),
        "gas cost 5000",
    );
}

#[test]
fn unmerged_small_data_section_is_rejected() {
    // GCC emits `.sdata` by default. Copied after `.data`, its bytes would land at the wrong
    // guest address.
    let elf = RiscvElfSpec {
        data: vec![(".data", vec![1, 2, 3, 4], 4), (".sdata", vec![9; 8], 8)],
        ..RiscvElfSpec::default()
    }
    .build();
    assert_rejected(
        &elf,
        &TranspilationMode::default(),
        |e| matches!(e, TranspilerError::SectionsNotMerged { first, second } if first == ".data" && second == ".sdata"),
        ".data plus .sdata",
    );
}

#[test]
fn sections_at_unexpected_addresses_are_rejected() {
    for (spec, section_name) in [
        (
            // Inside the first segment, below where the output can place it after the code.
            RiscvElfSpec {
                rodata_addr: Some(0x100),
                ..RiscvElfSpec::default()
            },
            ".rodata",
        ),
        (
            RiscvElfSpec {
                data_shift: 0x100,
                ..RiscvElfSpec::default()
            },
            ".data",
        ),
    ] {
        assert_rejected(
            &spec.build(),
            &TranspilationMode::default(),
            |e| {
                matches!(
                    e,
                    TranspilerError::ElfBuilder(ElfBuilderError::GuestAddressMismatch { section, .. })
                        if section == section_name
                )
            },
            &format!("{section_name} shifted up"),
        );
    }
}

#[test]
fn unsupported_allocatable_section_is_rejected() {
    let elf = RiscvElfSpec {
        extra: vec![TestSection::progbits(
            ".tdata",
            elf::SHF_ALLOC | elf::SHF_WRITE | elf::SHF_TLS,
            0x10_0000,
            vec![0; 4],
            4,
        )],
        ..RiscvElfSpec::default()
    }
    .build();
    assert_rejected(
        &elf,
        &TranspilationMode::default(),
        |e| {
            matches!(
                e,
                TranspilerError::RiscvElfParse(RvElfParseError::UnsupportedSection { section, .. })
                    if section == ".tdata"
            )
        },
        ".tdata",
    );
}

/// A local symbol with a layout symbol's name, such as a C `static _stack_top_value`, is private
/// to its object file, so it is ignored instead of failing the read.
#[test]
fn local_symbol_named_like_a_layout_symbol_is_ignored() {
    let elf = RiscvElfSpec {
        extra_symbols: vec![TestSymbol {
            name: "_stack_top_value",
            in_text: None,
            value: 0x1234,
            binding: elf::STB_LOCAL,
        }],
        ..RiscvElfSpec::default()
    }
    .build();
    for result in transpile(&elf, &TranspilationMode::default()) {
        if let Err(e) = result {
            panic!("a local `_stack_top_value` should be ignored: {e}");
        }
    }
}

/// Data an object puts in `.stack`, `.heap` or `.ecall_data` lands in a second section of that
/// name, which the runtime would not load. The input is rejected instead.
#[test]
fn initialized_runtime_section_is_rejected() {
    for name in [".stack", ".heap", ".ecall_data"] {
        let elf = RiscvElfSpec {
            extra: vec![TestSection::progbits(
                name,
                elf::SHF_ALLOC | elf::SHF_WRITE,
                0x10_0000,
                vec![1, 2, 3, 4],
                4,
            )],
            ..RiscvElfSpec::default()
        }
        .build();
        assert_rejected(
            &elf,
            &TranspilationMode::default(),
            |e| {
                matches!(
                    e,
                    TranspilerError::RiscvElfParse(RvElfParseError::InitializedRuntimeSection { section })
                        if section == name
                )
            },
            name,
        );
    }
}

/// 16 KiB of code plus an `.init_array` push `.rodata` to the second segment boundary, 0x8000.
/// The output must place it there too, where the code reads it.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn init_array_pushing_rodata_to_the_next_segment_is_supported() {
    // `lui a0, 0x8; lw a0, 0(a0); ret`, then `nop`s up to 16 KiB.
    const READ_RODATA: [u32; 3] = [0x0000_8537, 0x0005_2503, 0x0000_8067];
    const NOP: u32 = 0x0000_0013;
    let mut code = READ_RODATA.to_vec();
    code.resize(4096, NOP);
    let elf = RiscvElfSpec {
        code,
        init_array: vec![0],
        ..RiscvElfSpec::default()
    }
    .build();
    let outputs = transpile(&elf, &TranspilationMode::default());
    for result in &outputs {
        if let Err(e) = result {
            panic!("input with an .init_array should transpile: {e}");
        }
    }

    let native = TranspilerForArchitecture::get_native().expect("an aarch64 or x86_64 host");
    let index = match native {
        TranspilerForArchitecture::Arm64 => 0,
        TranspilerForArchitecture::X86_64 => 1,
    };
    let output = outputs[index].as_ref().expect("transpiled above");
    let mut vm = TranspiledVm::load(
        &output.elf_bytes,
        LoaderContext::Untrusted,
        &output.entry_point_mapping,
    )
    .expect("output should load");
    let mut runner = TranspiledCodeRunner::new(&mut vm);
    let stack_top = runner.loaded_layout().stack_top();
    runner.set_sp(stack_top);
    runner.set_gas(UserCodeGasAmount::max());
    let main = *output
        .entry_point_mapping
        .get("main")
        .expect("main entry point");
    let mut regs = RegistersInOut::new_with_function_code(main);
    runner
        .run_till_ecall_or_return(&mut regs)
        .expect("the call should complete");
    assert_eq!(regs.get_user_code_exit_value(), NORMAL_EXIT_VALUE);
    assert_eq!(regs.get_normal_return_value(), 0x0403_0201);
}

/// An illegal-instruction exit saves the guest's registers unchanged, including `ra`, which an
/// aarch64 `BL` to the exit would overwrite with a host code address.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn illegal_instruction_exit_keeps_the_guest_return_address() {
    const ILLEGAL: u32 = 0xFFFF_FFFF;
    let native = TranspilerForArchitecture::get_native().expect("an aarch64 or x86_64 host");
    let elf = RiscvElfSpec {
        code: vec![ILLEGAL, 0x0000_8067],
        ..RiscvElfSpec::default()
    }
    .build();
    for mode in [TranspilationMode::default(), TranspilationMode::Trusted] {
        let output = native
            .transpile_to_native_elf(
                TranspilerInput {
                    riscv_elf_bytes: &elf,
                    entry_points: HashSet::from(["main".to_string()]),
                },
                &mode,
            )
            .expect("should transpile");
        let mut vm = TranspiledVm::load(
            &output.elf_bytes,
            LoaderContext::Trusted,
            &output.entry_point_mapping,
        )
        .expect("output should load");
        let mut runner = TranspiledCodeRunner::new(&mut vm);
        let stack_top = runner.loaded_layout().stack_top();
        runner.set_sp(stack_top);
        runner.set_gas(UserCodeGasAmount::max());
        let main = *output
            .entry_point_mapping
            .get("main")
            .expect("main entry point");
        let mut regs = RegistersInOut::new_with_function_code(main);
        runner
            .run_till_ecall_or_return(&mut regs)
            .expect("the call should complete");
        assert_eq!(
            regs.get_user_code_exit_value(),
            ILLEGAL_INSTRUCTION_EXIT_VALUE
        );
        // Each entry starts with `ra` at the end of the code: 8, after the two instructions.
        assert_eq!(runner.read_register(RvPreservedRegister::Ra), 8, "{mode:?}");
    }
}

/// Reaching the end of `.text` by a direct branch, or by running past a final `div` (whose
/// transpilation branches to the next instruction), is valid input on every target.
#[test]
fn branches_to_the_end_of_the_code_transpile() {
    const BEQ_ZERO_ZERO_PLUS_8: u32 = 0x0000_0463;
    const RET: u32 = 0x0000_8067;
    const DIV_A0_A0_A1: u32 = 0x02B5_4533;
    for code in [vec![BEQ_ZERO_ZERO_PLUS_8, RET], vec![RET, DIV_A0_A0_A1]] {
        let elf = RiscvElfSpec {
            code: code.clone(),
            ..RiscvElfSpec::default()
        }
        .build();
        for result in transpile(&elf, &TranspilationMode::default()) {
            if let Err(e) = result {
                panic!("{code:x?} should transpile: {e}");
            }
        }
    }
}

fn assert_read_error(spec: RiscvElfSpec, expected: impl Fn(&RvElfParseError) -> bool, what: &str) {
    assert_rejected(
        &spec.build(),
        &TranspilationMode::default(),
        |e| matches!(e, TranspilerError::RiscvElfParse(read_error) if expected(read_error)),
        what,
    );
}

#[test]
fn entry_point_must_be_a_unique_global_symbol() {
    assert_read_error(
        RiscvElfSpec {
            entry_binding: elf::STB_LOCAL,
            ..RiscvElfSpec::default()
        },
        |e| matches!(e, RvElfParseError::MissingEntryPoints { entry_points } if entry_points == &["main"]),
        "only a local `main`",
    );
    assert_read_error(
        RiscvElfSpec {
            duplicate_entry: true,
            ..RiscvElfSpec::default()
        },
        |e| matches!(e, RvElfParseError::DuplicateSymbol { name } if name == "main"),
        "two `main` symbols",
    );
}

#[test]
fn only_linked_rv32ima_executables_are_accepted() {
    assert_read_error(
        RiscvElfSpec {
            e_type: elf::ET_REL,
            ..RiscvElfSpec::default()
        },
        |e| matches!(e, RvElfParseError::NotAnExecutable { .. }),
        "relocatable object",
    );
    for (e_flags, what) in [
        (0x0001, "compressed instructions"),
        (0x0002, "single-float ABI"),
        (0x0004, "double-float ABI"),
        (0x0008, "RV32E"),
    ] {
        assert_read_error(
            RiscvElfSpec {
                e_flags,
                ..RiscvElfSpec::default()
            },
            |e| matches!(e, RvElfParseError::UnsupportedElfFlags { .. }),
            what,
        );
    }
}

#[test]
fn second_init_array_is_rejected_even_after_an_empty_one() {
    let init_array = |data: Vec<u8>| {
        let mut section =
            TestSection::progbits(".init_array", elf::SHF_ALLOC | elf::SHF_WRITE, 8, data, 4);
        section.sh_type = elf::SHT_INIT_ARRAY;
        section
    };
    assert_read_error(
        RiscvElfSpec {
            extra: vec![init_array(Vec::new()), init_array(vec![0; 4])],
            ..RiscvElfSpec::default()
        },
        |e| matches!(e, RvElfParseError::MultipleSection { section } if section == ".init_array"),
        "two `.init_array` sections",
    );
}

#[test]
fn inconsistent_layout_symbols_are_rejected() {
    for (overrides, what) in [
        (
            vec![("_data_end_value", 0x200_0000)],
            "data end past 16 MiB",
        ),
        (
            vec![("_data_start_value", 0x3_0000)],
            "data start above the stack top",
        ),
    ] {
        assert_read_error(
            RiscvElfSpec {
                layout_overrides: overrides,
                ..RiscvElfSpec::default()
            },
            |e| matches!(e, RvElfParseError::BadLayoutSymbols { .. }),
            what,
        );
    }
}

#[test]
fn code_too_large_to_ever_fit_is_rejected_up_front() {
    const NOP: u32 = 0x0000_0013;
    let mut code = CODE.to_vec();
    code.resize(0x20_0000 / 4 + 1, NOP);
    assert_read_error(
        RiscvElfSpec {
            code,
            ..RiscvElfSpec::default()
        },
        |e| matches!(e, RvElfParseError::CodeSectionTooLarge(_)),
        "2 MiB + 4 bytes of code",
    );
}

/// A NOBITS section that is not loaded (no `SHF_ALLOC`) is not part of guest memory, so it
/// must not be taken for `.bss`.
#[test]
fn non_allocatable_nobits_section_is_ignored() {
    let mut not_loaded = TestSection::nobits(".not_loaded", 0, 64, 4);
    not_loaded.sh_flags = 0;
    let elf = RiscvElfSpec {
        extra: vec![not_loaded],
        ..RiscvElfSpec::default()
    }
    .build();
    for result in transpile(&elf, &TranspilationMode::default()) {
        if let Err(e) = result {
            panic!("a non-allocatable NOBITS section should be ignored: {e}");
        }
    }
}

const ECALL: u32 = 0x0000_0073;
const RET: u32 = 0x0000_8067;

/// `li a7, value` (`addi a7, zero, value`), for a 12-bit `value`.
fn li_a7(value: i32) -> u32 {
    ((value as u32) << 20) | (17 << 7) | 0x13
}

/// Transpiles `spec` for this host in the default mode and loads it.
fn load_native(spec: RiscvElfSpec) -> (TranspiledVm, u32) {
    load_native_in(spec, &TranspilationMode::default())
}

/// Transpiles `spec` for this host in `mode` and loads it. Returns the VM and the index of
/// `main`.
fn load_native_in(spec: RiscvElfSpec, mode: &TranspilationMode) -> (TranspiledVm, u32) {
    let native = TranspilerForArchitecture::get_native()
        .expect("tests that run output need an aarch64 or x86_64 host");
    let output = native
        .transpile_to_native_elf(
            TranspilerInput {
                riscv_elf_bytes: &spec.build(),
                entry_points: HashSet::from(["main".to_string()]),
            },
            mode,
        )
        .expect("should transpile");
    let main = *output
        .entry_point_mapping
        .get("main")
        .expect("main entry point");
    let vm = TranspiledVm::load(
        &output.elf_bytes,
        LoaderContext::Trusted,
        &output.entry_point_mapping,
    )
    .expect("output should load");
    (vm, main)
}

fn ready_runner(vm: &mut TranspiledVm) -> TranspiledCodeRunner<'_> {
    let mut runner = TranspiledCodeRunner::new(vm);
    let stack_top = runner.loaded_layout().stack_top();
    runner.set_sp(stack_top);
    runner.set_gas(UserCodeGasAmount::max());
    runner
}

/// Serves every ecall and accepts every exit, recording the ecall numbers it saw.
#[derive(Default)]
struct AcceptEverything {
    ecalls: Vec<u32>,
    /// Ask to halt during each ecall.
    halt_on_ecall: bool,
    halted: bool,
}

impl<'a> EcallHandler<TranspiledCodeRunner<'a>> for AcceptEverything {
    fn handle_ecall(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        self.ecalls
            .push(inout_registers.get_function_or_ecall_code());
        self.halted = self.halt_on_ecall;
        Ok(())
    }
    fn handle_user_code_exited(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn handle_internal_user_code_panic(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn handle_internal_user_out_of_gas(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn handle_internal_user_illegal_opcode(
        &mut self,
        _memory_accessor: &mut TranspiledCodeRunner<'a>,
        _inout_registers: &mut RegistersInOut,
    ) -> Result<(), UserCodeRunnerError> {
        Ok(())
    }
    fn should_halt(&self) -> bool {
        self.halted
    }
    fn reset_should_halt(&mut self) {
        self.halted = false;
    }
}

/// An ecall whose number is an exit value (0, -1 or -2) takes the illegal-instruction exit, so
/// a guest cannot pose as a normal, panic or illegal-instruction exit. Other numbers are ecalls.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn ecall_numbers_that_are_exit_values_are_illegal() {
    for (number, expected_exit) in [
        (0, ILLEGAL_INSTRUCTION_EXIT_VALUE),
        (-1, ILLEGAL_INSTRUCTION_EXIT_VALUE),
        (-2, ILLEGAL_INSTRUCTION_EXIT_VALUE),
        (5, 5),
    ] {
        let (mut vm, main) = load_native(RiscvElfSpec {
            code: vec![li_a7(number), ECALL, RET],
            ..RiscvElfSpec::default()
        });
        let mut runner = ready_runner(&mut vm);
        let mut regs = RegistersInOut::new_with_function_code(main);
        runner
            .run_till_ecall_or_return(&mut regs)
            .expect("the call should complete");
        assert_eq!(
            regs.get_user_code_exit_value(),
            expected_exit,
            "ecall number {number}"
        );
    }
}

#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn init_functions_have_their_ecalls_served() {
    let (mut vm, _) = load_native(RiscvElfSpec {
        code: vec![li_a7(7), ECALL, RET],
        init_array: vec![0],
        ..RiscvElfSpec::default()
    });
    let mut runner = ready_runner(&mut vm);
    let mut handler = AcceptEverything::default();
    run_init_array(&mut runner, &mut handler).expect("the init function should run to completion");
    assert_eq!(handler.ecalls, [7]);
}

/// On entry the guest sees a7 = the selector the host passed, on every target.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn entry_sees_the_selector_in_a7() {
    const MV_A0_A7: u32 = 0x0008_8513;
    for mode in [TranspilationMode::default(), TranspilationMode::Trusted] {
        let (mut vm, main) = load_native_in(
            RiscvElfSpec {
                code: vec![MV_A0_A7, RET],
                ..RiscvElfSpec::default()
            },
            &mode,
        );
        let mut runner = ready_runner(&mut vm);
        let mut regs = RegistersInOut::new_with_function_code(main);
        let outcome = call_entry_point(&mut runner, &mut AcceptEverything::default(), &mut regs);
        assert!(
            matches!(outcome, Ok(RunOutcome::Returned)),
            "{mode:?}: got {outcome:?}"
        );
        assert_eq!(regs.get_normal_return_value(), main as i32, "{mode:?}");
    }
}

/// The RISC-V ABI keeps a7 across an ecall, and compiled code relies on it: a loop that
/// repeats an ecall sets a7 once. Each resume must give the guest back its ecall number.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn ecall_keeps_a7() {
    const MV_A0_A7: u32 = 0x0008_8513;
    for mode in [TranspilationMode::default(), TranspilationMode::Trusted] {
        let (mut vm, main) = load_native_in(
            RiscvElfSpec {
                code: vec![li_a7(7), ECALL, ECALL, MV_A0_A7, RET],
                ..RiscvElfSpec::default()
            },
            &mode,
        );
        let mut runner = ready_runner(&mut vm);
        let mut handler = AcceptEverything::default();
        let mut regs = RegistersInOut::new_with_function_code(main);
        let outcome = call_entry_point(&mut runner, &mut handler, &mut regs);
        assert!(
            matches!(outcome, Ok(RunOutcome::Returned)),
            "{mode:?}: got {outcome:?}"
        );
        assert_eq!(handler.ecalls, [7, 7], "{mode:?}");
        assert_eq!(regs.get_normal_return_value(), 7, "{mode:?}");
    }
}

/// An init function the handler halts during an ecall has not finished, so it fails
/// `run_init_array`. After the ecall a7 holds the resume code, 0, which must not be mistaken
/// for a normal return.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn init_function_halted_in_an_ecall_is_reported() {
    let (mut vm, _) = load_native(RiscvElfSpec {
        code: vec![li_a7(7), ECALL, RET],
        init_array: vec![0],
        ..RiscvElfSpec::default()
    });
    let mut runner = ready_runner(&mut vm);
    let mut handler = AcceptEverything {
        halt_on_ecall: true,
        ..AcceptEverything::default()
    };
    let result = run_init_array(&mut runner, &mut handler);
    assert!(
        matches!(
            result,
            Err(UserCodeRunnerError::InitFunctionFailed {
                function_index: 1,
                outcome: RunOutcome::HaltedInEcall,
            })
        ),
        "got {result:?}"
    );
}

/// A call halted in an ecall resumes with the same registers, and the halt flag it left set
/// does not end the next call.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn halted_call_resumes_and_does_not_halt_the_next_one() {
    let (mut vm, main) = load_native(RiscvElfSpec {
        code: vec![li_a7(7), ECALL, RET],
        ..RiscvElfSpec::default()
    });
    let mut runner = ready_runner(&mut vm);
    let mut handler = AcceptEverything {
        halt_on_ecall: true,
        ..AcceptEverything::default()
    };
    let mut regs = RegistersInOut::new_with_function_code(main);
    let halted = call_entry_point(&mut runner, &mut handler, &mut regs);
    assert!(
        matches!(halted, Ok(RunOutcome::HaltedInEcall)),
        "got {halted:?}"
    );
    assert!(handler.should_halt());

    // Resuming finishes the call; the ecall is not served again.
    handler.halt_on_ecall = false;
    let resumed = call_entry_point(&mut runner, &mut handler, &mut regs);
    assert!(
        matches!(resumed, Ok(RunOutcome::Returned)),
        "got {resumed:?}"
    );
    assert_eq!(handler.ecalls, [7]);

    // A new call with a stale halt flag runs, and is served, to the end.
    handler.halted = true;
    let mut next_regs = RegistersInOut::new_with_function_code(main);
    let next = call_entry_point(&mut runner, &mut handler, &mut next_regs);
    assert!(matches!(next, Ok(RunOutcome::Returned)), "got {next:?}");
    assert_eq!(handler.ecalls, [7, 7]);
}

/// Each init function starts with the stack pointer the host set, even if the one before it
/// returned without restoring it.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn each_init_function_starts_with_the_hosts_stack_pointer() {
    const ADDI_SP_SP_MINUS_16: u32 = 0xFF01_0113;
    let (mut vm, _) = load_native(RiscvElfSpec {
        code: vec![ADDI_SP_SP_MINUS_16, RET],
        init_array: vec![0, 0],
        ..RiscvElfSpec::default()
    });
    let mut runner = ready_runner(&mut vm);
    let stack_top = runner.get_sp().as_u32();
    run_init_array(&mut runner, &mut AcceptEverything::default())
        .expect("the init functions should return");
    // Only the last init function's shift remains.
    assert_eq!(runner.get_sp().as_u32(), stack_top - 16);
}

/// An init function that does not return normally fails `run_init_array`, even when the
/// handler accepts the exit: the program's globals may be half-initialized.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn failing_init_function_is_reported() {
    const ILLEGAL: u32 = 0xFFFF_FFFF;
    let (mut vm, _) = load_native(RiscvElfSpec {
        code: vec![ILLEGAL, RET],
        init_array: vec![0],
        ..RiscvElfSpec::default()
    });
    let mut runner = ready_runner(&mut vm);
    let result = run_init_array(&mut runner, &mut AcceptEverything::default());
    assert!(
        matches!(
            result,
            Err(UserCodeRunnerError::InitFunctionFailed {
                function_index: 1,
                outcome: RunOutcome::IllegalInstruction,
            })
        ),
        "got {result:?}"
    );
}

/// After an exit, the return-address slot holds `NO_ECALL_RETURN_ADDRESS` whatever was in the
/// register that carries it. Here that register last held 4: on aarch64, writing `t3` goes
/// through x28. A resume must still take the panic exit, not continue at instruction 1.
#[cfg_attr(
    not(any(
        target_arch = "aarch64",
        all(target_arch = "x86_64", target_os = "linux")
    )),
    ignore = "needs an aarch64 host, or x86_64 Linux"
)]
#[test]
fn resume_after_an_exit_does_not_reuse_a_scratch_address() {
    const ADDI_T3_ZERO_4: u32 = 0x0040_0E13;
    const ILLEGAL: u32 = 0xFFFF_FFFF;
    let (mut vm, main) = load_native(RiscvElfSpec {
        code: vec![ADDI_T3_ZERO_4, ILLEGAL, RET],
        ..RiscvElfSpec::default()
    });
    let mut runner = ready_runner(&mut vm);
    let mut call = RegistersInOut::new_with_function_code(main);
    runner
        .run_till_ecall_or_return(&mut call)
        .expect("the call should complete");
    assert_eq!(
        call.get_user_code_exit_value(),
        ILLEGAL_INSTRUCTION_EXIT_VALUE
    );
    let mut resume = RegistersInOut::new_return_from_ecall();
    runner
        .run_till_ecall_or_return(&mut resume)
        .expect("the resume should complete");
    assert_eq!(resume.get_user_code_exit_value(), PANIC_EXIT_VALUE);
}
