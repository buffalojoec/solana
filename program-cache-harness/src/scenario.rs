//! The input to a run.

mod arbitrary;
mod fork_tree;

pub use fork_tree::{ForkTree, tree};
use {
    solana_clock::Slot,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner as Owner,
};

pub(crate) const NUM_PROGRAMS: u8 = 3;
pub(crate) const NUM_ENVIRONMENTS: u8 = 3;

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
    pub env: u8,
    /// Whether the bytecode the seed writes verifies (`FailedVerification`).
    pub verifies: bool,
}

/// One step of a scenario.
#[derive(Clone, Debug)]
pub enum Op {
    /// A program account is written in slot `at`, and the resulting entry is
    /// assigned into the cache.
    Deploy { program: u8, at: Slot, env: u8 },
    /// A program account is closed in slot `at`.
    Close { program: u8, at: Slot },
    /// A batch on `fork_tip` searches for several programs at once, naming
    /// for each the deployment slot its own account state reports.
    Extract { programs: Vec<u8>, fork_tip: Slot },
    /// A cooperative loading task started by a previous `Extract` completes,
    /// either verifying the bytecode or failing to.
    FinishLoad { program: u8, result: LoadResult },
    /// The root moves.
    Prune { root: Slot },
    /// The epoch boundary preparation phase recompiles one program for the
    /// environment which is coming, ahead of the boundary.
    RecompileForEpoch { program: u8, fork_tip: Slot },
    /// The root moves and the epoch turns over with it. The environment which
    /// was upcoming becomes the one every later batch runs on, and entries
    /// built for the old one are swept.
    CrossEpochBoundary { root: Slot },
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
