//! Performance benchmarking SVM's STF feature.

#[path = "../tests/mock_bank.rs"]
mod mock_bank;

use {
    crate::mock_bank::{
        create_executable_environment, register_builtins, MockBankCallback, MockForkGraph,
    },
    criterion::{criterion_group, criterion_main, Criterion},
    solana_sdk::{
        instruction::AccountMeta,
        pubkey::Pubkey,
        signature::Keypair,
        signer::Signer,
        system_instruction,
        transaction::{SanitizedTransaction, Transaction},
    },
    solana_svm::{
        account_loader::{CheckedTransactionDetails, TransactionCheckResult},
        transaction_processor::{
            ExecutionRecordingConfig, TransactionBatchProcessor, TransactionProcessingConfig,
            TransactionProcessingEnvironment,
        },
    },
    solana_type_overrides::sync::{Arc, RwLock},
    std::collections::HashSet,
};

const NUM_RANDOM_ACCOUNT_KEYS: usize = 12;

fn create_transactions(count: usize) -> Vec<SanitizedTransaction> {
    let alice = Keypair::new();
    let bob = Pubkey::new_unique();
    (0..count)
        .map(|_| {
            let mut ix = system_instruction::transfer(&alice.pubkey(), &bob, 100);
            ix.accounts
                .extend((0..NUM_RANDOM_ACCOUNT_KEYS).map(|_| AccountMeta {
                    pubkey: Pubkey::new_unique(),
                    is_signer: false,
                    is_writable: false,
                }));
            let tx = Transaction::new_signed_with_payer(
                &[ix],
                Some(&alice.pubkey()),
                &[&alice],
                solana_sdk::hash::Hash::default(),
            );
            SanitizedTransaction::from_transaction_for_tests(tx)
        })
        .collect()
}

fn create_check_results(count: usize) -> Vec<TransactionCheckResult> {
    (0..count)
        .map(|_| {
            TransactionCheckResult::Ok(CheckedTransactionDetails {
                nonce: None,
                lamports_per_signature: 0,
            })
        })
        .collect()
}

fn setup_batch_processor(
    mock_bank: &MockBankCallback,
    fork_graph: &Arc<RwLock<MockForkGraph>>,
) -> TransactionBatchProcessor<MockForkGraph> {
    let batch_processor = TransactionBatchProcessor::<MockForkGraph>::new(
        /* slot */ 0,
        /* epoch */ 0,
        HashSet::new(),
    );
    create_executable_environment(
        fork_graph.clone(),
        &mock_bank,
        &mut batch_processor.program_cache.write().unwrap(),
    );
    batch_processor.fill_missing_sysvar_cache_entries(mock_bank);
    register_builtins(&mock_bank, &batch_processor);
    batch_processor
}

fn stf_bench(c: &mut Criterion) {
    let mock_bank = MockBankCallback::default();
    let fork_graph = Arc::new(RwLock::new(MockForkGraph {}));
    let batch_processor = setup_batch_processor(&mock_bank, &fork_graph);

    let processing_environment = TransactionProcessingEnvironment::default();
    let mut processing_config = TransactionProcessingConfig {
        recording_config: ExecutionRecordingConfig {
            enable_log_recording: true, // Record logs, so hash them when STF is enabled.
            enable_return_data_recording: true, // Record return data, so hash it when STF is enabled.
            enable_cpi_recording: false,        // Don't care about inner instructions.
        },
        ..Default::default()
    };

    // Bench test against a few transaction sets (empty, small, large, massive).
    let transaction_sets = vec![
        ("Empty", create_transactions(0)),
        ("Small", create_transactions(10)),
        ("Large", create_transactions(1_000)),
        ("Massive", create_transactions(100_000)),
    ];
    let mut group = c.benchmark_group("SVM Performance");

    for (set_name, transaction_set) in transaction_sets {
        let santitized_txs = &transaction_set as &[SanitizedTransaction];
        let check_results = create_check_results(santitized_txs.len());

        // Without STF.
        processing_config.enable_stf = false; // Explicitly disabled for readibility.
        processing_config.enable_receipts = false; // Explicitly disabled for readibility.
        group.bench_function(
            format!("{} Transaction Batch: Without STF", set_name),
            |b| {
                b.iter(|| {
                    batch_processor.load_and_execute_sanitized_transactions(
                        &mock_bank,
                        santitized_txs,
                        check_results.clone(),
                        &processing_environment,
                        &processing_config,
                    )
                })
            },
        );

        // With STF.
        processing_config.enable_stf = true;
        group.bench_function(
            format!("{} Transaction Batch: With Just STF", set_name),
            |b| {
                b.iter(|| {
                    batch_processor.load_and_execute_sanitized_transactions(
                        &mock_bank,
                        santitized_txs,
                        check_results.clone(),
                        &processing_environment,
                        &processing_config,
                    )
                })
            },
        );

        // With STF and traces.
        processing_config.enable_receipts = true;
        group.bench_function(
            format!("{} Transaction Batch: With STF and Traces", set_name),
            |b| {
                b.iter(|| {
                    batch_processor.load_and_execute_sanitized_transactions(
                        &mock_bank,
                        santitized_txs,
                        check_results.clone(),
                        &processing_environment,
                        &processing_config,
                    )
                })
            },
        );
    }

    group.finish();
}

// Criterion main.
criterion_group!(benches, stf_bench);
criterion_main!(benches);
