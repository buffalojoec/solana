//! Program Cache Index V2: A fork graph data structure designed to store
//! program entries.
//!
//! At any given slot, a program's status may change. It may become ineligible
//! for use, or it may be recompiled and become eligible again. The program
//! cache index v2 is designed to track these status changes across forks.
//!
//! A program is _not_ eligible for use if it:
//!
//! * Has failed verification.
//! * Is delayed visibility.
//! * Is closed.
//!
//! # Pruning
//!
//! Because of the fork graph structure, pruning is conducted very trivially
//! each time the root slot is updated.
//!
//! Consider a fork graph.
//!
//!          0 <- root
//!        /   \
//!       10    5
//!       |     |
//!       20    11              11 <- root
//!                           / | \
//!                        14  15  25             15 <- root
//!                         |   |   |              |
//!                        15  16  27             16
//!                                                |
//!                                               19     
#![allow(unused)]

use {
    crate::loaded_programs::ProgramCacheEntry,
    solana_sdk::{clock::Slot, pubkey::Pubkey},
    std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    },
};

#[derive(Debug)]
struct Node {
    /// The node's branches.
    branches: HashSet<Node>,
    /// The cached program.
    program: Arc<ProgramCacheEntry>,
    /// The slot of the node.
    slot: Slot,
}

#[derive(Debug)]
pub(crate) struct ProgramCacheIndexV2 {
    /// Graph of program entries across forks.
    /// The key is the program address, while the value is the root node of
    /// each program's fork graph.
    graph: HashMap<Pubkey, Node>,
    /// The current root slot.
    root_slot: Slot,
}

impl ProgramCacheIndexV2 {
    pub(crate) fn new(root_slot: Slot) -> Self {
        Self {
            graph: HashMap::new(),
            root_slot,
        }
    }
}
