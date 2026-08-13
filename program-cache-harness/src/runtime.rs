//! The test apparatus driven by a [`Timeline`](crate::timeline::Timeline).
//!
//! One [`BankForks`] holds every bank in the timeline. All of them share a
//! single global program cache, which is what the harness observes.
//!
//! [`Genesis`] builds the bank at slot 0, injects the cache contents, and rolls
//! forward to its starting slot. With `FINALITY_SLOTS` of 4 and a genesis slot
//! of 4:
//!
//! ```text
//! 0 ─ 1 ─ 2 ─ 3 ─ 4
//! ▲               ▲
//! root            canonical tip
//! ```
//!
//! [`Step::NewSlot`] adds a bank on top of any existing one, and marks whether
//! it becomes the canonical tip. A canonical slot moves the tip, and the root
//! advances to stay `FINALITY_SLOTS` behind it.
//!
//! ```text
//! 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
//!     ▲               ▲
//!     root            canonical tip
//! ```
//!
//! Branching off an older slot works the same way. Slots 4 and 5 still descend
//! from the root here, so they survive; entries are pruned once the root moves
//! onto a branch that excludes them.
//!
//! ```text
//! 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
//!             └── 6
//!         ▲       ▲
//!         root    canonical tip
//! ```
//!
//! A non-canonical slot leaves both alone, so a branch can be built without
//! rooting along it:
//!
//! ```text
//! 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
//!             └── 6
//!     ▲               ▲
//!     root            canonical tip
//! ```
//!
//! [`Step::Invoke`] runs one transaction per target against a named bank, in a
//! single batch. [`Step::Assert`] reads the cache back from the canonical tip
//! and compares it against what the timeline declares.

use {
    crate::{
        bank::{create_genesis_bank, process_transactions_and_assert_success},
        consts::{FINALITY_SLOTS, NATIVE_BUILTINS},
        effects::assert_cache_contents,
        entry::{Entry, EntryType, NoopBuiltin},
        genesis::Genesis,
        timeline::Step,
        transaction::invoke,
    },
    solana_leader_schedule::SlotLeader,
    solana_program_runtime::solana_sbpf::program::BuiltinFunctionDefinition,
    solana_runtime::{
        bank::{Bank, test_utils::goto_end_of_slot},
        bank_forks::BankForks,
    },
    std::sync::{Arc, RwLock},
};

/// The prepared test environment, built from [`Genesis`], ready to start
/// executing steps.
pub(crate) struct TestRuntime {
    bank_forks: Arc<RwLock<BankForks>>,
    /// The fork the harness roots against, declared by each `NewSlot` step.
    canonical_tip: u64,
}

impl TestRuntime {
    /// Build the environment described by `genesis`, rolling the initial fork
    /// forward to its starting slot.
    pub(crate) fn new_from_genesis(genesis: Genesis) -> Self {
        let (mut bank, _) = create_genesis_bank(&genesis.feature_set);

        let environment = bank
            .transaction_processor()
            .program_runtime_environment
            .clone();

        for entry in &genesis.cache_contents {
            if entry.ty == EntryType::Builtin {
                bank.add_mockup_builtin(entry.id, NoopBuiltin::register);
                continue;
            }
            for (address, account) in entry.accounts().into_iter().flatten() {
                bank.store_account(&address, &account);
            }
            if let Some(cache_entry) = entry.program_cache_entry(&environment) {
                bank.transaction_processor()
                    .global_program_cache
                    .write()
                    .unwrap()
                    .assign_program(&environment, entry.id, entry.slot, cache_entry);
            }
        }

        let (_, bank_forks) = bank.wrap_with_bank_forks_for_tests();

        let mut runtime = Self {
            bank_forks,
            canonical_tip: 0,
        };
        for slot in 1..=genesis.slot {
            runtime.new_slot(slot.saturating_sub(1), slot, true);
        }
        runtime
    }

    /// Run a single step.
    pub(crate) fn step(&mut self, step: Step) {
        match step {
            Step::NewSlot {
                parent,
                slot,
                canonical,
            } => self.new_slot(parent, slot, canonical),
            Step::Invoke { slot, targets } => {
                let bank = self.bank(slot);
                let transactions = targets.iter().map(|target| invoke(&bank, target)).collect();
                process_transactions_and_assert_success(&bank, transactions);
            }
            Step::Assert(expected) => {
                assert_cache_contents(&self.cache_contents(), &expected);
            }
        }
    }

    /// Create a bank at `slot` on top of `parent`, optionally making it the
    /// canonical tip.
    fn new_slot(&mut self, parent: u64, slot: u64, canonical: bool) {
        let parent_bank = self.bank(parent);
        if !parent_bank.is_frozen() {
            goto_end_of_slot(parent_bank.clone());
        }
        Bank::new_from_parent_with_bank_forks(
            &self.bank_forks,
            parent_bank,
            SlotLeader::default(),
            slot,
        );
        if canonical {
            self.canonical_tip = slot;
            self.maybe_root();
        }
    }

    /// The global program cache, as seen from the canonical tip.
    fn cache_contents(&self) -> Vec<Entry> {
        let bank = self.bank(self.canonical_tip);
        let cache = bank
            .transaction_processor()
            .global_program_cache
            .read()
            .unwrap();
        cache
            .get_flattened_entries_for_tests()
            .into_iter()
            .filter(|(id, _)| !NATIVE_BUILTINS.contains(id))
            .filter_map(|(id, entry)| Entry::from_program_cache_entry(id, &entry))
            .collect()
    }

    /// The bank at `slot`. Panics if no step ever created it.
    fn bank(&self, slot: u64) -> Arc<Bank> {
        self.bank_forks
            .read()
            .unwrap()
            .get(slot)
            .unwrap_or_else(|| panic!("no bank at slot {slot}"))
    }

    /// Advance the root to `FINALITY_SLOTS` behind the canonical tip, if the
    /// tip has moved far enough ahead of it.
    fn maybe_root(&mut self) {
        let root = self.canonical_tip.saturating_sub(FINALITY_SLOTS);
        if root <= self.bank_forks.read().unwrap().root() {
            return;
        }
        // Production prunes the program cache against the new root before the
        // root actually moves; see `votor::root_utils`.
        self.bank_forks.read().unwrap().prune_program_cache(root);
        let _ = self.bank_forks.write().unwrap().set_root(root, None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(slot: u64) -> TestRuntime {
        TestRuntime::new_from_genesis(Genesis::new_with_features_all_enabled(Vec::new(), slot))
    }

    fn root(runtime: &TestRuntime) -> u64 {
        runtime.bank_forks.read().unwrap().root()
    }

    #[test]
    fn genesis_rolls_forward_to_its_slot() {
        let runtime = runtime(4);
        assert_eq!(runtime.canonical_tip, 4);
        assert_eq!(root(&runtime), 0);
        // `bank` panics on a slot the fork graph never built.
        (0..=4).for_each(|slot| {
            runtime.bank(slot);
        });
    }

    #[test]
    fn canonical_slot_drags_the_root_along() {
        let mut runtime = runtime(4);
        runtime.new_slot(4, 5, true);
        assert_eq!(runtime.canonical_tip, 5);
        assert_eq!(root(&runtime), 1);
    }

    #[test]
    fn non_canonical_slot_moves_neither_tip_nor_root() {
        let mut runtime = runtime(4);
        runtime.new_slot(4, 5, true);
        runtime.new_slot(3, 6, false);
        assert_eq!(runtime.canonical_tip, 5);
        assert_eq!(root(&runtime), 1);
    }

    #[test]
    fn canonical_branch_roots_along_the_new_fork() {
        let mut runtime = runtime(4);
        runtime.new_slot(4, 5, true);
        runtime.new_slot(3, 7, true);
        assert_eq!(runtime.canonical_tip, 7);
        assert_eq!(root(&runtime), 3);
    }

    #[test]
    fn branches_off_an_already_frozen_parent() {
        let mut runtime = runtime(4);
        // Freezes slot 4 on the way to building slot 5.
        runtime.new_slot(4, 5, true);
        runtime.new_slot(4, 6, false);
        runtime.bank(6);
    }
}
