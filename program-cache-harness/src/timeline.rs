//! Frames to run against the fork graph.

use {crate::entry::Entry, solana_pubkey::Pubkey};

/// One moment in the timeline, starting from the initial test environment
/// prepared by [`crate::genesis::Genesis`].
#[derive(Clone, Debug)]
pub struct Frame {
    /// Phase one, single-thread: lay out the fork graph.
    pub build: Vec<Build>,
    /// Phase two, concurrent: one thread per batch, against banks `build` has
    /// already created. Each batch checks its own fork-scoped batch cache.
    pub run: Vec<Run>,
    /// Phase three, single-thread: assert the entire contents of the global
    /// program cache.
    pub assert: Vec<Entry>,
}

/// A node to add to the fork graph.
#[derive(Clone, Copy, Debug)]
pub enum Build {
    /// Extend the canonical fork to `slot`. Its parent is the current tip.
    Advance {
        /// This node's slot.
        slot: u64,
    },
    /// Create a slot on `parent`, leaving the canonical fork where it is.
    /// Starting a branch and extending one are both just this.
    NewSlotOn {
        /// Parent slot.
        parent: u64,
        /// This node's slot.
        slot: u64,
    },
}

/// A batch of transactions to run against one node in the fork graph.
#[derive(Clone, Debug)]
pub enum Run {
    /// Invoke each target in the `targets` list with its own transaction.
    /// Invoking a target runs the cache extraction workflow, causing the
    /// program to be loaded into the transaction processing pipeline.
    Invoke {
        /// The node in the fork graph to target.
        slot: u64,
        /// Targets to invoke. Single transaction per invocation, one batch.
        targets: Vec<Pubkey>,
        /// The cache entry each target resolved to on this fork.
        served: Vec<Entry>,
    },
    /// Deploy each target in the `targets` list with its own transaction.
    /// A target already deployed on this fork is upgraded rather than created.
    Deploy {
        /// The node in the fork graph to target.
        slot: u64,
        /// Targets to deploy. Single transaction per deployment, one batch.
        targets: Vec<Pubkey>,
    },
    /// Close each target in the `targets` list with its own transaction.
    Close {
        /// The node in the fork graph to target.
        slot: u64,
        /// Targets to close. Single transaction per close, one batch.
        targets: Vec<Pubkey>,
    },
}
