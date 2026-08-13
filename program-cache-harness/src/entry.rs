//! Program cache entry types.

use {
    crate::consts::DEFAULT_ENTRY_OWNER,
    solana_account::AccountSharedData,
    solana_program_runtime::{
        declare_process_instruction,
        loaded_programs::ProgramRuntimeEnvironment,
        program_cache_entry::{ProgramCacheEntry, ProgramCacheEntryType},
        program_metrics::LoadProgramMetrics,
    },
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    std::sync::Arc,
};

/// The ELF backing every non-builtin entry. What the program does is not
/// important, only that it verifies and executes.
const NOOP_ELF: &[u8] =
    include_bytes!("../../programs/bpf_loader/test_elfs/out/sbpfv3_return_ok.so");

// The no-op builtin function backing every builtin entry.
declare_process_instruction!(NoopBuiltin, 1, |_invoke_context| { Ok(()) });

/// Represents the type of cache entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryType {
    /// The program account exists, but the cache holds nothing for it. Only
    /// meaningful as an input; reading the cache back never yields one.
    Cold,
    /// The no-op ELF, verified and compiled into the entry's `Executable`.
    Loaded,
    /// An unloaded executable; from eviction or new deployment.
    Unloaded,
    /// A closed tombstone.
    Closed,
    /// A builtin program.
    Builtin,
}

/// Represents a cached program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Program ID.
    pub id: Pubkey,
    /// Deployment slot.
    pub slot: u64,
    /// Type of cache entry.
    pub ty: EntryType,
}

impl Entry {
    fn new(id: Pubkey, slot: u64, ty: EntryType) -> Self {
        Self { id, slot, ty }
    }

    pub fn new_cold(id: Pubkey, slot: u64) -> Self {
        Self::new(id, slot, EntryType::Cold)
    }

    pub fn new_loaded(id: Pubkey, slot: u64) -> Self {
        Self::new(id, slot, EntryType::Loaded)
    }

    pub fn new_unloaded(id: Pubkey, slot: u64) -> Self {
        Self::new(id, slot, EntryType::Unloaded)
    }

    pub fn new_closed(id: Pubkey, slot: u64) -> Self {
        Self::new(id, slot, EntryType::Closed)
    }

    pub fn new_builtin(id: Pubkey) -> Self {
        Self::new(id, 0, EntryType::Builtin)
    }

    pub(crate) fn account(&self) -> Option<AccountSharedData> {
        if self.ty == EntryType::Builtin {
            // `Bank::add_mockup_builtin` writes its own account.
            return None;
        }
        Some(AccountSharedData::from(solana_account::Account {
            lamports: Rent::default().minimum_balance(NOOP_ELF.len()).max(1),
            data: NOOP_ELF.to_vec(),
            owner: DEFAULT_ENTRY_OWNER.into(),
            executable: true,
            rent_epoch: 0,
        }))
    }

    pub(crate) fn program_cache_entry(
        &self,
        environment: &ProgramRuntimeEnvironment,
    ) -> Option<Arc<ProgramCacheEntry>> {
        let entry = match self.ty {
            EntryType::Loaded => ProgramCacheEntry::load(
                &DEFAULT_ENTRY_OWNER.into(),
                environment.clone(),
                self.slot,
                NOOP_ELF,
                &mut LoadProgramMetrics::default(),
            )
            .expect("failed to load ELF"),
            EntryType::Unloaded => {
                ProgramCacheEntry::new_unloaded(self.slot, DEFAULT_ENTRY_OWNER, environment.clone())
            }
            EntryType::Closed => {
                ProgramCacheEntry::new_closed_tombstone(self.slot, DEFAULT_ENTRY_OWNER)
            }
            EntryType::Cold => {
                // "Cold" entries have on-chain accounts but aren't cached.
                return None;
            }
            EntryType::Builtin => {
                // `Bank::add_mockup_builtin` injects its own cache entry.
                return None;
            }
        };
        Some(Arc::new(entry))
    }

    pub(crate) fn from_program_cache_entry(id: Pubkey, entry: &ProgramCacheEntry) -> Option<Self> {
        let ty = match entry.program {
            ProgramCacheEntryType::Loaded(_) => EntryType::Loaded,
            ProgramCacheEntryType::Unloaded(_) => EntryType::Unloaded,
            ProgramCacheEntryType::Closed => EntryType::Closed,
            ProgramCacheEntryType::Builtin(_) => EntryType::Builtin,
            // No [`EntryType`] models these yet, so they drop out of snapshots.
            ProgramCacheEntryType::FailedVerification(_)
            | ProgramCacheEntryType::DelayVisibility => return None,
        };
        Some(Self::new(id, entry.deployment_slot, ty))
    }
}
