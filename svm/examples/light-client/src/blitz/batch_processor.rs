//! Wrapper around `TransactionBatchProcessor` to apply defaults, for
//! simplicity.

use {
    solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1,
    solana_compute_budget::compute_budget::ComputeBudget,
    solana_program_runtime::loaded_programs::{BlockRelation, ForkGraph, ProgramCacheEntry},
    solana_sdk::{
        clock::Slot,
        feature_set::FeatureSet,
        fee::FeeStructure,
        hash::Hash,
        rent_collector::RentCollector,
        transaction::{self, SanitizedTransaction},
    },
    solana_svm::{
        account_loader::CheckedTransactionDetails,
        transaction_processing_callback::TransactionProcessingCallback,
        transaction_processor::{
            LoadAndExecuteSanitizedTransactionsOutput, TransactionBatchProcessor,
            TransactionProcessingConfig, TransactionProcessingEnvironment,
        },
    },
    solana_system_program::system_processor,
    std::sync::{Arc, RwLock},
};

struct BlitzForkGraph {}

impl ForkGraph for BlitzForkGraph {
    fn relationship(&self, _a: Slot, _b: Slot) -> BlockRelation {
        BlockRelation::Unknown
    }
}

fn get_check_results(
    count: usize,
    lamports_per_signature: u64,
) -> Vec<transaction::Result<CheckedTransactionDetails>> {
    vec![
        transaction::Result::Ok(CheckedTransactionDetails {
            nonce: None,
            lamports_per_signature,
        });
        count
    ]
}

pub(crate) struct BlitzTransactionBatchProcessor {
    compute_budget: ComputeBudget,
    feature_set: Arc<FeatureSet>,
    fee_structure: FeeStructure,
    #[allow(unused)]
    fork_graph: Arc<RwLock<BlitzForkGraph>>,
    lamports_per_signature: u64,
    processor: TransactionBatchProcessor<BlitzForkGraph>,
    rent_collector: RentCollector,
}

impl BlitzTransactionBatchProcessor {
    pub(crate) fn new() -> Self {
        let compute_budget = ComputeBudget::default();
        let feature_set = FeatureSet::all_enabled();
        let fee_structure = FeeStructure::default();
        let fork_graph = Arc::new(RwLock::new(BlitzForkGraph {}));
        let lamports_per_signature = fee_structure.lamports_per_signature;
        let processor = TransactionBatchProcessor::<BlitzForkGraph>::default();
        let rent_collector = RentCollector::default();

        {
            let mut cache = processor.program_cache.write().unwrap();

            cache.fork_graph = Some(Arc::downgrade(&fork_graph));

            cache.environments.program_runtime_v1 = Arc::new(
                create_program_runtime_environment_v1(
                    &FeatureSet::default(),
                    &ComputeBudget::default(),
                    false,
                    false,
                )
                .unwrap(),
            );
        }

        Self {
            compute_budget,
            feature_set: Arc::new(feature_set),
            fee_structure,
            fork_graph,
            lamports_per_signature,
            processor,
            rent_collector,
        }
    }

    pub(crate) fn configure_builtins<CB: TransactionProcessingCallback>(&self, callbacks: &CB) {
        // Add the system program builtin.
        self.processor.add_builtin(
            callbacks,
            solana_system_program::id(),
            "system_program",
            ProgramCacheEntry::new_builtin(
                0,
                b"system_program".len(),
                system_processor::Entrypoint::vm,
            ),
        );
    }

    pub(crate) fn process_transaction_batch<CB: TransactionProcessingCallback>(
        &self,
        account_loader: &CB,
        batch: &[SanitizedTransaction],
    ) -> LoadAndExecuteSanitizedTransactionsOutput {
        self.processor.load_and_execute_sanitized_transactions(
            account_loader,
            batch,
            get_check_results(batch.len(), self.lamports_per_signature),
            &TransactionProcessingEnvironment {
                blockhash: Hash::default(),
                epoch_total_stake: None,
                epoch_vote_accounts: None,
                feature_set: Arc::clone(&self.feature_set),
                fee_structure: Some(&self.fee_structure),
                lamports_per_signature: self.lamports_per_signature,
                rent_collector: Some(&self.rent_collector),
            },
            &TransactionProcessingConfig {
                compute_budget: Some(self.compute_budget),
                ..Default::default()
            },
        )
    }
}
