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
//! [`Step::Advance`] extends the canonical fork, hanging the new slot off the
//! current tip. It is the only step that moves the tip, and so the only one
//! that roots: once the fork is longer than `FINALITY_SLOTS`, the root follows
//! that many blocks behind it.
//!
//! ```text
//! 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
//!     ▲               ▲
//!     root            canonical tip
//! ```
//!
//! [`Step::NewSlotOn`] builds anywhere else, naming its own parent, and leaves
//! both the tip and the root alone. Starting a branch and extending one are the
//! same step — the second just names the first as its parent:
//!
//! ```text
//! 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
//!             └── 6 ─ 7
//!     ▲               ▲
//!     root            canonical tip
//! ```
//!
//! Nothing can move the canonical fork onto a branch, so the root is always a
//! slot the fork itself passed through. Counting blocks rather than subtracting
//! slot numbers keeps that true even when a timeline skips slots.
//!
//! [`Step::Invoke`] runs one transaction per target against a named bank, in a
//! single batch, and [`Step::Deploy`] does the same for deployments. Both take
//! the slot they run against, so either fork can execute. [`Step::Assert`]
//! reads the cache back from the canonical tip and compares it against what the
//! timeline declares.

use {
    crate::{
        bank::{create_genesis_bank, process_transactions_and_assert_success},
        consts::{FINALITY_SLOTS, NATIVE_BUILTINS},
        effects::{assert_cache_contents, assert_deployed, assert_served},
        entry::{Entry, EntryType, NoopBuiltin},
        genesis::Genesis,
        timeline::Step,
        transaction::{deploy, invoke},
    },
    solana_keypair::Keypair,
    solana_leader_schedule::SlotLeader,
    solana_program_runtime::solana_sbpf::program::BuiltinFunctionDefinition,
    solana_runtime::{
        bank::{Bank, test_utils::goto_end_of_slot},
        bank_forks::BankForks,
    },
    solana_signer::Signer,
    std::sync::{Arc, RwLock},
};

/// The prepared test environment, built from [`Genesis`], ready to start
/// executing steps.
pub(crate) struct TestRuntime {
    bank_forks: Arc<RwLock<BankForks>>,
    /// The canonical fork, oldest slot first. Only `Advance` extends it, so
    /// the root is always one of its own entries.
    canonical: Vec<u64>,
    /// Authority over every program the harness seeds or deploys, so that an
    /// upgrade has someone to sign for it.
    upgrade_authority: Keypair,
}

impl TestRuntime {
    /// Build the environment described by `genesis`, rolling the initial fork
    /// forward to its starting slot.
    pub(crate) fn new_from_genesis(genesis: Genesis) -> Self {
        let (mut bank, _) = create_genesis_bank(&genesis.feature_set);
        let upgrade_authority = Keypair::new();

        let environment = bank
            .transaction_processor()
            .program_runtime_environment
            .clone();

        for entry in &genesis.cache_contents {
            if entry.ty == EntryType::Builtin {
                bank.add_mockup_builtin(entry.id, NoopBuiltin::register);
                continue;
            }
            for (address, account) in entry
                .accounts(&upgrade_authority.pubkey())
                .into_iter()
                .flatten()
            {
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
            canonical: vec![0],
            upgrade_authority,
        };
        for slot in 1..=genesis.slot {
            runtime.advance(slot);
        }
        runtime
    }

    /// Run a single step.
    pub(crate) fn step(&mut self, step: Step) {
        match step {
            Step::Advance { slot } => self.advance(slot),
            Step::NewSlotOn { parent, slot } => self.new_slot_on(parent, slot),
            Step::Invoke {
                slot,
                targets,
                served,
            } => {
                let bank = self.bank(slot);
                let transactions = targets.iter().map(|target| invoke(&bank, target)).collect();
                let batch = process_transactions_and_assert_success(&bank, transactions);
                assert_served(&batch, &targets, &served);
            }
            Step::Deploy { slot, targets } => {
                let bank = self.bank(slot);
                let transactions = targets
                    .iter()
                    .map(|target| deploy(&bank, target, &self.upgrade_authority))
                    .collect();
                let batch = process_transactions_and_assert_success(&bank, transactions);
                assert_deployed(&batch, &targets);
            }
            Step::Assert(expected) => {
                assert_cache_contents(&self.cache_contents(), &expected);
            }
        }
    }

    /// Extend the canonical fork to `slot`.
    fn advance(&mut self, slot: u64) {
        self.new_slot_on(self.canonical_tip(), slot);
        self.canonical.push(slot);
        self.maybe_root();
    }

    /// Create a bank at `slot` on top of `parent`.
    fn new_slot_on(&mut self, parent: u64, slot: u64) {
        assert!(
            slot > parent,
            "slot {slot} does not follow its parent {parent}"
        );
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
    }

    /// The head of the canonical fork.
    fn canonical_tip(&self) -> u64 {
        *self
            .canonical
            .last()
            .expect("canonical fork is never empty")
    }

    /// The global program cache, as seen from the canonical tip.
    fn cache_contents(&self) -> Vec<Entry> {
        let bank = self.bank(self.canonical_tip());
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

    /// Advance the root to `FINALITY_SLOTS` blocks behind the canonical tip,
    /// once the fork has grown that long. Counting the fork rather than
    /// subtracting slot numbers keeps the root on it even across skipped slots.
    fn maybe_root(&mut self) {
        let Some(index) = self
            .canonical
            .len()
            .checked_sub(FINALITY_SLOTS as usize)
            .and_then(|index| index.checked_sub(1))
        else {
            return;
        };
        let root = self.canonical[index];
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
        assert_eq!(runtime.canonical_tip(), 4);
        assert_eq!(root(&runtime), 0);
        // `bank` panics on a slot the fork graph never built.
        (0..=4).for_each(|slot| {
            runtime.bank(slot);
        });
    }

    #[test]
    fn canonical_slot_drags_the_root_along() {
        let mut runtime = runtime(4);
        runtime.advance(5);
        assert_eq!(runtime.canonical_tip(), 5);
        assert_eq!(root(&runtime), 1);
    }

    #[test]
    fn non_canonical_slot_moves_neither_tip_nor_root() {
        let mut runtime = runtime(4);
        runtime.advance(5);
        runtime.new_slot_on(3, 6);
        assert_eq!(runtime.canonical_tip(), 5);
        assert_eq!(root(&runtime), 1);
    }

    #[test]
    fn a_branch_can_be_extended_without_becoming_canonical() {
        let mut runtime = runtime(4);
        runtime.advance(5);
        runtime.new_slot_on(4, 6);
        // Extending the branch is the same step, hung off its own tip.
        runtime.new_slot_on(6, 7);
        runtime.bank(7);
        assert_eq!(runtime.canonical_tip(), 5);
        assert_eq!(root(&runtime), 1);
    }

    #[test]
    fn rooting_counts_blocks_not_slot_numbers() {
        let mut runtime = runtime(4);
        // A skipped run of slots. Subtracting `FINALITY_SLOTS` here would name
        // slot 96, which no step ever built.
        runtime.advance(100);
        assert_eq!(runtime.canonical_tip(), 100);
        assert_eq!(root(&runtime), 1);
    }

    #[test]
    fn branches_off_an_already_frozen_parent() {
        let mut runtime = runtime(4);
        // Freezes slot 4 on the way to building slot 5.
        runtime.advance(5);
        runtime.new_slot_on(4, 6);
        runtime.bank(6);
    }
}
