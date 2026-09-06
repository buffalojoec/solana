//! One extraction, and what a report keeps of it.

use {
    crate::{entry::EntryKind, ledger::AccountState},
    solana_clock::Slot,
    solana_program_runtime::{
        loaded_programs::ProgramRuntimeEnvironment, program_cache_entry::ProgramCacheEntry,
    },
    std::sync::Arc,
};

pub struct Extraction {
    pub program: u8,

    pub batch_slot: Slot,
    pub batch_env: ProgramRuntimeEnvironment,

    /// The caller's fork, from genesis up to its batch slot.
    pub ancestry: Vec<Slot>,
    /// What the caller's own account state said.
    pub account_state: AccountState,

    /// Every entry the run has put in at the deployment slot the caller named.
    /// There can be more than one for different environments.
    pub candidates: Vec<Arc<ProgramCacheEntry>>,

    /// The entry that the extraction returned. `None` means a reload was
    /// signaled.
    pub returned: Option<Arc<ProgramCacheEntry>>,
    /// Whether this batch was handed a cooperative loading task.
    pub started_load: bool,
    /// Whether a load for this same program and deployment slot has already
    /// completed.
    pub already_loaded_here: bool,
}

/// The part of an [`Extraction`] a `Report` keeps once the invariants have
/// run: no cache objects, so a report outlives the entries it saw and two runs
/// of one scenario stay comparable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractionRecord {
    pub program: u8,
    pub batch_slot: Slot,
    pub asked_for: Slot,
    pub hit: bool,
    pub kind: Option<EntryKind>,
    pub started_load: bool,
}

impl Extraction {
    pub fn record(&self) -> ExtractionRecord {
        ExtractionRecord {
            program: self.program,
            batch_slot: self.batch_slot,
            asked_for: self.account_state.deployment_slot,
            hit: self.returned.is_some() && !self.started_load,
            kind: self
                .returned
                .as_ref()
                .map(|entry| EntryKind::of(&entry.program)),
            started_load: self.started_load,
        }
    }
}
