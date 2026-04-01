//! Verification error migration test apparatus.
//!
//! These tests verify the behaviour of the deploy pipeline when SBPF ELF
//! verification is enabled vs. disabled (`skip_verification`).
//!
//! For each [`VerifierError`] variant we construct a minimal ELF that triggers
//! the violation, then assert:
//!
//!   1. **Verification ON** (`skip_verification = false`):
//!      `ProgramCacheEntry::new()` must fail.
//!
//!   2. **Verification OFF** (`skip_verification = true`):
//!      `Executable::load()` must succeed — the invalid bytecode can be loaded
//!      without verification. (We test at the `Executable` level to avoid JIT
//!      compilation panics on intentionally malformed bytecode.)
//!
//!   3. **Invoke** (via interpreter, no JIT): assert the expected runtime
//!      outcome — either a specific `EbpfError` variant for hardened checks, or
//!      `Ok(())` for accepted behaviours where the program runs with
//!      corrupted/wrong internal state but cannot escape the VM sandbox.
//!
//! The results are optionally ejected to a markdown table when the
//! `EJECT_ERROR_CODES` environment variable is set.

use {
    sbpf_test_utils::{
        TestContextObject, corpus,
        elf_builder::{elf_from_text_bytes, elf_with_assembly},
    },
    solana_program_runtime::loaded_programs::{
        ProgramCacheEntry, ProgramRuntimeEnvironment, get_mock_program_runtime_environment,
    },
    solana_sbpf::{
        elf::Executable,
        program::{BuiltinProgram, SBPFVersion},
        verifier::RequisiteVerifier,
        vm::{Config, EbpfVm},
    },
    solana_sdk_ids::bpf_loader_upgradeable,
    std::sync::Arc,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Outcome of a single error-migration probe.
struct ErrorMigration {
    /// The error string from deploy with verification ON.
    deploy_error: String,
    /// Whether loading without verification succeeds.
    load_without_verify_ok: bool,
    /// Error from invoking the program after loading without verification.
    /// `None` if the program could not be loaded, `Some(Ok(()))` if invoke
    /// succeeded, `Some(Err(msg))` if invoke failed.
    invoke_error: Option<Result<(), String>>,
}

/// Build a `ProgramRuntimeEnvironment` that accepts V3 ELFs.
fn make_v3_environment() -> ProgramRuntimeEnvironment {
    get_mock_program_runtime_environment()
}

/// Build an `Arc<BuiltinProgram>` for loading V3 ELFs directly.
fn make_v3_loader() -> Arc<BuiltinProgram<TestContextObject>> {
    let config = Config {
        enabled_sbpf_versions: SBPFVersion::V3..=SBPFVersion::V3,
        ..Config::default()
    };
    Arc::new(BuiltinProgram::new_loader(config))
}

/// Build an `Arc<BuiltinProgram>` for loading V3 ELFs with
/// `stricter_loader_checks` enabled.
///
/// This enables additional O(1) checks in `Executable::load()` that reject
/// structurally invalid ELFs (empty text section, text length not a multiple
/// of `INSN_SIZE`) even when bytecode verification is disabled.
fn make_v3_loader_strict() -> Arc<BuiltinProgram<TestContextObject>> {
    let config = Config {
        enabled_sbpf_versions: SBPFVersion::V3..=SBPFVersion::V3,
        stricter_loader_checks: true,
        ..Config::default()
    };
    Arc::new(BuiltinProgram::new_loader(config))
}

/// Attempt to deploy ELF bytes through `ProgramCacheEntry::new()` with
/// verification enabled.
fn deploy_with_verification(elf: &[u8]) -> Result<ProgramCacheEntry, String> {
    let env = make_v3_environment();
    ProgramCacheEntry::new(
        &bpf_loader_upgradeable::id(),
        env,
        0, // deployment_slot
        1, // effective_slot
        elf,
        elf.len(),
        false, // skip_verification = false
    )
    .map_err(|e| format!("{e}"))
}

/// Attempt to load ELF bytes and skip verification (no JIT).
/// This mirrors what `deploy_program()` does when `skip_verification = true`,
/// except we avoid JIT compilation to prevent panics on intentionally malformed
/// bytecode.
fn load_without_verification(elf: &[u8]) -> Result<Executable<TestContextObject>, String> {
    let loader = make_v3_loader();
    Executable::<TestContextObject>::load(elf, loader).map_err(|e| format!("{e}"))
}

/// Execute a loaded executable through the interpreter (no JIT).
/// Returns `Ok(())` if the program exits successfully, `Err(msg)` if
/// the VM returns an error, or `Err("PANIC: ...")` if execution panics
/// (indicating an unhardened code path).
fn invoke_via_interpreter(executable: &Executable<TestContextObject>) -> Result<(), String> {
    let sbpf_version = executable.get_sbpf_version();
    let config = executable.get_config();

    let mut stack =
        solana_sbpf::aligned_memory::AlignedMemory::zero_filled(config.stack_size());
    let mut heap = solana_sbpf::aligned_memory::AlignedMemory::with_capacity(0);
    let stack_len = stack.len();
    let mut context_object = TestContextObject::new(1_000_000);

    let memory_mapping = sbpf_test_utils::create_memory_mapping(
        executable,
        &mut stack,
        &mut heap,
        vec![],
        None,
    )
    .unwrap();

    let mut vm = EbpfVm::new(
        executable.get_loader().clone(),
        sbpf_version,
        &mut context_object,
        memory_mapping,
        stack_len,
    );
    vm.registers[1] = solana_sbpf::ebpf::MM_INPUT_START;

    // Catch panics from unhardened code paths (e.g. OOB register access).
    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vm.execute_program(
            executable,
            &mut solana_sbpf::vm::ExecutionMode::Interpreted,
        )
    }));

    match panic_result {
        Ok((_instruction_count, result)) => match result {
            solana_sbpf::error::ProgramResult::Ok(_) => Ok(()),
            solana_sbpf::error::ProgramResult::Err(e) => Err(format!("{e:?}")),
        },
        Err(panic) => {
            let msg = panic
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            Err(format!("PANIC: {msg}"))
        }
    }
}

/// Run the full error-migration check for a given ELF.
fn check_error_migration(elf: &[u8]) -> ErrorMigration {
    // With verification: deploy must fail
    let deploy_err =
        deploy_with_verification(elf).expect_err("deploy should fail with verification ON");

    // Without verification: load must succeed
    let load_result = load_without_verification(elf);
    let load_without_verify_ok = load_result.is_ok();
    let invoke_error = if let Ok(ref executable) = load_result {
        // Confirm it actually fails verification
        let verify_result = executable.verify::<RequisiteVerifier>();
        assert!(verify_result.is_err(), "expected verification to fail");
        // Invoke via interpreter
        Some(invoke_via_interpreter(executable))
    } else {
        None
    };

    ErrorMigration {
        deploy_error: deploy_err,
        load_without_verify_ok,
        invoke_error,
    }
}

/// Optionally append a valid-program row to the markdown table.
fn record_valid_result(name: &str) {
    if std::env::var("EJECT_ERROR_CODES").is_ok() {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("error_migration_table.md")
            .unwrap();
        writeln!(file, "| {name} | OK | OK | OK |").unwrap();
    }
}

/// Optionally append a row to the markdown error migration table.
fn record_result(name: &str, migration: &ErrorMigration) {
    if std::env::var("EJECT_ERROR_CODES").is_ok() {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("error_migration_table.md")
            .unwrap();
        let invoke_col = match &migration.invoke_error {
            Some(Ok(())) => "OK".to_owned(),
            Some(Err(e)) => e.clone(),
            None => "N/A".to_owned(),
        };
        writeln!(
            file,
            "| {name} | {deploy_err} | {skip_ok} | {invoke_col} |",
            deploy_err = migration.deploy_error,
            skip_ok = if migration.load_without_verify_ok {
                "OK"
            } else {
                "FAIL"
            },
        )
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// Assembly-based tests (assembler can emit the invalid bytecode)
// ---------------------------------------------------------------------------

/// Immediate division by zero (`div32 r0, 0`).
///
/// **Verifier check:** `DivisionByZero` — the verifier rejects any `div` or
/// `mod` instruction with `imm == 0` because bare Rust `/` and `%` operators
/// panic on integer division by zero.
///
/// **Verification ON:** Deploy fails with `"division by 0"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// reaches the `div32 r0, 0` instruction and returns
/// `EbpfError::DivideByZero`. This is a hardened code path — commit `eb5fd71`
/// added explicit `if insn.imm == 0` checks before each immediate-divisor
/// opcode in the interpreter and JIT. Without that fix, the interpreter
/// panicked and the JIT triggered an x86 `#DE` hardware exception.
#[test]
fn division_by_zero() {
    let elf = elf_with_assembly::<TestContextObject>(corpus::DIVISION_BY_ZERO, SBPFVersion::V3);
    let m = check_error_migration(&elf);
    record_result("division_by_zero", &m);
    assert!(
        m.deploy_error.contains("division by 0"),
        "expected 'division by 0', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Hardened: interpreter returns DivideByZero at execution time.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("DivideByZero"),
        "expected 'DivideByZero', got: {invoke_err}",
    );
}

/// Forward jump past end of program (`ja +100`).
///
/// **Verifier check:** `JumpOutOfCode` — the verifier rejects jumps whose
/// target falls outside `0..prog.len() / INSN_SIZE`.
///
/// **Verification ON:** Deploy fails with `"jump out of code"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// executes the jump, setting PC to a value past the end of the program. On
/// the next iteration the top-of-loop bounds check
/// (`is_pc_in_program(program, pc)`) catches the OOB PC and returns
/// `EbpfError::ExecutionOverrun`. This is inherently safe in the interpreter
/// because the bounds check fires before any memory access at the invalid PC.
#[test]
fn jump_out_of_code() {
    let elf = elf_with_assembly::<TestContextObject>(corpus::JUMP_OUT_OF_CODE, SBPFVersion::V3);
    let m = check_error_migration(&elf);
    record_result("jump_out_of_code", &m);
    assert!(
        m.deploy_error.contains("jump out of code"),
        "expected 'jump out of code', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Hardened: interpreter catches OOB PC on the next iteration.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("ExecutionOverrun"),
        "expected 'ExecutionOverrun', got: {invoke_err}",
    );
}

/// Write to r10 (`mov r10, 1`).
///
/// **Verifier check:** `CannotWriteR10` — the verifier rejects any
/// instruction that writes to register 10 (the frame pointer), except for
/// `add64 r10, imm` in versions with `manual_stack_frame_bump`.
///
/// **Verification ON:** Deploy fails with `"cannot write into register r10"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// executes `mov r10, 1`, overwriting the frame pointer with the value 1.
/// This corrupts the program's own stack frame — subsequent stack accesses
/// will use the wrong base address. However, this is **self-harm only**: the
/// stack lives entirely within the program's own memory region and cannot
/// escape the VM sandbox. The program continues execution (with a corrupted
/// frame pointer) and exits normally.
///
/// We accept this because the verifier's r10 write prohibition was
/// conservative — without verification the program is allowed to be
/// self-destructive within its own sandbox.
#[test]
fn cannot_write_r10() {
    let elf = elf_with_assembly::<TestContextObject>(corpus::CANNOT_WRITE_R10, SBPFVersion::V3);
    let m = check_error_migration(&elf);
    record_result("cannot_write_r10", &m);
    assert!(
        m.deploy_error.contains("cannot write into register r10"),
        "expected 'cannot write into register r10', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Accepted behavior: the program runs with a corrupted frame pointer.
    // Writing to r10 is self-harm — it cannot escape the sandbox. The program
    // loads `1` into the frame pointer then immediately exits.
    assert!(
        m.invoke_error.as_ref().unwrap().is_ok(),
        "expected invoke to succeed (corrupted frame pointer is accepted), got: {:?}",
        m.invoke_error,
    );
}

/// Destination register > 10 (`mov r11, 1`).
///
/// **Verifier check:** `InvalidDestinationRegister` — the verifier rejects
/// instructions with `dst > 10` because the register file has only 11
/// general-purpose entries (r0..r10).
///
/// **Verification ON:** Deploy fails with `"invalid destination register"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// executes `mov r11, 1`. The register file has 12 entries (r0..r10 plus
/// r11 = PC), so the bounds check (`dst >= self.reg.len()` where len=12)
/// passes for dst=11. Writing to r11 corrupts the program counter — the
/// interpreter sets PC to 1, which happens to point at the `exit`
/// instruction (the second instruction in this 2-instruction program). The
/// program then exits normally.
///
/// This is an **accepted behavior**: writing to r11 (the PC register) is
/// semantically wrong but deterministic and self-contained. The bounds
/// check only prevents OOB panics for indices >= 12. Rejecting writes to
/// r11 specifically would require a feature-gated semantic check.
#[test]
fn invalid_destination_register() {
    let elf = elf_with_assembly::<TestContextObject>(
        corpus::INVALID_DESTINATION_REGISTER,
        SBPFVersion::V3,
    );
    let m = check_error_migration(&elf);
    record_result("invalid_destination_register", &m);
    assert!(
        m.deploy_error.contains("invalid destination register"),
        "expected 'invalid destination register', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Accepted behavior: dst=11 is within the physical register file (len=12)
    // so the bounds check passes. Writing to r11 corrupts the PC to 1, which
    // points at the exit instruction. The program exits normally.
    assert!(
        m.invoke_error.as_ref().unwrap().is_ok(),
        "expected invoke to succeed (r11 write is accepted), got: {:?}",
        m.invoke_error,
    );
}

/// 32-bit shift by 32 (`lsh32 r0, 32`).
///
/// **Verifier check:** `ShiftWithOverflow` — the verifier rejects shifts
/// where the immediate equals or exceeds the operand width (32 for 32-bit
/// ops, 64 for 64-bit ops). The concern is that `1u32 << 32` is undefined
/// behaviour in C and panics in Rust debug mode.
///
/// **Verification ON:** Deploy fails with `"Shift with overflow"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// executes `lsh32 r0, 32` using Rust's `wrapping_shl`, which masks the
/// shift amount by 31 (i.e., `32 & 31 = 0`). The result differs from
/// erroring — `r0` is unchanged rather than zeroed — but it is well-defined
/// and deterministic across all validator platforms because x86 hardware
/// masks 32-bit shifts by 31 and Rust's `wrapping_shl` follows the same
/// behaviour.
///
/// We accept this because the result is deterministic and confined to the
/// program's own register state.
#[test]
fn shift_with_overflow_32() {
    let elf =
        elf_with_assembly::<TestContextObject>(corpus::SHIFT_WITH_OVERFLOW_32, SBPFVersion::V3);
    let m = check_error_migration(&elf);
    record_result("shift_with_overflow_32", &m);
    assert!(
        m.deploy_error.contains("Shift with overflow"),
        "expected 'Shift with overflow', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Accepted behavior: x86 masks 32-bit shifts by 31, so `lsh32 r0, 32`
    // becomes `lsh32 r0, 0` (a no-op). The result is deterministic and the
    // program exits normally.
    assert!(
        m.invoke_error.as_ref().unwrap().is_ok(),
        "expected invoke to succeed (masked shift is accepted), got: {:?}",
        m.invoke_error,
    );
}

/// 64-bit shift by 64 (`lsh64 r0, 64`).
///
/// **Verifier check:** `ShiftWithOverflow` — same as the 32-bit case but for
/// 64-bit operand width. The verifier rejects `imm >= 64` for 64-bit shift
/// instructions.
///
/// **Verification ON:** Deploy fails with `"Shift with overflow"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// executes `lsh64 r0, 64` using Rust's `wrapping_shl`, which masks the
/// shift amount by 63 (i.e., `64 & 63 = 0`). Same rationale as the 32-bit
/// case — deterministic, matches x86 hardware semantics.
///
/// We accept this for the same reason as `shift_with_overflow_32`.
#[test]
fn shift_with_overflow_64() {
    let elf =
        elf_with_assembly::<TestContextObject>(corpus::SHIFT_WITH_OVERFLOW_64, SBPFVersion::V3);
    let m = check_error_migration(&elf);
    record_result("shift_with_overflow_64", &m);
    assert!(
        m.deploy_error.contains("Shift with overflow"),
        "expected 'Shift with overflow', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Accepted behavior: x86 masks 64-bit shifts by 63, so `lsh64 r0, 64`
    // becomes `lsh64 r0, 0` (a no-op). Deterministic, exits normally.
    assert!(
        m.invoke_error.as_ref().unwrap().is_ok(),
        "expected invoke to succeed (masked shift is accepted), got: {:?}",
        m.invoke_error,
    );
}

/// callx with r10 (`callx r10`).
///
/// **Verifier check:** `InvalidRegister` — the verifier rejects `callx`
/// with an out-of-range register. In V3, `callx r10` is invalid because r10
/// is the frame pointer and not a valid call target.
///
/// **Verification ON:** Deploy fails with `"Invalid register"`.
///
/// **Verification OFF:** Deploy succeeds. In V3, `callx` uses the `dst`
/// register field (not `imm` as in V0/V1). For `callx r10`, dst=10 passes
/// the register bounds check (10 < 12). The interpreter reads `self.reg[10]`
/// (the frame pointer, a stack address) and attempts to jump to that address.
/// Since the stack address is not within the text segment, the interpreter
/// returns `EbpfError::CallOutsideTextSegment`.
///
/// Note: In V0/V1, `callx` uses `insn.imm` for the register index and the
/// hardening check (`imm < 0 || imm >= reg.len()`) returns
/// `InvalidInstruction` for out-of-range values. In V3, the register
/// index is in the `dst` field (already bounds-checked at the top of the
/// step loop), so the error path differs — the call proceeds but the
/// target address is invalid.
#[test]
fn invalid_register_callx() {
    let elf =
        elf_with_assembly::<TestContextObject>(corpus::INVALID_REGISTER_CALLX, SBPFVersion::V3);
    let m = check_error_migration(&elf);
    record_result("invalid_register_callx", &m);
    assert!(
        m.deploy_error.contains("Invalid register"),
        "expected 'Invalid register', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // In V3, callx uses the dst register field. r10 is the frame pointer
    // (a stack address), which is not a valid text segment address.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("CallOutsideTextSegment"),
        "expected 'CallOutsideTextSegment', got: {invoke_err}",
    );
}

// ---------------------------------------------------------------------------
// Raw-byte tests (assembler cannot emit these violations)
// ---------------------------------------------------------------------------

/// BE instruction with unsupported immediate (`be r1, 3`).
///
/// **Verifier check:** `UnsupportedLEBEArgument` — the verifier rejects
/// LE/BE (byte swap) instructions where the immediate is not 16, 32, or 64.
///
/// **Verification ON:** Deploy fails with `"unsupported argument for LE/BE"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter hits
/// the default match arm for the LE/BE immediate width and returns
/// `EbpfError::InvalidInstruction`. This was already safe before any
/// hardening work — both the JIT (compile-time error) and interpreter
/// (dispatch default arm) handled unknown widths.
#[test]
fn unsupported_lebe_argument() {
    let elf = elf_from_text_bytes(corpus::raw::UNSUPPORTED_LEBE_ARG);
    let m = check_error_migration(&elf);
    record_result("unsupported_lebe_argument", &m);
    assert!(
        m.deploy_error.contains("unsupported argument for LE/BE"),
        "expected 'unsupported argument for LE/BE', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Already safe: the interpreter's default match arm rejects unknown widths.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("InvalidInstruction"),
        "expected 'InvalidInstruction', got: {invoke_err}",
    );
}

/// LDDW as the last (and only) instruction — no room for the second slot.
///
/// **Verifier check:** `LDDWCannotBeLast` — the verifier rejects LDDW when
/// there is no room for the mandatory second 8-byte slot.
///
/// **Verification ON:** Deploy fails with `"LD_DW instruction cannot be
/// last"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// checks `(pc + 1) * INSN_SIZE >= program.len()` before calling
/// `augment_lddw_unchecked` and returns `EbpfError::ExecutionOverrun` (or
/// `InvalidInstruction` depending on the exact bounds-check path). This is a
/// hardened code path from commit `6097dd8` — without it,
/// `augment_lddw_unchecked` read past the end of the program and panicked.
#[test]
fn lddw_cannot_be_last() {
    let elf = elf_from_text_bytes(corpus::raw::LDDW_CANNOT_BE_LAST);
    let m = check_error_migration(&elf);
    record_result("lddw_cannot_be_last", &m);
    assert!(
        m.deploy_error.contains("LD_DW instruction cannot be last"),
        "expected 'LD_DW instruction cannot be last', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Hardened: bounds check before augment_lddw_unchecked fires. The
    // interpreter returns ExecutionOverrun or InvalidInstruction depending on
    // the exact check path.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("ExecutionOverrun") || invoke_err.contains("InvalidInstruction"),
        "expected 'ExecutionOverrun' or 'InvalidInstruction', got: {invoke_err}",
    );
}

/// LDDW followed by a non-zero opcode in the second slot.
///
/// **Verifier check:** `IncompleteLDDW` — the verifier rejects LDDW when
/// the second 8-byte slot has a non-zero opcode, because
/// `augment_lddw_unchecked` blindly merges both slots' `imm` fields into a
/// single 64-bit immediate without checking the second slot's opcode.
///
/// **Verification ON:** Deploy fails with `"incomplete LD_DW"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation `augment_lddw_unchecked`
/// merges the two slots' immediates — but the second slot's `imm` field
/// contains whatever value was placed there (in our corpus: zero, since the
/// second slot is `0x85 0x00 ... 0x00`). The resulting 64-bit immediate is
/// `0x0000_0000_5566_7788` — identical to what a correct LDDW with `imm_hi=0`
/// would produce. In general the second slot could contain any bytes,
/// producing a garbage immediate value. This only corrupts the program's own
/// register — it cannot escape the VM sandbox.
///
/// We accept this because checking would require walking the instruction
/// stream at load time (O(n) cost for a case that only hurts the program
/// itself).
///
/// Note: We construct custom bytes here (LDDW + exit) rather than using the
/// corpus entry directly, because the corpus `INCOMPLETE_LDDW` has no exit
/// instruction and would hit `ExecutionOverrun` instead of demonstrating the
/// accepted-behaviour path.
#[test]
fn incomplete_lddw() {
    // Custom bytes: LDDW with non-zero opcode in second slot, followed by exit.
    // This triggers IncompleteLDDW at verification but runs successfully
    // without verification — the LDDW gets a (potentially wrong) immediate
    // from the second slot's imm field, then the program exits.
    let custom_bytes: &[u8] = &[
        // lddw r0, 0x55667788 (first slot)
        0x18, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55,
        // second slot with non-zero opcode (0x85 = call) — triggers IncompleteLDDW
        0x85, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        // exit
        0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let elf = elf_from_text_bytes(custom_bytes);
    let m = check_error_migration(&elf);
    record_result("incomplete_lddw", &m);
    assert!(
        m.deploy_error.contains("incomplete LD_DW"),
        "expected 'incomplete LD_DW', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Accepted behavior: augment_lddw_unchecked merges the two slots' imm
    // fields blindly. The second slot's imm happens to be 0 in our test bytes,
    // so r0 gets 0x0000_0000_5566_7788. In general the immediate could be
    // garbage. Either way, the corruption is confined to the program's own
    // register — it cannot escape the sandbox.
    assert!(
        m.invoke_error.as_ref().unwrap().is_ok(),
        "expected invoke to succeed (garbage immediate is accepted), got: {:?}",
        m.invoke_error,
    );
}

/// Source register > 10 (`mov64 r0, r12`).
///
/// **Verifier check:** `InvalidSourceRegister` — the verifier rejects
/// instructions with `src > 10` for the same reason as
/// `InvalidDestinationRegister`: the register file has only 11 entries and
/// OOB indexing panics.
///
/// **Verification ON:** Deploy fails with `"invalid source register"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// hits the bounds check added in commit `6097dd8` —
/// `src >= self.reg.len()` — and returns `EbpfError::InvalidInstruction`.
/// This is a hardened code path; without the fix both interpreter and JIT
/// panicked.
#[test]
fn invalid_source_register() {
    let elf = elf_from_text_bytes(corpus::raw::INVALID_SOURCE_REGISTER);
    let m = check_error_migration(&elf);
    record_result("invalid_source_register", &m);
    assert!(
        m.deploy_error.contains("invalid source register"),
        "expected 'invalid source register', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Hardened: bounds check on register index fires.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("InvalidInstruction"),
        "expected 'InvalidInstruction', got: {invoke_err}",
    );
}

/// Unknown opcode (`0x06`).
///
/// **Verifier check:** `UnknownOpCode` — the verifier rejects instructions
/// with opcodes not in the SBPF instruction set.
///
/// **Verification ON:** Deploy fails with `"unknown eBPF opcode"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the interpreter
/// hits the default match arm in the instruction dispatch loop and returns
/// `EbpfError::UnsupportedInstruction`. This was already safe before any
/// hardening work — both the JIT (compile-time error on unknown opcode)
/// and the interpreter (default match arm) handled this case.
#[test]
fn unknown_opcode() {
    let elf = elf_from_text_bytes(corpus::raw::UNKNOWN_OPCODE);
    let m = check_error_migration(&elf);
    record_result("unknown_opcode", &m);
    assert!(
        m.deploy_error.contains("unknown eBPF opcode"),
        "expected 'unknown eBPF opcode', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Already safe: the interpreter's default match arm catches unknown opcodes.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("UnsupportedInstruction"),
        "expected 'UnsupportedInstruction', got: {invoke_err}",
    );
}

// ---------------------------------------------------------------------------
// Jump to middle of LDDW (assembly-based, LDDW available in V3)
// ---------------------------------------------------------------------------

/// Jump landing on the second slot of an LDDW (`ja +1` over a `lddw`).
///
/// **Verifier check:** `JumpToMiddleOfLDDW` — the verifier rejects jumps
/// whose target PC lands on the second slot of a two-slot LDDW instruction.
///
/// **Verification ON:** Deploy fails with `"jump to middle of LD_DW"`.
///
/// **Verification OFF:** Deploy succeeds. At invocation the jump transfers
/// control to the second LDDW slot, which has opcode 0x00. Opcode 0x00 is
/// not a valid SBPF instruction, so both the JIT (compile-time →
/// `ANCHOR_CALL_UNSUPPORTED_INSTRUCTION`) and interpreter (default match
/// arm) catch it as `EbpfError::UnsupportedInstruction`.
///
/// This is **already safe without any hardening** — the zero opcode
/// naturally falls into the error path. No new code was needed for this
/// case.
#[test]
fn jump_to_middle_of_lddw() {
    let elf =
        elf_with_assembly::<TestContextObject>(corpus::JUMP_TO_MIDDLE_OF_LDDW, SBPFVersion::V3);
    let m = check_error_migration(&elf);
    record_result("jump_to_middle_of_lddw", &m);
    assert!(
        m.deploy_error.contains("jump to middle of LD_DW"),
        "expected 'jump to middle of LD_DW', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
    // Already safe: the second LDDW slot has opc=0x00, which is caught as
    // UnsupportedInstruction by the interpreter's default match arm.
    let invoke_err = m.invoke_error.as_ref().unwrap().as_ref().unwrap_err();
    assert!(
        invoke_err.contains("UnsupportedInstruction"),
        "expected 'UnsupportedInstruction', got: {invoke_err}",
    );
}

// ---------------------------------------------------------------------------
// Loader-level checks (stricter_loader_checks)
// ---------------------------------------------------------------------------

/// Text section length not a multiple of `INSN_SIZE` (8 bytes).
///
/// **Verifier check:** `ProgramLengthNotMultiple` — the verifier rejects
/// programs whose text section length is not a multiple of 8 bytes (one
/// instruction slot).
///
/// For V3 ELFs, the strict ELF parser validates `p_filesz % INSN_SIZE == 0`
/// on the program header itself, rejecting non-aligned text sections at the
/// ELF structural level with `InvalidProgramHeader`. This happens before
/// `stricter_loader_checks` or verification ever run.
///
/// The `stricter_loader_checks` config flag adds the same check for
/// V0/V1/V2 loaders where the lenient parser does not validate text
/// alignment. Both layers provide the same security property: programs
/// with non-aligned text cannot be loaded, regardless of whether
/// verification is enabled.
///
/// This test verifies that a V3 ELF with non-aligned text is rejected by
/// both a normal and a strict loader. The strict parser catches it first,
/// so both fail with `InvalidProgramHeader`.
#[test]
fn program_length_not_multiple() {
    // Build a raw ELF with a 5-byte text section (not a multiple of 8).
    // We cannot use elf_from_text_bytes because it asserts alignment.
    let text_bytes: &[u8] = &[0x95, 0x00, 0x00, 0x00, 0x00]; // 5 bytes
    let elf = build_raw_elf_with_unaligned_text(text_bytes);

    // V3 strict parser rejects non-aligned text at the ELF header level.
    // This applies regardless of stricter_loader_checks.
    let loader_strict = make_v3_loader_strict();
    let load_strict =
        Executable::<TestContextObject>::load(&elf, loader_strict).map_err(|e| format!("{e}"));
    assert!(
        load_strict.is_err(),
        "expected load to fail with stricter_loader_checks ON"
    );
    let err_strict = load_strict.unwrap_err();
    assert!(
        err_strict.contains("program header"),
        "expected 'program header' in error, got: {err_strict}",
    );

    // Without stricter_loader_checks the strict parser still catches it.
    let loader_normal = make_v3_loader();
    let load_normal =
        Executable::<TestContextObject>::load(&elf, loader_normal).map_err(|e| format!("{e}"));
    assert!(
        load_normal.is_err(),
        "expected load to fail even without stricter_loader_checks (strict parser catches it)"
    );
    let err_normal = load_normal.unwrap_err();
    assert!(
        err_normal.contains("program header"),
        "expected 'program header' in error, got: {err_normal}",
    );
}

/// Build a minimal V3-strict ELF from raw text bytes, without the alignment
/// assertion in `elf_from_text_bytes`. Used for `ProgramLengthNotMultiple`
/// testing where the text section intentionally has a non-multiple-of-8 length.
fn build_raw_elf_with_unaligned_text(text_bytes: &[u8]) -> Vec<u8> {
    use solana_sbpf::ebpf;

    const EHDR_SIZE: usize = 64;
    const PHDR_SIZE: usize = 56;
    const HEADER_REGION: usize = EHDR_SIZE + 2 * PHDR_SIZE; // 0xB0 = 176

    let text_vaddr: u64 = ebpf::MM_BYTECODE_START;
    let text_offset = HEADER_REGION as u64;
    let text_size = text_bytes.len() as u64;

    let rodata_offset = HEADER_REGION as u64;
    let rodata_vaddr: u64 = ebpf::MM_RODATA_START;
    let rodata_size: u64 = 0;

    let mut buf = Vec::with_capacity(HEADER_REGION.saturating_add(text_bytes.len()));

    // ELF header (64 bytes)
    buf.extend_from_slice(&[0x7f, 0x45, 0x4c, 0x46]); // EI_MAG
    buf.push(2); // EI_CLASS = ELFCLASS64
    buf.push(1); // EI_DATA  = ELFDATA2LSB
    buf.push(1); // EI_VERSION = EV_CURRENT
    buf.push(0); // EI_OSABI
    buf.push(0); // EI_ABIVERSION
    buf.extend_from_slice(&[0u8; 7]); // EI_PAD
    buf.extend_from_slice(&3u16.to_le_bytes()); // e_type = ET_DYN
    buf.extend_from_slice(&247u16.to_le_bytes()); // e_machine = EM_BPF
    buf.extend_from_slice(&1u32.to_le_bytes()); // e_version
    buf.extend_from_slice(&text_vaddr.to_le_bytes()); // e_entry
    buf.extend_from_slice(&(EHDR_SIZE as u64).to_le_bytes()); // e_phoff
    buf.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
    buf.extend_from_slice(&3u32.to_le_bytes()); // e_flags = SBPF V3
    buf.extend_from_slice(&(EHDR_SIZE as u16).to_le_bytes()); // e_ehsize
    buf.extend_from_slice(&(PHDR_SIZE as u16).to_le_bytes()); // e_phentsize
    buf.extend_from_slice(&2u16.to_le_bytes()); // e_phnum
    buf.extend_from_slice(&0u16.to_le_bytes()); // e_shentsize
    buf.extend_from_slice(&0u16.to_le_bytes()); // e_shnum
    buf.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx
    debug_assert_eq!(buf.len(), EHDR_SIZE);

    // Program header 0: rodata (PF_R)
    write_phdr_raw(&mut buf, 1, 4, rodata_offset, rodata_vaddr, rodata_size);

    // Program header 1: text (PF_X)
    write_phdr_raw(&mut buf, 1, 1, text_offset, text_vaddr, text_size);
    debug_assert_eq!(buf.len(), HEADER_REGION);

    // Text bytes
    buf.extend_from_slice(text_bytes);

    buf
}

fn write_phdr_raw(
    buf: &mut Vec<u8>,
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
) {
    buf.extend_from_slice(&p_type.to_le_bytes());
    buf.extend_from_slice(&p_flags.to_le_bytes());
    buf.extend_from_slice(&p_offset.to_le_bytes());
    buf.extend_from_slice(&p_vaddr.to_le_bytes());
    buf.extend_from_slice(&p_vaddr.to_le_bytes()); // p_paddr
    buf.extend_from_slice(&p_filesz.to_le_bytes());
    buf.extend_from_slice(&p_filesz.to_le_bytes()); // p_memsz
    buf.extend_from_slice(&0u64.to_le_bytes()); // p_align
}

// ---------------------------------------------------------------------------
// Skipped tests
// ---------------------------------------------------------------------------

// `UnalignedImmediate` targets `add r10, <odd>` in versions with
// `manual_stack_frame_bump()` (V1 and V2 only). V3 does not have
// `manual_stack_frame_bump`, so this verifier check is unreachable for V3
// programs. We cannot test it here because our harness uses a V3 loader.
//
// A V1/V2 test would require a V1 loader and a V1-format ELF, which is
// outside the scope of this test file (it focuses on V3 migration
// behaviour).

// ---------------------------------------------------------------------------
// Valid program baseline
// ---------------------------------------------------------------------------

/// A valid program must deploy, load, and invoke successfully under both
/// verification states.
///
/// This is the control case: `mov32 r0, 0` followed by `exit` is a valid
/// program that sets the return value to 0 and exits. It should pass
/// verification, load without verification, and invoke without error in all
/// configurations.
#[test]
fn valid_noop() {
    let elf = elf_with_assembly::<TestContextObject>("mov32 r0, 0\nexit", SBPFVersion::V3);

    // With verification: deploy must succeed.
    let deploy_result = deploy_with_verification(&elf);
    assert!(
        deploy_result.is_ok(),
        "valid program should deploy with verification: {:?}",
        deploy_result.err()
    );

    // Without verification: load must succeed.
    let load_result = load_without_verification(&elf);
    assert!(
        load_result.is_ok(),
        "valid program should load without verification: {:?}",
        load_result.err()
    );

    // Invoke must succeed.
    let invoke_result = invoke_via_interpreter(load_result.as_ref().unwrap());
    assert!(
        invoke_result.is_ok(),
        "valid program should invoke successfully: {:?}",
        invoke_result.err()
    );

    record_valid_result("valid_noop");
}

// ---------------------------------------------------------------------------
// Markdown table header ejection
// ---------------------------------------------------------------------------

/// When run first (alphabetically) with `--test-threads=1`, prints the table
/// header. This test always passes.
#[test]
fn aaa_emit_table_header() {
    if std::env::var("EJECT_ERROR_CODES").is_ok() {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("error_migration_table.md")
            .unwrap();
        writeln!(
            file,
            "| Violation | Deploy Error (verify ON) | Load (verify OFF) | Invoke (verify OFF) |"
        )
        .unwrap();
        writeln!(file, "|---|---|---|---|").unwrap();
    }
}
