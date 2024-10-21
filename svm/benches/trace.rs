#[path = "../tests/mock_rollup.rs"]
mod mock_rollup;

use {
    criterion::{criterion_group, criterion_main, Criterion},
    mock_rollup::{
        mock_bank::{
            create_executable_environment, register_builtins, MockBankCallback, MockForkGraph,
        },
        MockRollup, TraceHandler,
    },
    solana_sdk::{
        account::AccountSharedData,
        instruction::AccountMeta,
        keccak::Hasher,
        pubkey::Pubkey,
        signature::Keypair,
        signer::Signer,
        system_instruction, system_program,
        transaction::{SanitizedTransaction, Transaction},
    },
    solana_svm::{
        account_loader::{CheckedTransactionDetails, TransactionCheckResult},
        transaction_processor::{
            ExecutionRecordingConfig, TransactionBatchProcessor, TransactionProcessingConfig,
            TransactionProcessingEnvironment,
        },
    },
    solana_svm_trace::{
        receipt::{hash_receipt, SVMTransactionReceipt},
        stf::{hash_trace, STFTrace},
        trie::Trie,
    },
    solana_svm_transaction::svm_transaction::SVMTransaction,
    solana_type_overrides::sync::{Arc, RwLock},
    std::collections::HashSet,
};

#[derive(Default)]
struct NoOp;
impl TraceHandler for NoOp {
    fn digest_transaction(&self, _transaction: &impl SVMTransaction) {}
    fn digest_receipt(&self, _transaction: &impl SVMTransaction, _receipt: &SVMTransactionReceipt) {
    }
    fn digest_trace(&self, _trace: &STFTrace<impl SVMTransaction>) {}
}

#[derive(Default)]
struct TransactionInclusionHandler {
    transactions_trie: RwLock<Trie>,
}
impl TraceHandler for TransactionInclusionHandler {
    fn digest_transaction(&self, transaction: &impl SVMTransaction) {
        // For benching purposes, just hash the signature.
        let hash_fn = |hasher: &mut Hasher| hasher.hash(transaction.signature().as_ref());
        self.transactions_trie.write().unwrap().append(&hash_fn);
    }

    fn digest_receipt(&self, _transaction: &impl SVMTransaction, _receipt: &SVMTransactionReceipt) {
    }
    fn digest_trace(&self, _trace: &STFTrace<impl SVMTransaction>) {}
}

#[derive(Default)]
struct TransactionReceiptHandler {
    receipts_trie: RwLock<Trie>,
}
impl TraceHandler for TransactionReceiptHandler {
    fn digest_transaction(&self, _transaction: &impl SVMTransaction) {}

    fn digest_receipt(&self, transaction: &impl SVMTransaction, receipt: &SVMTransactionReceipt) {
        let hash_fn = |hasher: &mut Hasher| {
            hasher.hash(transaction.signature().as_ref());
            hash_receipt(hasher, receipt);
        };
        self.receipts_trie.write().unwrap().append(&hash_fn);
    }

    fn digest_trace(&self, _trace: &STFTrace<impl SVMTransaction>) {}
}

#[derive(Default)]
struct TransactionSTFTraceHandler {
    traces_trie: RwLock<Trie>,
}
impl TraceHandler for TransactionSTFTraceHandler {
    fn digest_transaction(&self, _transaction: &impl SVMTransaction) {}
    fn digest_receipt(&self, _transaction: &impl SVMTransaction, _receipt: &SVMTransactionReceipt) {
    }

    fn digest_trace(&self, trace: &STFTrace<impl SVMTransaction>) {
        // For benching purposes, just hash it.
        let hash_fn = |hasher: &mut Hasher| {
            hash_trace(hasher, trace);
        };
        self.traces_trie.write().unwrap().append(&hash_fn);
    }
}

const NUM_RANDOM_ACCOUNT_KEYS: usize = 12;

fn create_transactions(count: usize, banks: &[&MockBankCallback]) -> Vec<SanitizedTransaction> {
    let mut accounts_to_store = vec![];

    let payer = Keypair::new();
    let payer_account = AccountSharedData::new(100_000_000_000, 0, &system_program::id());
    accounts_to_store.push((payer.pubkey(), payer_account));

    let txs = (0..count)
        .map(|_| {
            let destination = Pubkey::new_unique();
            let destination_account = AccountSharedData::default();
            accounts_to_store.push((destination, destination_account));

            let random_accounts = (0..NUM_RANDOM_ACCOUNT_KEYS)
                .map(|_| (Pubkey::new_unique(), AccountSharedData::default()))
                .collect::<Vec<_>>();
            let random_account_metas = random_accounts.iter().map(|(pubkey, _)| AccountMeta {
                pubkey: *pubkey,
                is_signer: false,
                is_writable: false,
            });
            accounts_to_store.extend_from_slice(&random_accounts);

            let mut ix = system_instruction::transfer(&payer.pubkey(), &destination, 100);
            ix.accounts.extend(random_account_metas);

            let tx = Transaction::new_signed_with_payer(
                &[ix],
                Some(&payer.pubkey()),
                &[&payer],
                solana_sdk::hash::Hash::default(),
            );
            SanitizedTransaction::from_transaction_for_tests(tx)
        })
        .collect();

    banks.iter().for_each(|b| {
        let mut account_store = b.account_shared_data.write().unwrap();
        accounts_to_store.iter().for_each(|(pubkey, account)| {
            account_store.insert(*pubkey, account.clone());
        });
    });

    txs
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
    let batch_processor = TransactionBatchProcessor::<MockForkGraph>::new_uninitialized(
        /* slot */ 0, /* epoch */ 0,
    );
    create_executable_environment(
        fork_graph.clone(),
        &mock_bank,
        &mut batch_processor.program_cache.write().unwrap(),
    );
    register_builtins(&mock_bank, &batch_processor);
    batch_processor
}

fn trace(c: &mut Criterion) {
    let rollup_noop = MockRollup::<NoOp>::default();
    let rollup_with_transaction_inclusion_handler =
        MockRollup::<TransactionInclusionHandler>::default();
    let rollup_with_transaction_receipt_handler =
        MockRollup::<TransactionReceiptHandler>::default();
    let rollup_with_transaction_stf_trace_handler =
        MockRollup::<TransactionSTFTraceHandler>::default();

    let fork_graph = Arc::new(RwLock::new(MockForkGraph {}));
    let processing_environment = TransactionProcessingEnvironment::default();
    let processing_config = TransactionProcessingConfig {
        recording_config: ExecutionRecordingConfig {
            enable_log_recording: true,         // Record logs
            enable_return_data_recording: true, // Record return data
            enable_cpi_recording: false,        // Don't care about inner instructions.
        },
        ..Default::default()
    };

    // Bench test against a few transaction sets (empty, small, large, massive).
    let transaction_sets = vec![
        (
            "Empty",
            create_transactions(
                0,
                &[
                    rollup_noop.bank(),
                    rollup_with_transaction_inclusion_handler.bank(),
                    rollup_with_transaction_receipt_handler.bank(),
                    rollup_with_transaction_stf_trace_handler.bank(),
                ],
            ),
        ),
        (
            "Small",
            create_transactions(
                10,
                &[
                    rollup_noop.bank(),
                    rollup_with_transaction_inclusion_handler.bank(),
                    rollup_with_transaction_receipt_handler.bank(),
                    rollup_with_transaction_stf_trace_handler.bank(),
                ],
            ),
        ),
        (
            "Large",
            create_transactions(
                1_000,
                &[
                    rollup_noop.bank(),
                    rollup_with_transaction_inclusion_handler.bank(),
                    rollup_with_transaction_receipt_handler.bank(),
                    rollup_with_transaction_stf_trace_handler.bank(),
                ],
            ),
        ),
        (
            "Massive",
            create_transactions(
                100_000,
                &[
                    rollup_noop.bank(),
                    rollup_with_transaction_inclusion_handler.bank(),
                    rollup_with_transaction_receipt_handler.bank(),
                    rollup_with_transaction_stf_trace_handler.bank(),
                ],
            ),
        ),
    ];
    let mut group = c.benchmark_group("SVM Trace Performance");

    for (set_name, transaction_set) in transaction_sets {
        let santitized_txs = &transaction_set as &[SanitizedTransaction];
        let check_results = create_check_results(santitized_txs.len());

        // Control.
        let batch_processor = setup_batch_processor(rollup_noop.bank(), &fork_graph);
        group.bench_function(format!("{} Transaction Batch: Control", set_name), |b| {
            b.iter(|| {
                batch_processor.load_and_execute_sanitized_transactions(
                    &rollup_noop, // No-Op handlers.
                    santitized_txs,
                    check_results.clone(),
                    &processing_environment,
                    &processing_config,
                )
            })
        });

        // With transaction hashing.
        let batch_processor = setup_batch_processor(
            rollup_with_transaction_inclusion_handler.bank(),
            &fork_graph,
        );
        group.bench_function(
            format!("{} Transaction Batch: With Transaction Hashing", set_name),
            |b| {
                b.iter(|| {
                    batch_processor.load_and_execute_sanitized_transactions(
                        &rollup_with_transaction_inclusion_handler, // Transaction hashing handlers.
                        santitized_txs,
                        check_results.clone(),
                        &processing_environment,
                        &processing_config,
                    )
                })
            },
        );

        // With receipt hashing.
        let batch_processor =
            setup_batch_processor(rollup_with_transaction_receipt_handler.bank(), &fork_graph);
        group.bench_function(
            format!("{} Transaction Batch: With Receipt Hashing", set_name),
            |b| {
                b.iter(|| {
                    batch_processor.load_and_execute_sanitized_transactions(
                        &rollup_with_transaction_receipt_handler, // Receipt hashing handlers.
                        santitized_txs,
                        check_results.clone(),
                        &processing_environment,
                        &processing_config,
                    )
                })
            },
        );

        // With STF trace hashing.
        let batch_processor = setup_batch_processor(
            rollup_with_transaction_stf_trace_handler.bank(),
            &fork_graph,
        );
        group.bench_function(
            format!("{} Transaction Batch: With STF Trace Hashing", set_name),
            |b| {
                b.iter(|| {
                    batch_processor.load_and_execute_sanitized_transactions(
                        &rollup_with_transaction_stf_trace_handler, // STF trace hashing handlers.
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
criterion_group!(benches, trace);
criterion_main!(benches);
