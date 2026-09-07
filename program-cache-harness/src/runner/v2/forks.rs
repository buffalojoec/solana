//! Actual production-grade fork graph using `BankForks`.

use {
    super::{account, keypair::keypair},
    crate::scenario::{ForkTree, SLOTS_PER_EPOCH},
    agave_feature_set::FeatureSet,
    solana_accounts_db::{
        accounts::Accounts, ancestors::Ancestors, blockhash_queue::BlockhashQueue,
    },
    solana_clock::{BankId, DEFAULT_TICKS_PER_SLOT, MAX_RECENT_BLOCKHASHES, Slot},
    solana_epoch_schedule::EpochSchedule,
    solana_fee_calculator::FeeRateGovernor,
    solana_hash::Hash,
    solana_keypair::Keypair,
    solana_leader_schedule::SlotLeader,
    solana_pubkey::Pubkey,
    solana_runtime::{
        bank::{Bank, BankFieldsToDeserialize, BankRc},
        bank_forks::BankForks,
        conformance::new_accounts_for_tests_single_threaded,
    },
    solana_signer::Signer as _,
    std::{
        collections::HashMap,
        sync::{Arc, RwLock},
    },
};

// Feature used to seed the environment switch.
const ENVIRONMENT_FEATURE: Pubkey = agave_feature_set::enable_sha512_syscall::ID;

pub struct Forks {
    bank_forks: Arc<RwLock<BankForks>>,
    tree: ForkTree,
    payer: Keypair,
}

impl Forks {
    pub fn new(tree: &ForkTree) -> Self {
        let accounts_db = Arc::clone(&new_accounts_for_tests_single_threaded().accounts_db);
        let payer = keypair(u8::MAX);

        let accounts = account::genesis(&payer.pubkey(), &ENVIRONMENT_FEATURE);
        let seed = Accounts::new(Arc::clone(&accounts_db));
        seed.store_accounts(
            (0, &accounts[..]),
            BankId::default(),
            None,
            &Ancestors::from(vec![0]),
        );
        accounts_db.add_root(0);

        let mut features = FeatureSet::all_enabled();
        // SIMD-0232 looks the leader's vote account up in epoch stakes when
        // fees are distributed at freeze; this harness has neither.
        features.deactivate(&agave_feature_set::custom_commission_collector::ID);
        features.deactivate(&ENVIRONMENT_FEATURE);

        let mut blockhash_queue = BlockhashQueue::new(MAX_RECENT_BLOCKHASHES);
        blockhash_queue.register_hash(&Hash::default(), 0);
        let fields = BankFieldsToDeserialize {
            slot: 0,
            parent_slot: 0,
            block_height: 0,
            blockhash_queue,
            tick_height: 0,
            max_tick_height: DEFAULT_TICKS_PER_SLOT,
            ticks_per_slot: DEFAULT_TICKS_PER_SLOT,
            fee_rate_governor: FeeRateGovernor::new(0, 0),
            // Uniform epochs.
            epoch_schedule: EpochSchedule::custom(SLOTS_PER_EPOCH, SLOTS_PER_EPOCH, false),
            ..BankFieldsToDeserialize::default()
        };
        let genesis = Bank::new_for_txn_tests(
            BankRc::new(Accounts::new(Arc::clone(&accounts_db))),
            fields,
            features,
            HashMap::default(),
        );
        let (_genesis, bank_forks) = genesis.wrap_with_bank_forks_for_tests();

        Self {
            bank_forks,
            tree: tree.clone(),
            payer,
        }
    }

    /// The bank at a slot, built on demand along with any ancestor it needs.
    pub fn bank(&self, slot: Slot) -> Option<Arc<Bank>> {
        if let Some(bank) = self.bank_forks.read().unwrap().get(slot) {
            return Some(bank);
        }
        if !self.tree.contains(slot) || slot <= self.root() {
            // Must be a slot in the tree, and can't be rooted already.
            return None;
        }
        let parent = self.bank(self.tree.parent(slot))?;
        let bank = Bank::new_from_parent(parent, SlotLeader::default(), slot);
        Some(
            self.bank_forks
                .write()
                .unwrap()
                .insert(bank)
                .clone_without_scheduler(),
        )
    }

    pub fn writable_bank(&self, slot: Slot) -> Option<Arc<Bank>> {
        self.bank(slot).filter(|bank| !bank.is_frozen())
    }

    pub fn contains(&self, slot: Slot) -> bool {
        self.bank(slot).is_some()
    }

    pub fn payer(&self) -> &Keypair {
        &self.payer
    }

    pub fn root(&self) -> Slot {
        self.bank_forks.read().unwrap().root()
    }

    pub fn root_bank(&self) -> Arc<Bank> {
        self.bank_forks.read().unwrap().root_bank()
    }

    pub fn set_root(&self, slot: Slot) {
        self.bank_forks.write().unwrap().set_root(slot, None, None);
    }

    pub fn clear(&self, slot: Slot) {
        self.bank_forks.write().unwrap().clear_bank(slot, false);
    }

    pub fn prune_program_cache(&self, slot: Slot) {
        let Some(bank) = self.bank(slot) else {
            return;
        };
        let bank_forks = self.bank_forks.read().unwrap();
        bank.prune_program_cache(&bank_forks);
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::scenario::{NUM_ENVIRONMENTS, SLOTS_PER_EPOCH, slots_in_new_epoch, tree},
        solana_program_runtime::loaded_programs::ProgramRuntimeEnvironment,
        solana_system_transaction as system_transaction,
    };

    fn environment(forks: &Forks, slot: Slot) -> ProgramRuntimeEnvironment {
        forks
            .bank(slot)
            .expect("a bank")
            .transaction_processor()
            .program_runtime_environment
            .clone()
    }

    #[test]
    fn a_bank_above_the_root_executes_a_transaction() {
        let forks = Forks::new(&tree(&[&[1, 2]]));
        let bank = forks.bank(2).expect("a bank at slot 2");
        let transaction = system_transaction::transfer(
            forks.payer(),
            &keypair(1).pubkey(),
            solana_rent::Rent::default().minimum_balance(0),
            bank.last_blockhash(),
        );
        assert_eq!(bank.process_transaction(&transaction), Ok(()));
    }

    #[test]
    fn a_short_tree_stays_inside_one_epoch() {
        let forks = Forks::new(&tree(&[&[1, 8, 15]]));
        assert_eq!(forks.bank(15).expect("a bank at slot 15").epoch(), 0);
        assert_eq!(
            *environment(&forks, 1),
            *environment(&forks, 15),
            "one environment for as long as the run stays in one epoch"
        );
    }

    #[test]
    fn a_tree_across_the_boundary_reaches_both_environments() {
        let below = SLOTS_PER_EPOCH - 1;
        let forks = Forks::new(&tree(&[&[
            below,
            slots_in_new_epoch(0),
            slots_in_new_epoch(1),
        ]]));
        let mut seen: Vec<ProgramRuntimeEnvironment> = Vec::new();
        for slot in [below, slots_in_new_epoch(0), slots_in_new_epoch(1)] {
            let env = environment(&forks, slot);
            if !seen.contains(&env) {
                seen.push(env);
            }
        }
        assert_eq!(seen.len(), usize::from(NUM_ENVIRONMENTS));
    }
}
