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
    crate::loaded_programs::{ProgramCacheEntry, ProgramCacheEntryType},
    solana_sdk::{clock::Slot, pubkey::Pubkey},
    std::{
        collections::{hash_map::Entry, HashMap},
        sync::{atomic::Ordering, Arc},
    },
};

#[derive(Debug)]
struct Node {
    /// The node's branches.
    branches: Vec<Node>,
    /// The cached program.
    program: Arc<ProgramCacheEntry>,
    /// The slot of the node.
    slot: Slot,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.program.deployment_slot == other.program.deployment_slot && self.slot == other.slot
    }
}

impl Eq for Node {}

impl std::hash::Hash for Node {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.program) as u64).hash(state);
        self.slot.hash(state);
    }
}

impl Node {
    fn new(program: &Arc<ProgramCacheEntry>, slot: Slot) -> Self {
        Self {
            branches: Vec::new(),
            program: Arc::clone(program),
            slot,
        }
    }

    /// Insert a new entry in the program graph. Returns `true` when the entry
    /// is inserted, `false` otherwise.
    ///
    /// Consider a fork graph.
    ///
    /// ```
    ///
    ///            |3|                  |9|                      |12|
    ///             |                    |                        |
    ///             x                    |                        |
    ///         5   ^ can't          5   |                   5    |
    ///        /      add          /   \                   /   \  |
    ///       8                   8    |9|                8     9  
    ///                           |                       |    / \
    ///                          10     ^ new            10   11 |12|
    ///                                   branch            
    ///                                                            ^ added to
    ///                                                              branch
    /// ```
    ///
    /// Thus:
    ///
    /// * If `slot` < `self.slot`, do nothing.
    /// * If `slot` == `self.slot`, update the current node if it's a match.
    /// * If `slot` > `self.slot`, move into the branches.
    ///
    /// Note that the fork graph can contain multiple program entries at the
    /// same slot. This is because the program may have been recompiled with
    /// a different environment. This is why, in order to update a program at
    /// a given slot, it must be a match.
    fn insert(&mut self, program: &Arc<ProgramCacheEntry>, slot: Slot) -> bool {
        if slot < self.slot {
            return false;
        }
        if slot == self.slot {
            if self.program.deployment_slot == program.deployment_slot {
                self.update(program);
                return true;
            } else {
                return false;
            }
        }
        for branch in self.branches.iter_mut() {
            if branch.insert(program, slot) {
                return true;
            }
        }
        // Insert a new branch.
        self.branches.push(Node::new(program, slot));
        true
    }

    /// Update an existing node with the provided entry.
    ///
    /// Only three types of replacements are allowed:
    ///
    /// * `Builtin` -> `Builtin`: Usage counters are updated.
    /// * `Unloaded` -> `Loaded`: Usage counters are updated.
    /// * `Loaded` -> `Unloaded`: Usage counters are _not_ updated.
    ///
    /// Notice that when a loaded program is replaced with an unloaded program,
    /// the usage counters are _not_ updated. This is because the program has
    /// been unloaded, not invoked.
    fn update(&mut self, new: &Arc<ProgramCacheEntry>) {
        match (&self.program.program, &new.program) {
            (ProgramCacheEntryType::Builtin(_), ProgramCacheEntryType::Builtin(_))
            | (ProgramCacheEntryType::Unloaded(_), ProgramCacheEntryType::Loaded(_)) => {
                // Usage counters are updated.
                new.tx_usage_counter.fetch_add(
                    self.program.tx_usage_counter.load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
                new.ix_usage_counter.fetch_add(
                    self.program.ix_usage_counter.load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
                self.program = Arc::clone(new);
            }
            (ProgramCacheEntryType::Loaded(_), ProgramCacheEntryType::Unloaded(_)) => {
                // Usage counters are _not_ updated.
                self.program = Arc::clone(new);
            }
            _ => {
                // This is a bug. How can we get rid of this?
                panic!("Unexpected replacement of an entry");
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct ProgramCacheIndexV2 {
    /// Graph of program entries across forks.
    /// The key is the program address, while the value is the root nodes of
    /// each program's fork graph.
    graph: HashMap<Pubkey, Vec<Node>>,
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

    /// Insert a new program entry into the program graph.
    pub(crate) fn insert(&mut self, address: Pubkey, program: &Arc<ProgramCacheEntry>) {
        let slot = program.deployment_slot;
        let root = self.graph.entry(address).or_default();
        for node in root.iter_mut() {
            if node.insert(program, slot) {
                return;
            }
        }
        root.push(Node::new(program, slot));
    }

    /// Remove all entries for a program by its address.
    pub(crate) fn remove_program(&mut self, address: &Pubkey) {
        self.graph.remove(address);
    }
}
