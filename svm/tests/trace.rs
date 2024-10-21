mod mock_rollup;

use {
    mock_rollup::{
        mock_bank::{
            create_executable_environment, register_builtins, MockBankCallback, MockForkGraph,
        },
        MockRollup, TraceHandler,
    },
    solana_program_runtime::loaded_programs::ProgramCacheEntry,
    solana_sdk::{
        account::AccountSharedData,
        instruction::{AccountMeta, Instruction},
        keccak::Hasher,
        pubkey::Pubkey,
        signature::{Keypair, Signature},
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
        stf::{hash_account, hash_environment, hash_transaction, STFEnvironment, STFTrace},
        trie::Trie,
    },
    solana_svm_transaction::svm_transaction::SVMTransaction,
    solana_type_overrides::sync::{Arc, RwLock},
    std::collections::HashSet,
};

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
        mock_bank,
        &mut batch_processor.program_cache.write().unwrap(),
    );
    register_builtins(mock_bank, &batch_processor);
    batch_processor
}

fn register_compute_budget_builtin(
    mock_bank: &MockBankCallback,
    batch_processor: &TransactionBatchProcessor<MockForkGraph>,
) {
    const DEPLOYMENT_SLOT: u64 = 0;
    let compute_budget_name = "solana_compute_budget_program";
    batch_processor.add_builtin(
        mock_bank,
        solana_sdk::compute_budget::id(),
        compute_budget_name,
        ProgramCacheEntry::new_builtin(
            DEPLOYMENT_SLOT,
            compute_budget_name.len(),
            solana_compute_budget_program::Entrypoint::vm,
        ),
    );
}

#[test]
fn test_processed_transactions() {
    // Our handler here is simply going to track transaction signatures.
    #[derive(Default)]
    struct TestHandler {
        seen_signatures_from_digested_transactions: RwLock<HashSet<Signature>>,
        seen_signatures_from_digested_receipts: RwLock<HashSet<Signature>>,
        seen_signatures_from_digested_traces: RwLock<HashSet<Signature>>,
    }
    impl TraceHandler for TestHandler {
        fn digest_transaction(&self, transaction: &impl SVMTransaction) {
            // If the callback was invoked, store the transaction signature.
            self.seen_signatures_from_digested_transactions
                .write()
                .unwrap()
                .insert(*transaction.signature());
        }

        fn digest_receipt(
            &self,
            transaction: &impl SVMTransaction,
            _receipt: &SVMTransactionReceipt,
        ) {
            // If the callback was invoked, store the transaction signature.
            self.seen_signatures_from_digested_receipts
                .write()
                .unwrap()
                .insert(*transaction.signature());
        }

        fn digest_trace(&self, trace: &STFTrace<impl SVMTransaction>) {
            // If the callback was invoked, store the transaction signature.
            if let STFTrace::Directive(directive) = trace {
                self.seen_signatures_from_digested_traces
                    .write()
                    .unwrap()
                    .insert(*directive.transaction.signature());
            }
        }
    }

    let rollup = MockRollup::<TestHandler>::default();
    let fork_graph = Arc::new(RwLock::new(MockForkGraph {}));
    let batch_processor = setup_batch_processor(rollup.bank(), &fork_graph);

    let processing_environment = TransactionProcessingEnvironment {
        rent_collector: Some(rollup.rent_collector()),
        ..Default::default()
    };
    let processing_config = TransactionProcessingConfig {
        recording_config: ExecutionRecordingConfig {
            enable_log_recording: true,         // Record logs
            enable_return_data_recording: true, // Record return data
            enable_cpi_recording: false,        // Don't care about inner instructions.
        },
        ..Default::default()
    };

    // Set up Alice's account to have enough lamports for transfer and fees.
    let alice = Keypair::new();
    rollup.add_rent_exempt_account(&alice.pubkey(), &[], &system_program::id(), 100_000_000);

    // Don't set up Bob's account.
    let bob = Keypair::new();

    // Set up another payer - Carol - who can attempt a transfer to Bob.
    let carol = Keypair::new();
    rollup.add_rent_exempt_account(&carol.pubkey(), &[], &system_program::id(), 80_000_000);

    // Set up an account with an unknown owner.
    let account_with_unknown_owner = Pubkey::new_unique();
    rollup.add_rent_exempt_account(&account_with_unknown_owner, &[], &Pubkey::new_unique(), 0);

    let sanitized_txs = [
        // The first transaction should succeed.
        // Alice has enough lamports for the transfer and fee.
        Transaction::new_signed_with_payer(
            &[system_instruction::transfer(
                &alice.pubkey(),
                &bob.pubkey(),
                80_000_000,
            )],
            Some(&alice.pubkey()),
            &[&alice],
            solana_sdk::hash::Hash::default(),
        ),
        // The second transaction should execute but fail with an error.
        // Carol would no longer be rent-exempt after the transfer.
        Transaction::new_signed_with_payer(
            &[system_instruction::transfer(
                &carol.pubkey(),
                &bob.pubkey(),
                80_001_000, // Carol has 80_000_000 lamports in excess.
            )],
            Some(&carol.pubkey()),
            &[&carol],
            solana_sdk::hash::Hash::default(),
        ),
        // The third transaction should fail to load, therefore it should not
        // execute.
        // The error is caused by the unknown owner.
        Transaction::new_signed_with_payer(
            &[Instruction::new_with_bytes(
                Pubkey::new_unique(),
                &[],
                vec![AccountMeta::new_readonly(account_with_unknown_owner, false)],
            )],
            Some(&alice.pubkey()), // Fee payer doesn't matter here. Alice has enough.
            &[&alice],
            solana_sdk::hash::Hash::default(),
        ),
    ]
    .into_iter()
    .map(SanitizedTransaction::from_transaction_for_tests)
    .collect::<Vec<_>>();

    // Invoke SVM.
    let results = batch_processor.load_and_execute_sanitized_transactions(
        &rollup,
        &sanitized_txs,
        create_check_results(sanitized_txs.len()),
        &processing_environment,
        &processing_config,
    );

    // The first transaction should have been successful and we should have
    // gotten a signature and a receipt.
    let result = results.processing_results[0].as_ref().unwrap();
    assert!(result.execution_details().unwrap().was_successful());
    assert!(rollup
        .trace_handler()
        .seen_signatures_from_digested_transactions
        .read()
        .unwrap()
        .contains(sanitized_txs[0].signature()));
    assert!(rollup
        .trace_handler()
        .seen_signatures_from_digested_receipts
        .read()
        .unwrap()
        .contains(sanitized_txs[0].signature()));
    assert!(rollup
        .trace_handler()
        .seen_signatures_from_digested_traces
        .read()
        .unwrap()
        .contains(sanitized_txs[0].signature()));

    // The second transaction should have executed but failed with an error.
    // We should still have gotten a signature and a receipt.
    let result = results.processing_results[1].as_ref().unwrap();
    assert!(!result.execution_details().unwrap().was_successful());
    assert!(rollup
        .trace_handler()
        .seen_signatures_from_digested_transactions
        .read()
        .unwrap()
        .contains(sanitized_txs[1].signature()));
    assert!(rollup
        .trace_handler()
        .seen_signatures_from_digested_receipts
        .read()
        .unwrap()
        .contains(sanitized_txs[1].signature()));
    assert!(rollup
        .trace_handler()
        .seen_signatures_from_digested_traces
        .read()
        .unwrap()
        .contains(sanitized_txs[1].signature()));

    // The third transaction should have failed to load and should not have
    // given us a signature or a receipt.
    let result = &results.processing_results[2];
    assert!(result.is_err());
    assert!(!rollup
        .trace_handler()
        .seen_signatures_from_digested_transactions
        .read()
        .unwrap()
        .contains(sanitized_txs[2].signature()));
    assert!(!rollup
        .trace_handler()
        .seen_signatures_from_digested_receipts
        .read()
        .unwrap()
        .contains(sanitized_txs[2].signature()));
    assert!(!rollup
        .trace_handler()
        .seen_signatures_from_digested_traces
        .read()
        .unwrap()
        .contains(sanitized_txs[2].signature()));
}

#[test]
fn test_proofs() {
    // Our handler here is going to use the trie structure defined in
    // svm-trace to store various callback entries in Merkle trees.
    #[derive(Default)]
    struct TestHandler {
        transactions_trie: RwLock<Trie>,
        receipts_trie: RwLock<Trie>,
        traces_trie: RwLock<Trie>,
        stf_hasher: RwLock<Hasher>,
        // This is cheating a bit, but we're stashing the pre-state for each
        // transaction, just for test purposes.
        pub pre_state_accounts: RwLock<Vec<Vec<(Pubkey, AccountSharedData)>>>,
    }
    impl TraceHandler for TestHandler {
        fn digest_transaction(&self, transaction: &impl SVMTransaction) {
            let hash_fn = |hasher: &mut Hasher| {
                hasher.hash(transaction.signature().as_ref());
            };
            self.transactions_trie.write().unwrap().append(hash_fn);
        }

        fn digest_receipt(
            &self,
            transaction: &impl SVMTransaction,
            receipt: &SVMTransactionReceipt,
        ) {
            let hash_fn = |hasher: &mut Hasher| {
                hasher.hash(transaction.signature().as_ref());
                hash_receipt(hasher, receipt);
            };
            self.receipts_trie.write().unwrap().append(hash_fn);
        }

        fn digest_trace(&self, trace: &STFTrace<impl SVMTransaction>) {
            let stf_hasher = &mut *self.stf_hasher.write().unwrap();
            match trace {
                STFTrace::State(state) => {
                    let mut pre_state_accounts = vec![];
                    // Right before hashing the pre-state, hash the signature.
                    for (pubkey, account) in state.accounts {
                        hash_account(stf_hasher, pubkey, account);
                        pre_state_accounts.push((*pubkey, account.clone()));
                    }
                    self.pre_state_accounts
                        .write()
                        .unwrap()
                        .push(pre_state_accounts);
                }
                STFTrace::Directive(directive) => {
                    hash_environment(stf_hasher, directive.environment);
                    hash_transaction(stf_hasher, directive.transaction);
                }
                STFTrace::NewState(state) => {
                    for (pubkey, account) in state.accounts {
                        hash_account(stf_hasher, pubkey, account);
                    }
                    // Now that we've hashed the post-state, we can fold this
                    // node into the tree.
                    self.traces_trie
                        .write()
                        .unwrap()
                        .push(stf_hasher.result_reset());
                }
            }
        }
    }

    let rollup = MockRollup::<TestHandler>::default();
    let fork_graph = Arc::new(RwLock::new(MockForkGraph {}));
    let batch_processor = setup_batch_processor(rollup.bank(), &fork_graph);
    register_compute_budget_builtin(rollup.bank(), &batch_processor);

    let processing_environment = TransactionProcessingEnvironment {
        rent_collector: Some(rollup.rent_collector()),
        ..Default::default()
    };
    let processing_config = TransactionProcessingConfig {
        recording_config: ExecutionRecordingConfig {
            enable_log_recording: true,         // Record logs
            enable_return_data_recording: true, // Record return data
            enable_cpi_recording: false,        // Don't care about inner instructions.
        },
        ..Default::default()
    };

    // We want a few different transactions so things like CUs and logs are
    // different, but Alice is going to pay for all of them.
    let alice = Keypair::new();
    rollup.add_rent_exempt_account(
        &alice.pubkey(),
        &[],
        &system_program::id(),
        100_000_000_000_000,
    );
    let account_to_create = Keypair::new();
    let account_with_no_funds = Keypair::new();

    let sanitized_txs = [
        Transaction::new_signed_with_payer(
            &[system_instruction::create_account(
                &alice.pubkey(),
                &account_to_create.pubkey(),
                20_000_000,
                0,
                &system_program::id(),
            )],
            Some(&alice.pubkey()),
            &[&alice, &account_to_create],
            solana_sdk::hash::Hash::default(),
        ),
        Transaction::new_signed_with_payer(
            &[system_instruction::transfer(
                &account_with_no_funds.pubkey(),
                &Pubkey::new_unique(),
                80_000_000,
            )],
            Some(&alice.pubkey()),
            &[&alice, &account_with_no_funds],
            solana_sdk::hash::Hash::default(),
        ),
        Transaction::new_signed_with_payer(
            &[
                solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(
                    200_000_000,
                ),
                system_instruction::transfer(&alice.pubkey(), &Pubkey::new_unique(), 80_000_000),
            ],
            Some(&alice.pubkey()),
            &[&alice],
            solana_sdk::hash::Hash::default(),
        ),
    ]
    .into_iter()
    .map(SanitizedTransaction::from_transaction_for_tests)
    .collect::<Vec<_>>();

    // Invoke SVM.
    let result = batch_processor.load_and_execute_sanitized_transactions(
        &rollup,
        &sanitized_txs,
        create_check_results(sanitized_txs.len()),
        &processing_environment,
        &processing_config,
    );

    // Merklize the tries.
    let transactions_tree = rollup
        .trace_handler()
        .transactions_trie
        .read()
        .unwrap()
        .merklize();
    let receipts_tree = rollup
        .trace_handler()
        .receipts_trie
        .read()
        .unwrap()
        .merklize();
    let traces_tree = rollup
        .trace_handler()
        .traces_trie
        .read()
        .unwrap()
        .merklize();

    // Verify the proofs.
    let mut hasher = solana_sdk::keccak::Hasher::default();
    for (i, res) in result.processing_results.iter().enumerate() {
        // Assert the transaction was processed.
        assert!(res.is_ok());
        let unwrapped_result = res.as_ref().unwrap();
        let execution_details = unwrapped_result.execution_details().unwrap();
        let loaded_transaction = &unwrapped_result
            .executed_transaction()
            .unwrap()
            .loaded_transaction;

        // Verify the proof on the transactions trie.
        let candidate = {
            // First hash the transaction entry manually, then with the leaf
            // prefix.
            hasher.hash(sanitized_txs[i].signature().as_ref());
            let raw_hash = hasher.result_reset();
            hasher.hashv(&[&[0], raw_hash.as_ref()]);
            hasher.result_reset()
        };
        let index = transactions_tree.get_leaf_index(&candidate).unwrap();
        let proof = transactions_tree.find_path(index).unwrap();
        assert!(
            proof.verify(candidate),
            "Failed to verify transaction inclusion proof"
        );

        // Verify the proof on the receipts trie.
        let candidate = {
            // First hash the receipt entry manually, then with the leaf
            // prefix.
            hasher.hash(sanitized_txs[i].signature().as_ref());
            hash_receipt(
                &mut hasher,
                &SVMTransactionReceipt {
                    compute_units_consumed: &execution_details.executed_units,
                    fee_details: &loaded_transaction.fee_details,
                    log_messages: execution_details.log_messages.as_ref(),
                    return_data: execution_details.return_data.as_ref(),
                    status: &execution_details.status,
                },
            );
            let raw_hash = hasher.result_reset();
            hasher.hashv(&[&[0], raw_hash.as_ref()]);
            hasher.result_reset()
        };
        let index = receipts_tree.get_leaf_index(&candidate).unwrap();
        let proof = receipts_tree.find_path(index).unwrap();
        assert!(proof.verify(candidate), "Failed to verify receipt proof");

        // Verify the proof on the traces trie.
        // Again, we're cheating a bit here for test purposes, since our hook
        // has been stashing transaction pre-state.
        let candidate = {
            // First hash the trace entry manually, then with the leaf prefix.
            for (pubkey, account) in &rollup.trace_handler().pre_state_accounts.read().unwrap()[i] {
                hash_account(&mut hasher, pubkey, account);
            }
            hash_environment(
                &mut hasher,
                &STFEnvironment {
                    feature_set: &processing_environment.feature_set,
                    fee_structure: processing_environment.fee_structure,
                    lamports_per_signature: &processing_environment.lamports_per_signature,
                    rent_collector: processing_environment.rent_collector,
                },
            );
            hash_transaction(&mut hasher, &sanitized_txs[i]);
            for (pubkey, account) in &loaded_transaction.accounts {
                hash_account(&mut hasher, pubkey, account);
            }
            let raw_hash = hasher.result_reset();
            hasher.hashv(&[&[0], raw_hash.as_ref()]);
            hasher.result_reset()
        };
        let index = traces_tree.get_leaf_index(&candidate).unwrap();
        let proof = traces_tree.find_path(index).unwrap();
        assert!(proof.verify(candidate), "Failed to verify STF proof");
    }
}
