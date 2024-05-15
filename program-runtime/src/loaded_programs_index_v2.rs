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
    crate::loaded_programs::{
        ProgramCacheEntry, ProgramCacheEntryOwner, ProgramCacheEntryType, ProgramRuntimeEnvironment,
    },
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
    /// The runtime environment the program was compiled for.
    /// Represented as a `u64` pointer to its location in memory.
    environment: u64,
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
        let environment = program
            .program
            .get_environment()
            .map_or(0, |env| Arc::as_ptr(env) as u64);
        Self {
            branches: Vec::new(),
            environment,
            program: Arc::clone(program),
            slot,
        }
    }

    /// Get all entries, flattened, which are verified and compiled.
    fn get_flattened_entries(
        &self,
        include_program_runtime_v1: bool,
        include_program_runtime_v2: bool,
    ) -> Vec<Arc<ProgramCacheEntry>> {
        let mut entries = vec![];
        if self.is_verified_compiled(include_program_runtime_v1, include_program_runtime_v2) {
            entries.push(Arc::clone(&self.program));
            entries.extend(self.branches.iter().flat_map(|branch| {
                branch.get_flattened_entries(include_program_runtime_v1, include_program_runtime_v2)
            }));
        }
        entries
    }

    /// [Test-only]: Get all entries, flattened.
    fn get_flattened_entries_for_tests(&self) -> Vec<Arc<ProgramCacheEntry>> {
        let mut entries = vec![Arc::clone(&self.program)];
        entries.extend(
            self.branches
                .iter()
                .flat_map(|branch| branch.get_flattened_entries_for_tests()),
        );
        entries
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

    /// Returns whether or not a node is a verified, compiled program.
    fn is_verified_compiled(
        &self,
        include_program_runtime_v1: bool,
        include_program_runtime_v2: bool,
    ) -> bool {
        match &self.program.program {
            ProgramCacheEntryType::Loaded(_) => {
                (self.program.account_owner != ProgramCacheEntryOwner::LoaderV4
                    && include_program_runtime_v1)
                    || (self.program.account_owner == ProgramCacheEntryOwner::LoaderV4
                        && include_program_runtime_v2)
            }
            _ => false,
        }
    }

    /// Prune the node and its branches based on the provided new root slot.
    ///
    /// Consider a fork graph's rooting behavior.
    ///
    /// ```
    ///
    ///        0 <- root    
    ///      /   \          
    ///     10    5           5 <- root
    ///     |    / \         / \
    ///    20   11  12      11  12          11 <- root
    ///                     |              / | \
    ///                     15          14  15  25         15 <- root
    ///                                  |   |   |          |
    ///                                 15  16  27         16
    ///                                                     |
    ///                                                    19
    ///
    /// ```
    ///
    /// Now consider the following rules.
    ///
    /// Once a slot becomes rooted:
    ///
    /// * No slots less than it can exist.
    /// * Orphaned branches must be pruned.
    ///
    /// Additionally, a program may have multiple entries at the same slot,
    /// which exist for different environments. Any outdated environments are
    /// also pruned.
    fn prune(&mut self, new_root_slot: Slot) {
        self.branches.retain(|branch| branch.slot >= new_root_slot);
        self.branches
            .iter_mut()
            .for_each(|branch| branch.prune(new_root_slot));
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
    /// The current root environment.
    root_environment: u64,
    /// The current root slot.
    root_slot: Slot,
}

impl ProgramCacheIndexV2 {
    pub(crate) fn new(
        root_slot: Slot,
        root_environment: Option<ProgramRuntimeEnvironment>,
    ) -> Self {
        let root_environment = root_environment.map_or(0, |env| Arc::as_ptr(&env) as u64);
        Self {
            graph: HashMap::new(),
            root_environment,
            root_slot,
        }
    }

    /// Get all entries, flattened, which are verified and compiled.
    pub(crate) fn get_flattened_entries(
        &self,
        include_program_runtime_v1: bool,
        include_program_runtime_v2: bool,
    ) -> Vec<(Pubkey, Arc<ProgramCacheEntry>)> {
        self.graph
            .iter()
            .flat_map(|(address, fork)| {
                fork.iter()
                    .flat_map(|node| {
                        node.get_flattened_entries(
                            include_program_runtime_v1,
                            include_program_runtime_v2,
                        )
                        .into_iter()
                        .map(move |program| (*address, program))
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// [Test-only]: Get all entries in the graph, flattened, regardless of
    /// verification/compilation status.
    pub(crate) fn get_flattened_entries_for_tests(&self) -> Vec<(Pubkey, Arc<ProgramCacheEntry>)> {
        self.graph
            .iter()
            .flat_map(|(address, fork)| {
                fork.iter()
                    .flat_map(|node| {
                        node.get_flattened_entries_for_tests()
                            .into_iter()
                            .map(move |program| (*address, program))
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
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

    /// Prune the graph.
    fn prune(&mut self, new_root_slot: Slot) {
        self.root_slot = new_root_slot;
        for fork in self.graph.values_mut() {
            for root in fork.iter_mut() {
                root.prune(new_root_slot);
            }
        }
    }

    /// Remove all entries for a program by its address.
    pub(crate) fn remove_program(&mut self, address: &Pubkey) {
        self.graph.remove(address);
    }
}
