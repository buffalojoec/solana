//! Steps to run against the fork graph.

use {crate::effects::Expect, solana_pubkey::Pubkey};

/// A step in the fork graph.
#[derive(Clone, Debug)]
pub enum Step {
    /// Create a new slot in the fork graph. The `parent` dictates which fork
    /// this new node lives on.
    NewSlot {
        /// Parent slot.
        parent: u64,
        /// This node's slot.
        slot: u64,
        /// Whether this node becomes the canonical tip, the fork the harness
        /// roots against.
        canonical: bool,
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
    /// Assert what the global program cache holds at this point.
    Assert(Vec<Expect>),
}

/// The timeline of events that takes place, starting from the initial test
/// environment prepared by [`crate::genesis::Genesis`].
#[derive(Clone, Debug)]
pub struct Timeline {
    /// Steps to run through in the timeline.
    pub steps: Vec<Step>,
}
