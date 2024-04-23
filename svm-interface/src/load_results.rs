//! Solana transactions loaded by SVM.

use solana_sdk::{
    nonce_info::NonceFull,
    rent_debits::RentDebits,
    transaction::Result,
    transaction_context::{IndexOfAccount, TransactionAccount},
};

pub type TransactionRent = u64;
pub type TransactionProgramIndices = Vec<Vec<IndexOfAccount>>;
pub type TransactionLoadResult = (Result<LoadedTransaction>, Option<NonceFull>);

/// Solana transactions loaded by SVM.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct LoadedTransaction {
    pub accounts: Vec<TransactionAccount>,
    pub program_indices: TransactionProgramIndices,
    pub rent: TransactionRent,
    pub rent_debits: RentDebits,
}
