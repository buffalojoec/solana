#![cfg(feature = "agave-unstable-api")]
use {
    solana_account::AccountSharedData,
    solana_clock::Slot,
    solana_legacy_jit_cache_stats::ProgramStatistics,
    solana_precompile_error::PrecompileError,
    solana_pubkey::Pubkey,
    solana_sbpf::{elf::Executable, program::BuiltinProgram, vm::ContextObject},
    std::sync::Arc,
};

/// Callback used by InvokeContext in SVM
pub trait InvokeContextCallback {
    /// Returns the total current epoch stake for the network.
    fn get_epoch_stake(&self) -> u64 {
        0
    }

    /// Returns the current epoch stake for the given vote account.
    fn get_epoch_stake_for_vote_account(&self, _vote_address: &Pubkey) -> u64 {
        0
    }

    /// Returns true if the program_id corresponds to a precompiled program
    fn is_precompile(&self, _program_id: &Pubkey) -> bool {
        false
    }

    /// Calls the precompiled program corresponding to the given program ID.
    fn process_precompile(
        &self,
        _program_id: &Pubkey,
        _data: &[u8],
        _instruction_datas: Vec<&[u8]>,
    ) -> Result<(), PrecompileError> {
        Err(PrecompileError::InvalidPublicKey)
    }
}

/// Runtime callbacks for transaction processing.
pub trait TransactionProcessingCallback {
    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<(AccountSharedData, Slot)>;

    fn inspect_account(&self, _address: &Pubkey, _account_state: AccountState, _is_writable: bool) {
    }
}

/// The state the account is in initially, before transaction processing
#[derive(Debug)]
pub enum AccountState<'a> {
    /// This account is dead, and will be created by this transaction
    Dead,
    /// This account is alive, and already existed prior to this transaction
    Alive(&'a AccountSharedData),
}

/// This trait lets us abstract over the program JIT cache implementation in SVM
/// and program-runtime, to make room for the Kita Cache.
pub trait InvokeContextProgramLoader<C: ContextObject> {
    /// Find the loaded program for `program_id`, if present.
    fn find(&self, program_id: &Pubkey) -> Option<Arc<dyn LoadedProgram<C>>>;

    /// Compile `elf_bytes` for `program_id` and insert the result into the
    /// cache, so a subsequent [`find`](Self::find) hits.
    fn load(&self, program_id: &Pubkey, elf_bytes: &[u8]);

    /// Insert a freshly deployed program into the cache.
    fn deploy(&self, program_id: &Pubkey, program: Arc<Executable<C>>);
}

/// This trait lets us abstract over the program JIT cache implementation in SVM
/// and program-runtime, to make room for the Kita Cache.
pub trait LoadedProgram<C: ContextObject> {
    /// The verified, executable program, if this entry holds one.
    fn executable(&self) -> Option<&Executable<C>>;

    /// The built-in program, if this entry is a built-in.
    fn builtin(&self) -> Option<&BuiltinProgram<C>>;

    /// Used to record usage stats on the legacy cache.
    fn legacy_stats(&self) -> Option<&ProgramStatistics> {
        None
    }
}
