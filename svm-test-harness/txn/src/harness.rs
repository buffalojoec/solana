//! Transaction harness.

use {
    agave_feature_set::{raise_cpi_nesting_limit_to_8, FeatureSet},
    agave_precompiles::{get_precompile, is_precompile},
    agave_syscalls::create_program_runtime_environment_v1,
    solana_account::{Account, AccountSharedData},
    solana_builtins::BUILTINS,
    solana_clock::Slot,
    solana_compute_budget::compute_budget::ComputeBudget,
    solana_compute_budget_instruction::instructions_processor::process_compute_budget_instructions,
    solana_fee_structure::FeeDetails,
    solana_precompile_error::PrecompileError,
    solana_program_runtime::loaded_programs::{
        BlockRelation, ForkGraph, ProgramCache, ProgramCacheEntry, ProgramRuntimeEnvironments,
    },
    solana_pubkey::Pubkey,
    solana_sdk_ids::native_loader,
    solana_svm::{
        account_loader::CheckedTransactionDetails,
        program_loader::load_program_with_pubkey,
        transaction_processing_result::ProcessedTransaction,
        transaction_processor::{
            ExecutionRecordingConfig, TransactionBatchProcessor, TransactionProcessingConfig,
            TransactionProcessingEnvironment,
        },
    },
    solana_svm_callback::{InvokeContextCallback, TransactionProcessingCallback},
    solana_svm_test_harness_fixture::{txn_context::TxnContext, txn_result::TxnResult},
    solana_svm_timings::ExecuteTimings,
    solana_svm_transaction::svm_message::SVMStaticMessage,
    std::{
        collections::HashSet,
        sync::{Arc, RwLock},
    },
};

const LAMPORTS_PER_SIGNATURE: u64 = 5000;
const SLOTS_PER_EPOCH: u64 = 432_000;

struct MockForkGraph;

impl ForkGraph for MockForkGraph {
    fn relationship(&self, _a: Slot, _b: Slot) -> BlockRelation {
        BlockRelation::Unknown
    }
}

struct TxnContextCallback<'a>(&'a TxnContext);

impl InvokeContextCallback for TxnContextCallback<'_> {
    fn is_precompile(&self, program_id: &Pubkey) -> bool {
        is_precompile(program_id, |feature_id: &Pubkey| {
            self.0.feature_set.is_active(feature_id)
        })
    }

    fn process_precompile(
        &self,
        program_id: &Pubkey,
        data: &[u8],
        instruction_datas: Vec<&[u8]>,
    ) -> std::result::Result<(), PrecompileError> {
        if let Some(precompile) = get_precompile(program_id, |feature_id: &Pubkey| {
            self.0.feature_set.is_active(feature_id)
        }) {
            precompile.verify(data, &instruction_datas, &self.0.feature_set)
        } else {
            Err(PrecompileError::InvalidPublicKey)
        }
    }
}

impl TransactionProcessingCallback for TxnContextCallback<'_> {
    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<(AccountSharedData, Slot)> {
        self.0
            .accounts
            .iter()
            .find(|(key, _)| key == pubkey)
            .filter(|(_, account)| account.lamports > 0)
            .map(|(_, account)| (AccountSharedData::from(account.clone()), self.0.slot))
    }
}

fn is_loader_owned(owner: &Pubkey) -> bool {
    solana_sdk_ids::loader_v4::check_id(owner)
        || solana_sdk_ids::bpf_loader_deprecated::check_id(owner)
        || solana_sdk_ids::bpf_loader::check_id(owner)
        || solana_sdk_ids::bpf_loader_upgradeable::check_id(owner)
}

fn load_programs_from_accounts(
    program_cache: &mut ProgramCache<MockForkGraph>,
    environments: &ProgramRuntimeEnvironments,
    callback: &TxnContextCallback<'_>,
) -> HashSet<Pubkey> {
    let mut builtin_ids = HashSet::new();
    let slot = callback.0.slot;

    for (pubkey, account) in &callback.0.accounts {
        // Track builtins.
        if native_loader::check_id(&account.owner) && account.executable {
            if let Some(builtin) = BUILTINS.iter().find(|b| b.program_id == *pubkey) {
                builtin_ids.insert(*pubkey);
                let entry = Arc::new(ProgramCacheEntry::new_builtin(
                    0,
                    account.data.len(),
                    builtin.entrypoint,
                ));
                program_cache.assign_program(
                    &ProgramRuntimeEnvironments::default(),
                    *pubkey,
                    0,
                    entry,
                );
            }
            continue;
        }

        // Load programs owned by one of the BPF loaders.
        if is_loader_owned(&account.owner) {
            if let Some((loaded_program, _last_modification_slot)) = load_program_with_pubkey(
                callback,
                environments,
                pubkey,
                slot,
                &mut ExecuteTimings::default(),
                false, // reload
            ) {
                program_cache.assign_program(environments, *pubkey, slot, loaded_program);
            }
        }
    }

    builtin_ids
}

fn setup_batch_processor(
    callback: &TxnContextCallback<'_>,
    compute_budget: &ComputeBudget,
    feature_set: &FeatureSet,
    fork_graph: &Arc<RwLock<MockForkGraph>>,
    epoch: u64,
) -> TransactionBatchProcessor<MockForkGraph> {
    let slot = callback.0.slot;

    let environments = ProgramRuntimeEnvironments {
        program_runtime_v1: Arc::new(
            create_program_runtime_environment_v1(
                &feature_set.runtime_features(),
                &compute_budget.to_budget(),
                false, /* deployment */
                false, /* debugging_features */
            )
            .unwrap(),
        ),
        ..ProgramRuntimeEnvironments::default()
    };

    let mut batch_processor = TransactionBatchProcessor::new_uninitialized(slot, epoch);

    batch_processor.environments.program_runtime_v1 = Arc::clone(&environments.program_runtime_v1);
    batch_processor.environments.program_runtime_v2 = Arc::clone(&environments.program_runtime_v2);

    let mut program_cache = batch_processor.global_program_cache.write().unwrap();
    program_cache.set_fork_graph(Arc::downgrade(fork_graph));

    // Load programs from accounts directly into global cache.
    let builtin_ids = load_programs_from_accounts(&mut program_cache, &environments, callback);
    drop(program_cache);

    // Set builtin program IDs.
    batch_processor
        .builtin_program_ids
        .get_mut()
        .unwrap()
        .extend(builtin_ids);

    batch_processor
}

/// Execute a single transaction against the Solana VM.
pub fn execute_txn(
    input: TxnContext,
    compute_budget: &ComputeBudget,
    recording_config: Option<ExecutionRecordingConfig>,
) -> Option<TxnResult> {
    let blockhash = input.blockhash_queue.first().cloned().unwrap_or_default();
    let feature_set = input.feature_set.runtime_features();
    let slot = input.slot;
    let epoch = slot / SLOTS_PER_EPOCH;

    let simd_0268_active = input
        .feature_set
        .is_active(&raise_cpi_nesting_limit_to_8::id());

    let compute_budget_limits = process_compute_budget_instructions(
        SVMStaticMessage::program_instructions_iter(&input.transaction),
        &input.feature_set,
    )
    .ok()?;

    let callback = TxnContextCallback(&input);
    let fork_graph = Arc::new(RwLock::new(MockForkGraph));
    let batch_processor = setup_batch_processor(
        &callback,
        compute_budget,
        &input.feature_set,
        &fork_graph,
        epoch,
    );

    let check_result = Ok(CheckedTransactionDetails::new(
        None,
        compute_budget_limits.get_compute_budget_and_limits(
            compute_budget_limits.loaded_accounts_bytes,
            FeeDetails::default(),
            simd_0268_active,
        ),
    ));

    let environments = batch_processor.get_environments_for_epoch(epoch);
    let processing_environment = TransactionProcessingEnvironment {
        blockhash,
        feature_set,
        blockhash_lamports_per_signature: LAMPORTS_PER_SIGNATURE,
        program_runtime_environments_for_execution: environments.clone(),
        program_runtime_environments_for_deployment: environments,
        ..Default::default()
    };

    let recording_config = recording_config.unwrap_or(ExecutionRecordingConfig {
        enable_cpi_recording: false,
        enable_log_recording: true,
        enable_return_data_recording: true,
        enable_transaction_balance_recording: false,
    });

    let processing_config = TransactionProcessingConfig {
        account_overrides: None,
        check_program_deployment_slot: false,
        log_messages_bytes_limit: None,
        limit_to_load_programs: true,
        recording_config,
        drop_on_failure: false,
        all_or_nothing: false,
    };

    let transactions = vec![input.transaction.clone()];
    let check_results = vec![check_result];

    let output = batch_processor.load_and_execute_sanitized_transactions(
        &callback,
        &transactions,
        check_results,
        &processing_environment,
        &processing_config,
    );

    match &output.processing_results[0] {
        Ok(txn) => {
            let resulting_accounts = match txn {
                ProcessedTransaction::Executed(executed_tx) => executed_tx
                    .loaded_transaction
                    .accounts
                    .iter()
                    .map(|(pubkey, account)| (*pubkey, Account::from(account.clone())))
                    .collect(),
                ProcessedTransaction::FeesOnly(fees_only_tx) => fees_only_tx
                    .rollback_accounts
                    .iter()
                    .map(|(pubkey, account)| (*pubkey, Account::from(account.clone())))
                    .collect(),
            };

            let return_data = match txn {
                ProcessedTransaction::Executed(executed_tx) => executed_tx
                    .execution_details
                    .return_data
                    .as_ref()
                    .map(|info| info.data.clone())
                    .unwrap_or_default(),
                ProcessedTransaction::FeesOnly(_) => vec![],
            };

            let inner_instructions = match txn {
                ProcessedTransaction::Executed(executed_tx) => {
                    executed_tx.execution_details.inner_instructions.clone()
                }
                ProcessedTransaction::FeesOnly(_) => None,
            };

            let log_messages = match txn {
                ProcessedTransaction::Executed(executed_tx) => {
                    executed_tx.execution_details.log_messages.clone()
                }
                ProcessedTransaction::FeesOnly(_) => None,
            };

            Some(TxnResult {
                status: txn.status(),
                resulting_accounts,
                return_data,
                executed_units: txn.executed_units(),
                fee_details: txn.fee_details(),
                loaded_accounts_data_size: txn.loaded_accounts_data_size() as u64,
                inner_instructions,
                log_messages,
            })
        }
        Err(err) => Some(TxnResult {
            status: Err(err.clone()),
            resulting_accounts: vec![],
            return_data: vec![],
            executed_units: 0,
            fee_details: FeeDetails::default(),
            loaded_accounts_data_size: 0,
            inner_instructions: None,
            log_messages: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*, agave_feature_set::FeatureSet, solana_hash::Hash, solana_keypair::Keypair,
        solana_message::Message, solana_signer::Signer,
        solana_svm_test_harness_fixture::txn_context::TxnContext,
        solana_svm_test_harness_instr::keyed_account::keyed_account_for_system_program,
        solana_system_interface::instruction as system_instruction, solana_sysvar_id::SysvarId,
        solana_transaction::Transaction,
    };

    fn sysvar_account(data: Vec<u8>) -> Account {
        Account {
            lamports: 1,
            data,
            owner: solana_sysvar_id::id(),
            executable: false,
            rent_epoch: u64::MAX,
        }
    }

    fn create_sysvar_accounts() -> Vec<(Pubkey, Account)> {
        vec![
            (
                solana_clock::Clock::id(),
                sysvar_account(bincode::serialize(&solana_clock::Clock::default()).unwrap()),
            ),
            (
                solana_rent::Rent::id(),
                sysvar_account(bincode::serialize(&solana_rent::Rent::default()).unwrap()),
            ),
            (
                solana_epoch_schedule::EpochSchedule::id(),
                sysvar_account(
                    bincode::serialize(&solana_epoch_schedule::EpochSchedule::default()).unwrap(),
                ),
            ),
            keyed_account_for_system_program(),
        ]
    }

    fn user_account(lamports: u64) -> Account {
        Account {
            lamports,
            data: vec![],
            owner: solana_sdk_ids::system_program::id(),
            executable: false,
            rent_epoch: u64::MAX,
        }
    }

    #[test]
    fn test_system_transfer() {
        let fee_payer = Keypair::new();
        let recipient = Pubkey::new_unique();
        let blockhash = Hash::new_unique();
        let slot = 10u64;
        let feature_set = FeatureSet::default();

        let mut accounts = vec![
            (fee_payer.pubkey(), user_account(10_000_000)),
            (recipient, user_account(0)),
        ];
        accounts.extend(create_sysvar_accounts());

        let transfer_amount = 1_000_000;
        let instruction =
            system_instruction::transfer(&fee_payer.pubkey(), &recipient, transfer_amount);
        let message = Message::new(&[instruction], Some(&fee_payer.pubkey()));
        let transaction = Transaction::new(&[&fee_payer], message, blockhash);

        let context = TxnContext::new(
            feature_set.clone(),
            vec![blockhash],
            slot,
            accounts,
            transaction,
        );

        // Set up the Compute Budget.
        let compute_budget = ComputeBudget::new_with_defaults(false, false);

        let result = execute_txn(context, &compute_budget, None)
            .expect("Transaction execution should succeed");
        assert!(result.status.is_ok(), "{:?}", result.status);

        let fee_payer_result = result
            .resulting_accounts
            .iter()
            .find(|(k, _)| k == &fee_payer.pubkey())
            .unwrap();
        let recipient_result = result
            .resulting_accounts
            .iter()
            .find(|(k, _)| k == &recipient)
            .unwrap();

        assert_eq!(fee_payer_result.1.lamports, 10_000_000 - transfer_amount);
        assert_eq!(recipient_result.1.lamports, transfer_amount);
    }

    #[test]
    fn test_transfer_insufficient_funds() {
        let fee_payer = Keypair::new();
        let recipient = Pubkey::new_unique();
        let blockhash = Hash::new_unique();
        let slot = 10u64;
        let feature_set = FeatureSet::default();

        let mut accounts = vec![
            (fee_payer.pubkey(), user_account(1000)),
            (recipient, user_account(0)),
        ];
        accounts.extend(create_sysvar_accounts());

        let instruction = system_instruction::transfer(&fee_payer.pubkey(), &recipient, 1_000_000);
        let message = Message::new(&[instruction], Some(&fee_payer.pubkey()));
        let transaction = Transaction::new(&[&fee_payer], message, blockhash);

        let context = TxnContext::new(
            feature_set.clone(),
            vec![blockhash],
            slot,
            accounts,
            transaction,
        );

        // Set up the Compute Budget.
        let compute_budget = ComputeBudget::new_with_defaults(false, false);

        let result = execute_txn(context, &compute_budget, None).unwrap();
        assert!(result.status.is_err());
    }
}
