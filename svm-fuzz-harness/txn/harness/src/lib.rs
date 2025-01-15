//! Solana SVM fuzz harness for transaction.
//!
//! This module represents the entrypoint function available for fuzzing or
//! conformance testing, and includes various setup that is not imposed on the
//! developer in the base entrypoint.
//!
//! Specifically, this harness is just one implementation wherein various
//! required components - such as the bank - are configured, various checks
//! are imposed, and the base entrypoint for the runtime is invoked
//! (`solana-svm-fuzz-harness-txn-entrypoint`).
//!
//! This particular harness is used by the Firedancer team to test Firedancer's
//! conformance with Agave. Other validator clients may wish to use this same
//! harness, or define their own, which can be built on the base entrypoint in
//! similar fashion.

use {
    prost::Message,
    solana_account::{Account, AccountSharedData},
    solana_epoch_schedule::EpochSchedule,
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_sdk::{
        genesis_config::GenesisConfig, instruction::InstructionError, message::SanitizedMessage,
        transaction::TransactionError,
    },
    solana_svm::{
        account_loader::LoadedTransaction, transaction_processing_result::ProcessedTransaction,
    },
    solana_svm_fuzz_harness_fixture::{
        proto::{TxnContext as ProtoTxnContext, TxnResult as ProtoTxnResult},
        txn::{
            context::TxnContext,
            result::{ResultingState, TxnResult},
        },
    },
    solana_svm_fuzz_harness_txn_entrypoint::EnvironmentContext,
    std::{collections::HashSet, ffi::c_int},
};

/* Returns (txn_err, instr_err, custom_err, instr_err_idx) */
fn transaction_error_to_err_nums(transaction_error: &TransactionError) -> (u32, u32, u32, u32) {
    let (instr_err_no, custom_err_no, instr_err_idx) = match transaction_error.clone() {
        TransactionError::InstructionError(instr_err_idx, instruction_error) => {
            let instr_err_no = {
                let serialized = bincode::serialize(&instruction_error).unwrap_or(vec![0, 0, 0, 0]);
                u32::from_le_bytes(serialized[0..4].try_into().unwrap()).saturating_add(1)
            };
            let custom_err_no = match instruction_error {
                InstructionError::Custom(custom_err_no) => custom_err_no,
                _ => 0,
            };
            (instr_err_no, custom_err_no, instr_err_idx as u32)
        }
        _ => (0, 0, 0),
    };
    let txn_err_no = {
        let serialized = bincode::serialize(&transaction_error).unwrap_or(vec![0, 0, 0, 0]);
        u32::from_le_bytes(serialized[0..4].try_into().unwrap()).saturating_add(1)
    };
    (txn_err_no, instr_err_no, custom_err_no, instr_err_idx)
}

fn convert_loaded_transaction(value: LoadedTransaction) -> ResultingState {
    let rent_debits = value
        .rent_debits
        .into_unordered_rewards_iter()
        .map(|(key, value)| (key, value.lamports as u64))
        .collect::<Vec<_>>();

    let account_states = value
        .accounts
        .into_iter()
        .map(|(pubkey, acct)| (pubkey, Account::from(acct)))
        .collect::<Vec<_>>();

    ResultingState {
        account_states,
        rent_debits,
        transaction_rent: value.rent,
    }
}

fn convert_processed_transaction(txn: ProcessedTransaction) -> TxnResult {
    let is_ok = match &txn {
        ProcessedTransaction::Executed(executed_tx) => executed_tx.execution_details.status.is_ok(),
        ProcessedTransaction::FeesOnly(_) => false,
    };
    let (status, instruction_error, custom_error, instruction_error_index) =
        match txn.status().as_ref().map_err(transaction_error_to_err_nums) {
            Ok(_) => (0, 0, 0, 0),
            Err((status, instr_err, custom_err, instr_err_idx)) => {
                (status, instr_err, custom_err, instr_err_idx)
            }
        };
    let rent = match &txn {
        ProcessedTransaction::Executed(executed_tx) => executed_tx.loaded_transaction.rent,
        ProcessedTransaction::FeesOnly(_) => 0,
    };
    let resulting_state: Option<ResultingState> = match &txn {
        ProcessedTransaction::Executed(executed_tx) => Some(convert_loaded_transaction(
            executed_tx.loaded_transaction.clone(),
        )),
        ProcessedTransaction::FeesOnly(_) => None,
    };
    let executed_units = match &txn {
        ProcessedTransaction::Executed(executed_tx) => executed_tx.execution_details.executed_units,
        ProcessedTransaction::FeesOnly(_) => 0,
    };
    let return_data = match &txn {
        ProcessedTransaction::Executed(executed_tx) => executed_tx
            .execution_details
            .return_data
            .as_ref()
            .map(|info| info.clone().data)
            .unwrap_or_default(),
        ProcessedTransaction::FeesOnly(_) => vec![],
    };
    TxnResult {
        executed: true,
        sanitization_error: false,
        resulting_state,
        rent,
        is_ok,
        status,
        instruction_error,
        instruction_error_index,
        custom_error,
        return_data,
        executed_units,
        fee_details: Some(txn.fee_details()),
    }
}

fn execute_txn(input: TxnContext) -> Option<TxnResult> {
    let fee_collector = Pubkey::new_unique();

    let account_keys = input.transaction.message.static_account_keys().to_owned();

    /* HACK: Set the genesis config rent and epoch schedule from the "to-be" sysvars, if present */
    let rent: Rent = input
        .accounts
        .iter()
        .find(|(pubkey, acct)| pubkey == &solana_sdk::sysvar::rent::id() && acct.lamports > 0)
        .and_then(|(_, acct)| bincode::deserialize(&acct.data).ok())
        .unwrap_or_default();
    let epoch_schedule: EpochSchedule = input
        .accounts
        .iter()
        .find(|(pubkey, acct)| {
            pubkey == &solana_sdk::sysvar::epoch_schedule::id() && acct.lamports > 0
        })
        .and_then(|(_, acct)| bincode::deserialize(&acct.data).ok())
        .unwrap_or_default();

    /* HACK: Add dummy ALUT and config program accounts to genesis config so that their builtin versions don't get added to the program cache */
    let mut genesis_config = GenesisConfig {
        creation_time: 0,
        rent,
        epoch_schedule,
        ..GenesisConfig::default()
    };
    [
        (
            solana_sdk::address_lookup_table::program::id(),
            AccountSharedData::new(1u64, 0, &solana_sdk::bpf_loader_upgradeable::id()),
        ),
        (
            solana_sdk::config::program::id(),
            AccountSharedData::new(1u64, 0, &solana_sdk::bpf_loader_upgradeable::id()),
        ),
    ]
    .into_iter()
    .for_each(|(key, account)| {
        genesis_config.add_account(key, account);
    });

    let genesis_hash = Some(input.blockhash_queue[0]);

    let joe = solana_svm_fuzz_harness_txn_entrypoint::process_transaction(
        &input.accounts,
        input.transaction,
        EnvironmentContext::new(
            /* accounts_to_remove_from_bank */
            &[
                solana_sdk::address_lookup_table::program::id(),
                solana_sdk::config::program::id(),
            ],
            &input.blockhash_queue,
            &input.epoch_context.feature_set,
            &fee_collector,
            &genesis_config,
            genesis_hash,
            input.slot_context.slot,
        ),
    );

    let (sanitized_transaction, mut txn_result) = match joe {
        Ok((san, proc)) => (san, convert_processed_transaction(proc)),
        Err(e) => {
            let (status, instruction_error, _custom_error, instruction_error_index) =
                transaction_error_to_err_nums(&e);
            return Some(TxnResult {
                executed: false,
                sanitization_error: true,
                resulting_state: None,
                rent: 0,
                is_ok: false,
                status,
                instruction_error,
                instruction_error_index,
                custom_error: 0, // TODO: precompile error codes are not conformant, so we're ignoring custom error codes for now. This should be revisited in the future.
                return_data: vec![],
                executed_units: 0,
                fee_details: None,
            });
        }
    };

    if let Some(relevant_accounts) = &mut txn_result.resulting_state {
        let mut loaded_account_keys = HashSet::<Pubkey>::new();
        loaded_account_keys.extend(account_keys.iter());
        match sanitized_transaction.message() {
            SanitizedMessage::Legacy(_) => {}
            SanitizedMessage::V0(message) => {
                loaded_account_keys.extend(message.loaded_addresses.writable.clone().iter());
                loaded_account_keys.extend(message.loaded_addresses.readonly.clone().iter());
            }
        }

        relevant_accounts.account_states = relevant_accounts
            .clone()
            .account_states
            .into_iter()
            .enumerate()
            .filter(|&(i, _)| sanitized_transaction.message().is_writable(i))
            .map(|(_, account)| account)
            .collect();

        // Only keep accounts that were passed in as account_keys or as ALUT accounts
        relevant_accounts
            .account_states
            .retain(|(pubkey, _)| loaded_account_keys.contains(pubkey));

        txn_result.resulting_state = Some(relevant_accounts.clone());
    }

    Some(txn_result)
}

/// Main harness for executing Agave's execution-layer transaction entrypoint
/// using a Protobuf transaction context.
///
/// Returns the transactions's result as a Protobuf transaction "effects".
pub fn execute_txn_proto(input: ProtoTxnContext) -> Option<ProtoTxnResult> {
    let txn_context = TxnContext::try_from(input).unwrap();
    let txn_effects = execute_txn(txn_context);
    txn_effects.map(Into::into)
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn sol_compat_txn_execute_v1(
    out_ptr: *mut u8,
    out_psz: *mut u64,
    in_ptr: *mut u8,
    in_sz: u64,
) -> c_int {
    if in_ptr.is_null() || in_sz == 0 {
        return 0;
    }
    let in_slice = std::slice::from_raw_parts(in_ptr, in_sz as usize);
    let Ok(txn_context) = ProtoTxnContext::decode(&in_slice[..in_sz as usize]) else {
        return 0;
    };

    let Some(txn_result) = execute_txn_proto(txn_context) else {
        return 0;
    };

    let out_slice = std::slice::from_raw_parts_mut(out_ptr, (*out_psz) as usize);
    let out_vec = txn_result.encode_to_vec();
    if out_vec.len() > out_slice.len() {
        return 0;
    }

    out_slice[..out_vec.len()].copy_from_slice(&out_vec);
    *out_psz = out_vec.len() as u64;

    1
}
