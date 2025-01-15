//! Solana SVM fuzz harness transaction entrypoint.
//!
//! This entrypoint provides an API for Agave's transaction processing pipeline
//! within SVM.
//!
//! It is primarily used by the fuzz harness, located in this same parent
//! directory, however it can also be used standalone for other testing tools,
//! such as those which test BPF programs.

use {
    solana_account::{Account, AccountSharedData},
    solana_accounts_db::{
        accounts_db::AccountsDbConfig,
        accounts_file::StorageAccess,
        accounts_index::{AccountsIndexConfig, IndexLimitMb},
    },
    solana_clock::MAX_PROCESSING_AGE,
    solana_feature_set::FeatureSet,
    solana_hash::Hash,
    solana_pubkey::Pubkey,
    solana_runtime::{bank::Bank, bank_forks::BankForks},
    solana_sdk::{
        genesis_config::GenesisConfig,
        transaction::{
            Result as TransactionResult, SanitizedTransaction, TransactionVerificationMode,
            VersionedTransaction,
        },
    },
    solana_svm::{
        runtime_config::RuntimeConfig,
        transaction_error_metrics::TransactionErrorMetrics,
        transaction_processing_result::ProcessedTransaction,
        transaction_processor::{ExecutionRecordingConfig, TransactionProcessingConfig},
    },
    solana_timings::ExecuteTimings,
    std::{
        num::NonZeroUsize,
        sync::{atomic::AtomicBool, Arc},
    },
};

/// Contextual values for the execution environment.
pub struct EnvironmentContext<'a> {
    accounts_to_remove_from_bank: &'a [Pubkey],
    blockhash_queue: &'a [Hash],
    feature_set: &'a FeatureSet,
    fee_collector: &'a Pubkey,
    genesis_config: &'a GenesisConfig,
    genesis_hash: Option<Hash>,
    slot: u64,
}

impl<'a> EnvironmentContext<'a> {
    pub fn new(
        accounts_to_remove_from_bank: &'a [Pubkey],
        blockhash_queue: &'a [Hash],
        feature_set: &'a FeatureSet,
        fee_collector: &'a Pubkey,
        genesis_config: &'a GenesisConfig,
        genesis_hash: Option<Hash>,
        slot: u64,
    ) -> Self {
        Self {
            accounts_to_remove_from_bank,
            blockhash_queue,
            feature_set,
            fee_collector,
            genesis_config,
            genesis_hash,
            slot,
        }
    }
}

#[allow(deprecated)]
pub fn process_transaction(
    accounts: &[(Pubkey, Account)],
    transaction: VersionedTransaction,
    environment_context: EnvironmentContext,
) -> TransactionResult<(SanitizedTransaction, ProcessedTransaction)> {
    let slot = environment_context.slot;

    // Bank on slot 0.
    let index = Some(AccountsIndexConfig {
        bins: Some(2),
        num_flush_threads: Some(NonZeroUsize::new(1).unwrap()),
        index_limit_mb: IndexLimitMb::InMemOnly,
        ..AccountsIndexConfig::default()
    });
    let accounts_db_config = Some(AccountsDbConfig {
        index,
        storage_access: StorageAccess::File,
        skip_initial_hash_calc: true,
        ..AccountsDbConfig::default()
    });
    let bank = Bank::new_with_paths(
        environment_context.genesis_config,
        Arc::new(RuntimeConfig::default()),
        vec![],
        None,
        None,
        false,
        accounts_db_config,
        None,
        Some(*environment_context.fee_collector),
        Arc::new(AtomicBool::new(false)),
        environment_context.genesis_hash,
        Some(environment_context.feature_set.clone()),
    );
    let bank_forks = BankForks::new_rw_arc(bank);
    let mut bank = bank_forks.read().unwrap().root_bank();
    bank.rehash();

    if slot > 0 {
        let new_bank = Bank::new_from_parent(bank.clone(), environment_context.fee_collector, slot);
        bank = bank_forks
            .write()
            .unwrap()
            .insert(new_bank)
            .clone_without_scheduler();
        bank.get_transaction_processor()
            .program_cache
            .write()
            .unwrap()
            .prune(slot, bank.epoch());
    }

    environment_context
        .accounts_to_remove_from_bank
        .iter()
        .for_each(|pubkey| bank.store_account(pubkey, &AccountSharedData::default()));

    bank.get_transaction_processor().reset_sysvar_cache();
    for (pubkey, account) in accounts {
        bank.store_account(pubkey, &AccountSharedData::from(account.clone()));
    }
    bank.get_transaction_processor()
        .fill_missing_sysvar_cache_entries(bank.as_ref());

    // Update rent and epoch schedule sysvar accounts to the minimum rent
    // exempt balance.
    bank.update_epoch_schedule();
    bank.update_rent();

    let sysvar_recent_blockhashes = bank.get_sysvar_cache_for_tests().get_recent_blockhashes();
    let mut lamports_per_signature: Option<u64> = None;
    if let Ok(recent_blockhashes) = &sysvar_recent_blockhashes {
        if let Some(hash) = recent_blockhashes.first() {
            if hash.fee_calculator.lamports_per_signature != 0 {
                lamports_per_signature = Some(hash.fee_calculator.lamports_per_signature);
            }
        }
    }

    // Register blockhashes in bank.
    for blockhash in environment_context.blockhash_queue.iter() {
        bank.register_recent_blockhash_for_test(blockhash, lamports_per_signature);
    }
    bank.update_recent_blockhashes();
    bank.get_transaction_processor().reset_sysvar_cache();
    bank.get_transaction_processor()
        .fill_missing_sysvar_cache_entries(bank.as_ref());

    let sanitized_transaction = bank.verify_transaction(
        transaction,
        TransactionVerificationMode::HashAndVerifyPrecompiles,
    )?;

    let transactions = [sanitized_transaction.clone()];

    let batch = bank.prepare_sanitized_batch(&transactions);

    let recording_config = ExecutionRecordingConfig {
        enable_cpi_recording: false,
        enable_log_recording: true,
        enable_return_data_recording: true,
    };

    let mut metrics = TransactionErrorMetrics::default();
    let mut timings = ExecuteTimings::default();

    let configs = TransactionProcessingConfig {
        account_overrides: None,
        compute_budget: bank.compute_budget(),
        log_messages_bytes_limit: None,
        limit_to_load_programs: true,
        recording_config,
        transaction_account_lock_limit: None,
        check_program_modification_slot: false,
    };

    bank.load_and_execute_transactions(
        &batch,
        MAX_PROCESSING_AGE,
        &mut timings,
        &mut metrics,
        configs,
    )
    .processing_results
    .pop()
    .unwrap()
    .map(|processed_transaction| (sanitized_transaction, processed_transaction))
}
