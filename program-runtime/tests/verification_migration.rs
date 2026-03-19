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
        vm::Config,
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

/// Run the full error-migration check for a given ELF.
fn check_error_migration(elf: &[u8]) -> ErrorMigration {
    // With verification: deploy must fail
    let deploy_err =
        deploy_with_verification(elf).expect_err("deploy should fail with verification ON");

    // Without verification: load must succeed
    let load_result = load_without_verification(elf);
    let load_without_verify_ok = load_result.is_ok();
    if let Ok(ref executable) = load_result {
        // Confirm it actually fails verification
        let verify_result = executable.verify::<RequisiteVerifier>();
        assert!(verify_result.is_err(), "expected verification to fail");
    }

    ErrorMigration {
        deploy_error: deploy_err,
        load_without_verify_ok,
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
        writeln!(file, "| {name} | OK | OK |").unwrap();
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
        writeln!(
            file,
            "| {name} | {deploy_err} | {skip_ok} |",
            deploy_err = migration.deploy_error,
            skip_ok = if migration.load_without_verify_ok {
                "OK (loaded)"
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
}

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
}

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
}

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
}

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
}

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
}

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
}

// ---------------------------------------------------------------------------
// Raw-byte tests (assembler cannot emit these violations)
// ---------------------------------------------------------------------------

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
}

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
}

#[test]
fn incomplete_lddw() {
    let elf = elf_from_text_bytes(corpus::raw::INCOMPLETE_LDDW);
    let m = check_error_migration(&elf);
    record_result("incomplete_lddw", &m);
    assert!(
        m.deploy_error.contains("incomplete LD_DW"),
        "expected 'incomplete LD_DW', got: {}",
        m.deploy_error
    );
    assert!(m.load_without_verify_ok);
}

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
}

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
}

// ---------------------------------------------------------------------------
// Jump to middle of LDDW (assembly-based, LDDW available in V3)
// ---------------------------------------------------------------------------

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
}

// ---------------------------------------------------------------------------
// Valid program baseline
// ---------------------------------------------------------------------------

/// A valid program must deploy and load successfully under both feature states.
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
            "| Violation | Deploy Error (verification ON) | Deploy (verification OFF) |"
        )
        .unwrap();
        writeln!(file, "|---|---|---|").unwrap();
    }
}
