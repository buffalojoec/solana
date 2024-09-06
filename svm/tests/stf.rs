//! Performance testing SVM's STF feature.

mod mock_bank;

use {
    crate::mock_bank::{
        create_executable_environment, register_builtins, MockBankCallback, MockForkGraph,
    },
    solana_sdk::{
        account::AccountSharedData,
        instruction::{AccountMeta, Instruction},
        pubkey::Pubkey,
        rent_collector::RentCollector,
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
    solana_type_overrides::sync::{Arc, RwLock},
    std::collections::HashSet,
};

struct Rollup {
    mock_bank: MockBankCallback,
    rent_collector: RentCollector,
}

impl Rollup {
    fn new() -> Self {
        Self {
            mock_bank: MockBankCallback::default(),
            rent_collector: RentCollector::default(),
        }
    }

    fn add_rent_exempt_account(
        &self,
        pubkey: &Pubkey,
        data: &[u8],
        owner: &Pubkey,
        excess_lamports: u64,
    ) {
        let space = data.len();
        let lamports = self
            .rent_collector
            .rent
            .minimum_balance(space)
            .saturating_add(excess_lamports);
        let mut account = AccountSharedData::new(lamports, space, owner);
        account.set_data_from_slice(data);
        self.mock_bank
            .account_shared_data
            .write()
            .unwrap()
            .insert(*pubkey, account);
    }

    fn callbacks(&self) -> &MockBankCallback {
        &self.mock_bank
    }
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

// Note: This test will need to be adjusted to handle fees-only transactions.
#[test]
fn test_processed_transactions() {
    let rollup = Rollup::new();
    let fork_graph = Arc::new(RwLock::new(MockForkGraph {}));
    let batch_processor = setup_batch_processor(rollup.callbacks(), &fork_graph);

    let processing_environment = TransactionProcessingEnvironment {
        rent_collector: Some(&rollup.rent_collector),
        ..Default::default()
    };
    let processing_config = TransactionProcessingConfig {
        recording_config: ExecutionRecordingConfig {
            enable_log_recording: true, // Record logs, so hash them when STF is enabled.
            enable_return_data_recording: true, // Record return data, so hash it when STF is enabled.
            enable_cpi_recording: false,        // Don't care about inner instructions.
        },
        enable_stf: true,      // Enable STF.
        enable_receipts: true, // Enable traces.
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
    .map(|tx| SanitizedTransaction::from_transaction_for_tests(tx))
    .collect::<Vec<_>>();

    let check_results = create_check_results(sanitized_txs.len());

    // Invoke SVM.
    let results = batch_processor.load_and_execute_sanitized_transactions(
        rollup.callbacks(),
        &sanitized_txs,
        check_results,
        &processing_environment,
        &processing_config,
    );

    // The first transaction should have been successful and it should have
    // valid STF and receipt traces.
    let result = results.processing_results[0].as_ref().unwrap();
    assert!(result.execution_details().unwrap().was_successful());
    assert!(result.stf().is_some());
    assert!(result.trace().is_some());

    // The second transaction should have executed but failed with an error.
    // It should also have valid STF and receipt traces.
    let result = results.processing_results[1].as_ref().unwrap();
    assert!(!result.execution_details().unwrap().was_successful());
    assert!(result.stf().is_some());
    assert!(result.trace().is_some());

    // The third transaction should have failed to load and should not have an
    // STF or receipt trace.
    let result = &results.processing_results[2];
    assert!(result.is_err());
}
