//! Program (re)compilation.
//!
//! Loads, verifies, and JIT-compiles a program against a given environment,
//! producing a cache [`Entry`] - either a compiled [`Program`](Entry::Program)
//! or a [`Tombstone`](Entry::Tombstone) classified by the failure, so a program
//! that cannot be compiled is recorded rather than silently retried.
//!
//! Shared by the anticipation cache and by on-the-fly recompilation triggered
//! during transaction processing, so both classify failures identically.
//! Program deployments recompile through their own path.

use {
    crate::entry::{Entry, Reason},
    solana_sbpf::{
        elf::{ElfError, Executable},
        error::EbpfError,
        program::BuiltinProgram,
        verifier::{RequisiteVerifier, VerifierError},
        vm::ContextObject,
    },
    std::sync::Arc,
};

/// Compile `elf_bytes` against `loader`, returning the resulting cache entry: a
/// compiled program on success, or a tombstone classifying the failure.
///
/// `loader` carries the environment - config and registered syscalls - to
/// compile against.
pub fn compile<C: ContextObject>(loader: &Arc<BuiltinProgram<C>>, elf_bytes: &[u8]) -> Entry<C> {
    match try_compile(loader, elf_bytes) {
        Ok(executable) => Entry::Program(Arc::new(executable)),
        Err(reason) => Entry::Tombstone(reason),
    }
}

// Run the load -> verify -> JIT-compile pipeline, mapping any failure to the
// tombstone reason that best explains it.
fn try_compile<C: ContextObject>(
    loader: &Arc<BuiltinProgram<C>>,
    elf_bytes: &[u8],
) -> Result<Executable<C>, Reason> {
    let executable = Executable::load(elf_bytes, Arc::clone(loader)).map_err(reason_from_elf)?;
    executable
        .verify::<RequisiteVerifier>()
        .map_err(reason_from_ebpf)?;
    // JIT compilation only runs where it is supported; elsewhere the executable
    // is still loaded and verified, matching the runtime's own behavior.
    #[cfg(all(not(target_os = "windows"), target_arch = "x86_64"))]
    executable.jit_compile().map_err(reason_from_ebpf)?;
    Ok(executable)
}

// Classify an ELF load failure.
fn reason_from_elf(error: ElfError) -> Reason {
    match error {
        // The program references a syscall the loader does not have; a later
        // feature activation may register it.
        ElfError::UnresolvedSymbol(..) => Reason::UnknownSyscall,
        // The program requires an SBPF version not enabled by the current
        // feature set; a feature activation may enable it.
        ElfError::UnsupportedSBPFVersion => Reason::UnknownConfig,
        // A fundamentally incompatible binary format - never valid here.
        ElfError::WrongAbi
        | ElfError::WrongEndianess
        | ElfError::WrongMachine
        | ElfError::WrongClass
        | ElfError::WrongType => Reason::UnsupportedAbi,
        _ => Reason::Other,
    }
}

// Classify a verification or JIT-compilation failure.
fn reason_from_ebpf(error: EbpfError) -> Reason {
    match error {
        EbpfError::ElfError(elf) => reason_from_elf(elf),
        EbpfError::VerifierError(verifier) => reason_from_verifier(verifier),
        _ => Reason::Other,
    }
}

// Classify a bytecode verification failure.
fn reason_from_verifier(error: VerifierError) -> Reason {
    match error {
        // The program calls a syscall code the loader does not recognize; a
        // later feature activation may register it.
        VerifierError::InvalidSyscall(_) => Reason::UnknownSyscall,
        _ => Reason::Other,
    }
}
