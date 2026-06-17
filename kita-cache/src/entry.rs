//! Program JIT cache entry.

use {
    solana_clock::Slot,
    solana_sbpf::{elf::Executable, program::BuiltinProgram, vm::ContextObject},
    solana_svm_callback::LoadedProgram,
    std::sync::Arc,
};

/// A program JIT cache entry: a compiled program, a built-in, a delayed-
/// visibility program, or a tombstone.
pub enum Entry<C: ContextObject> {
    /// A successfully JIT-compiled program.
    Program(Arc<Executable<C>>),
    /// A built-in program, backed into the validator rather than compiled from
    /// an on-chain ELF.
    Builtin(Arc<BuiltinProgram<C>>),
    /// A compiled program that is not yet visible (see [`DelayedProgram`]).
    DelayedVisibility(DelayedProgram<C>),
    /// A program that could not be JIT-compiled, marked with the [`Reason`].
    Tombstone(Reason),
}

/// A compiled program withheld from execution until its `effective_slot`.
///
/// The Kita Cache itself has no need to delay a freshly deployed program - it
/// could compile and serve it immediately. This variant exists solely to match
/// the legacy program cache, which makes a newly deployed program effective only
/// one slot after deployment; honoring that is required to stay in consensus
/// while both caches coexist. Once the Kita Cache is the production cache, delay
/// visibility should be removed from the protocol and this variant deleted.
pub struct DelayedProgram<C: ContextObject> {
    pub program: Arc<Executable<C>>,
    pub effective_slot: Slot,
}

impl<C: ContextObject> Clone for DelayedProgram<C> {
    fn clone(&self) -> Self {
        Self {
            program: Arc::clone(&self.program),
            effective_slot: self.effective_slot,
        }
    }
}

impl<C: ContextObject> Clone for Entry<C> {
    fn clone(&self) -> Self {
        match self {
            Entry::Program(program) => Entry::Program(Arc::clone(program)),
            Entry::Builtin(builtin) => Entry::Builtin(Arc::clone(builtin)),
            Entry::DelayedVisibility(delayed) => Entry::DelayedVisibility(delayed.clone()),
            Entry::Tombstone(reason) => Entry::Tombstone(reason.clone()),
        }
    }
}

impl<C: ContextObject> Entry<C> {
    /// Whether this entry should be recompiled in response to a feature
    /// activation.
    pub fn should_recompile_for_feature_activation(&self) -> bool {
        match self {
            Entry::Program(_) | Entry::DelayedVisibility(_) => true,
            // Built-ins are native, not compiled from an ELF.
            Entry::Builtin(_) => false,
            Entry::Tombstone(reason) => reason.should_recompile_for_feature_activation(),
        }
    }
}

impl<C: ContextObject> LoadedProgram<C> for Entry<C> {
    fn executable(&self) -> Option<&Executable<C>> {
        match self {
            Entry::Program(executable) => Some(executable.as_ref()),
            _ => None,
        }
    }

    fn builtin(&self) -> Option<&BuiltinProgram<C>> {
        match self {
            Entry::Builtin(builtin) => Some(builtin.as_ref()),
            _ => None,
        }
    }

    // `legacy_stats` defaults to `None`: the kita cache tracks usage separately
    // from the legacy `ProgramStatistics`.
}

/// Why a program was tombstoned rather than compiled.
#[derive(Clone, Debug)]
pub enum Reason {
    /// A syscall is not recognized under the current feature set.
    UnknownSyscall,
    /// A syscall is recognized and not supported.
    UnsupportedSyscall,
    /// The loader configuration is not recognized under the current feature set.
    UnknownConfig,
    /// The loader configuration is recognized and not supported.
    UnsupportedConfig,
    /// The program ABI is recognized and not supported.
    /// This is tied to the program's owner, ie. Loader V1.
    UnsupportedAbi,
    /// Any other reason compilation failed.
    Other,
}

impl Reason {
    fn should_recompile_for_feature_activation(&self) -> bool {
        match self {
            // An unknown syscall or config may become recognized once a feature
            // is activated, so it is worth another attempt.
            Reason::UnknownSyscall | Reason::UnknownConfig => true,
            // An unsupported syscall or config will not become valid unless
            // the program is re-deployed. An unsupported ABI will never become
            // valid. No need to recompile.
            Reason::UnsupportedSyscall | Reason::UnsupportedConfig | Reason::UnsupportedAbi => {
                false
            }
            // The reason is unknown, so err toward retrying.
            Reason::Other => true,
        }
    }
}
