use {
    crate::{
        account_loader::{CheckedTransactionDetails, TransactionCheckResult},
        transaction_error_metrics::TransactionErrorMetrics,
        transaction_processor::{TransactionProcessingConfig, TransactionProcessingEnvironment},
    },
    solana_sdk::{account::AccountSharedData, pubkey::Pubkey, transaction},
    solana_svm_transaction::svm_transaction::SVMTransaction,
    solana_timings::ExecuteTimings,
};

/// Runtime callbacks for transaction processing.
pub trait TransactionProcessingCallback {
    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize>;

    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData>;

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}

    fn inspect_account(&self, _address: &Pubkey, _account_state: AccountState, _is_writable: bool) {
    }

    fn check_transactions(
        &self,
        sanitized_txs: &[impl SVMTransaction],
        environment: &TransactionProcessingEnvironment,
        _config: &TransactionProcessingConfig,
        _lock_results: &[transaction::Result<()>],
        _error_counters: &mut TransactionErrorMetrics,
        _timings: &mut ExecuteTimings,
    ) -> Vec<TransactionCheckResult> {
        vec![
            Ok(CheckedTransactionDetails {
                nonce: None,
                lamports_per_signature: environment.lamports_per_signature,
            });
            sanitized_txs.len()
        ]
    }
}

/// The state the account is in initially, before transaction processing
#[derive(Debug)]
pub enum AccountState<'a> {
    /// This account is dead, and will be created by this transaction
    Dead,
    /// This account is alive, and already existed prior to this transaction
    Alive(&'a AccountSharedData),
}
