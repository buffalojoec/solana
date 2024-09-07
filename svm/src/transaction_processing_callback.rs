use {
    solana_sdk::{account::AccountSharedData, pubkey::Pubkey},
    solana_svm_trace::{receipt::SVMTransactionReceipt, stf::STFTrace},
    solana_svm_transaction::svm_transaction::SVMTransaction,
};

/// Runtime callbacks for transaction processing.
pub trait TransactionProcessingCallback {
    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize>;

    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData>;

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}

    fn inspect_account(&self, _address: &Pubkey, _account_state: AccountState, _is_writable: bool) {
    }

    /// Hook for digesting a processed transaction during batch processing.
    /// Only called when a transaction is processed. Transactions that fail
    /// validation are not passed to digest.
    ///
    /// Designed for transaction inclusion proof generation (light clients).
    fn digest_processed_transaction(&self, _transaction: &impl SVMTransaction) {}

    /// Hook for digesting a processed transaction receipt during batch
    /// processing. Only called when a transaction is processed. Transactions
    /// that fail validation are not passed to digest.
    ///
    /// Designed for transaction result proof generation (light clients).
    fn digest_processed_receipt(
        &self,
        _transaction: &impl SVMTransaction,
        _receipt: &SVMTransactionReceipt,
    ) {
    }

    /// Hook for digesting a processed transactions STF trace during batch
    /// processing. Only called when a transaction is processed. Transactions
    /// that fail validation are not passed to digest.
    ///
    /// Designed for transaction STF proof generation (light clients).
    fn digest_processed_stf_trace(&self, _trace: &STFTrace<impl SVMTransaction>) {}
}

/// The state the account is in initially, before transaction processing
#[derive(Debug)]
pub enum AccountState<'a> {
    /// This account is dead, and will be created by this transaction
    Dead,
    /// This account is alive, and already existed prior to this transaction
    Alive(&'a AccountSharedData),
}
