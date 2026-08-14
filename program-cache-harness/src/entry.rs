//! Program cache entry types.

use {
    crate::consts::{DEFAULT_ENTRY_OWNER, NOOP_ELF},
    solana_account::AccountSharedData,
    solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState},
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

/// Which program runtime environment an entry belongs to.
///
/// The cache compares environments by `Arc` pointer, not by content, so these
/// are identities rather than configurations: `Alternate` is a second
/// environment built exactly like `Current`, and differs only in being a
/// different allocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Env {
    /// The environment the bank executes with.
    #[default]
    Current,
    /// An environment the bank is not executing with. `extract` skips entries
    /// under it, and `prune` discards them once the root moves past.
    Alternate,
}

/// The environments a timeline can name, and their identities.
pub(crate) struct Environments {
    pub(crate) current: ProgramRuntimeEnvironment,
    pub(crate) alternate: ProgramRuntimeEnvironment,
}

impl Environments {
    pub(crate) fn get(&self, env: Env) -> &ProgramRuntimeEnvironment {
        match env {
            Env::Current => &self.current,
            Env::Alternate => &self.alternate,
        }
    }

    /// Resolve an entry's environment back to a stable id. Entries carrying
    /// none — tombstones and builtins — report `Current`.
    fn id_of(&self, environment: Option<&ProgramRuntimeEnvironment>) -> Env {
        match environment {
            Some(environment) if environment == &self.alternate => Env::Alternate,
            _ => Env::Current,
        }
    }
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
    /// The environment this entry belongs to.
    pub env: Env,
}

impl Entry {
    fn new(id: Pubkey, slot: u64, ty: EntryType) -> Self {
        Self {
            id,
            slot,
            ty,
            env: Env::Current,
        }
    }

    /// Place this entry under `env` rather than the bank's own environment.
    pub fn in_env(mut self, env: Env) -> Self {
        self.env = env;
        self
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

    pub(crate) fn accounts(
        &self,
        upgrade_authority: &Pubkey,
    ) -> Option<[(Pubkey, AccountSharedData); 2]> {
        if self.ty == EntryType::Builtin {
            // `Bank::add_mockup_builtin` writes its own account.
            return None;
        }

        let rent = Rent::default();
        let programdata_address = get_program_data_address(&self.id);

        let program = {
            let data = bincode::serialize(&UpgradeableLoaderState::Program {
                programdata_address,
            })
            .unwrap();
            AccountSharedData::from(solana_account::Account {
                lamports: rent.minimum_balance(data.len()).max(1),
                data,
                owner: DEFAULT_ENTRY_OWNER.into(),
                executable: true,
                rent_epoch: 0,
            })
        };

        let programdata = {
            // The deployment slot the cache reports comes from this header.
            let mut data = bincode::serialize(&UpgradeableLoaderState::ProgramData {
                slot: self.slot,
                upgrade_authority_address: Some(*upgrade_authority),
            })
            .unwrap();
            data.extend_from_slice(NOOP_ELF);
            AccountSharedData::from(solana_account::Account {
                lamports: rent.minimum_balance(data.len()).max(1),
                data,
                owner: DEFAULT_ENTRY_OWNER.into(),
                executable: false,
                rent_epoch: 0,
            })
        };

        Some([(self.id, program), (programdata_address, programdata)])
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

    pub(crate) fn from_program_cache_entry(
        id: Pubkey,
        entry: &ProgramCacheEntry,
        environments: &Environments,
    ) -> Option<Self> {
        let ty = match entry.program {
            ProgramCacheEntryType::Loaded(_) => EntryType::Loaded,
            ProgramCacheEntryType::Unloaded(_) => EntryType::Unloaded,
            ProgramCacheEntryType::Closed => EntryType::Closed,
            ProgramCacheEntryType::Builtin(_) => EntryType::Builtin,
            // No [`EntryType`] models these yet, so they drop out of snapshots.
            ProgramCacheEntryType::FailedVerification(_)
            | ProgramCacheEntryType::DelayVisibility => return None,
        };
        let env = environments.id_of(entry.program.get_environment());
        Some(Self::new(id, entry.deployment_slot, ty).in_env(env))
    }
}

#[cfg(test)]
mod tests {
    use {super::*, solana_account::ReadableAccount};

    #[test]
    fn builtins_bring_no_accounts() {
        assert!(
            Entry::new_builtin(Pubkey::new_unique())
                .accounts(&Pubkey::new_unique())
                .is_none()
        );
    }

    #[test]
    fn loader_v3_splits_a_program_across_two_accounts() {
        let id = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let [
            (program_address, program),
            (programdata_address, programdata),
        ] = Entry::new_cold(id, 7)
            .accounts(&authority)
            .expect("no accounts");

        assert_eq!(program_address, id);
        assert_eq!(programdata_address, get_program_data_address(&id));
        assert_eq!(program.owner(), &Pubkey::from(DEFAULT_ENTRY_OWNER));
        assert_eq!(programdata.owner(), &Pubkey::from(DEFAULT_ENTRY_OWNER));
        assert!(program.executable());
        assert!(!programdata.executable());

        assert_eq!(
            bincode::deserialize::<UpgradeableLoaderState>(program.data()).unwrap(),
            UpgradeableLoaderState::Program {
                programdata_address,
            },
        );

        let (header, elf) = programdata
            .data()
            .split_at(UpgradeableLoaderState::size_of_programdata_metadata());
        assert_eq!(
            bincode::deserialize::<UpgradeableLoaderState>(header).unwrap(),
            UpgradeableLoaderState::ProgramData {
                slot: 7,
                upgrade_authority_address: Some(authority),
            },
        );
        assert_eq!(elf, NOOP_ELF);
    }
}
