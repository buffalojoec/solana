//! The input to a run.

#![allow(clippy::arithmetic_side_effects)]

mod arbitrary;
mod fork_tree;

pub use fork_tree::{ForkTree, tree};
use {
    solana_clock::Slot, solana_epoch_schedule::MINIMUM_SLOTS_PER_EPOCH,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner as Owner,
};

/// The shortest epoch the schedule allows.
pub const SLOTS_PER_EPOCH: Slot = MINIMUM_SLOTS_PER_EPOCH;
/// One epoch boundary per scenario.
pub const MAX_SLOT: Slot = SLOTS_PER_EPOCH * 2 - 1;

/// The `n`th slot of the epoch a run crosses into.
pub fn slots_in_new_epoch(n: Slot) -> Slot {
    let slot = SLOTS_PER_EPOCH + n;
    debug_assert!(slot <= MAX_SLOT, "a scenario crosses one epoch boundary");
    slot
}

pub(crate) const NUM_PROGRAMS: u8 = 3;
pub(crate) const NUM_ENVIRONMENTS: u8 = 2;

pub(crate) fn environment_of(slot: Slot) -> u8 {
    (slot / SLOTS_PER_EPOCH) as u8 % NUM_ENVIRONMENTS
}

/// Input: an entire harness run.
#[derive(Clone, Debug, Default)]
pub struct Scenario {
    pub tree: ForkTree,
    pub seeds: Vec<Seed>,
    pub ops: Vec<Op>,
}

/// Seeded deployment.
#[derive(Clone, Copy, Debug)]
pub struct Seed {
    pub program: u8,
    pub owner: Owner,
    /// Whether the bytecode the seed writes verifies (`FailedVerification`).
    pub verifies: bool,
}

/// One step of a scenario.
#[derive(Clone, Debug)]
pub enum Op {
    /// A program account is written in slot `at`, and the resulting entry is
    /// assigned into the cache. It is built for the environment `at` runs on.
    Deploy { program: u8, at: Slot },
    /// A program account is closed in slot `at`.
    Close { program: u8, at: Slot },
    /// A batch on `fork_tip` searches for several programs at once, naming
    /// for each the deployment slot its own account state reports.
    Extract { programs: Vec<u8>, fork_tip: Slot },
    /// A cooperative loading task started by a previous `Extract` completes,
    /// either verifying the bytecode or failing to.
    FinishLoad { program: u8, result: LoadResult },
    /// The root moves, and with it the epoch if the new root is in the next
    /// one.
    Prune { root: Slot },
    /// The epoch boundary preparation phase recompiles one program for the
    /// environment which is coming, ahead of the boundary.
    RecompileForEpoch { program: u8, fork_tip: Slot },
    /// A block is dumped and will be replayed, the way `BankForks::clear_bank`
    /// handles a duplicate. Whatever that block deployed never happened, so
    /// the entries go with it.
    PurgeSlot { slot: Slot },
}

/// What a cooperative load produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadResult {
    Loaded,
    FailedVerification,
}
