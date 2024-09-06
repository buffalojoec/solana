use solana_sdk::{account::AccountSharedData, pubkey::Pubkey};
use solana_svm_trace::receipt::SVMTransactionReceipt;

/// Runtime callbacks for transaction processing.
pub trait TransactionProcessingCallback {
    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize>;

    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData>;

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}

    fn inspect_account(&self, _address: &Pubkey, _account_state: AccountState, _is_writable: bool) {
    }

    // ::[SVM_STF]::
    // If we use these callbacks instead of adding the hashes to SMV's return
    // type, it's much easier to link the hashes to a global data structure,
    // one which may or may not be per-block.
    // In the case of a Merklized block (for STF or receipts), this is how
    // you'd make that connection.

    fn consume_stf(
        &self,
        _slot: u64,
        _blockhash: &solana_sdk::hash::Hash,
        _tx_signature: &solana_sdk::signature::Signature,
        _stf: &solana_sdk::keccak::Hash,
    ) {
    }

    /// Consume a transaction receipt. Only available if `enable_receipts` was
    /// set to `true` in the provided `TransactionProcessingConfig`.
    fn consume_receipt(
        &self,
        _slot: u64,
        _blockhash: &solana_sdk::hash::Hash,
        _tx_signature: &solana_sdk::signature::Signature,
        _receipt: &SVMTransactionReceipt,
    ) {
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
