//! Steps to run against the fork graph.

use {crate::entry::Entry, solana_pubkey::Pubkey};

/// A step in the fork graph.
#[derive(Clone, Debug)]
pub enum Step {
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
    /// Invoke each target in the `targets` list with its own transaction.
    /// Invoking a target runs the cache extraction workflow, causing the
    /// program to be loaded into the transaction processing pipeline.
    Invoke {
        /// The node in the fork graph to target.
        slot: u64,
        /// Targets to invoke. Single transaction per invocation, one batch.
        targets: Vec<Pubkey>,
    },
    /// Deploy each target in the `targets` list with its own transaction.
    /// A target already deployed on this fork is upgraded rather than created.
    Deploy {
        /// The node in the fork graph to target.
        slot: u64,
        /// Targets to deploy. Single transaction per deployment, one batch.
        targets: Vec<Pubkey>,
    },
    /// Assert the entire contents of the global program cache at this point.
    /// Anything the list omits must be absent.
    Assert(Vec<Entry>),
}

/// The timeline of events that takes place, starting from the initial test
/// environment prepared by [`crate::genesis::Genesis`].
#[derive(Clone, Debug)]
pub struct Timeline {
    /// Steps to run through in the timeline.
    pub steps: Vec<Step>,
}
