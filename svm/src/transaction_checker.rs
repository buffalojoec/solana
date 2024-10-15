//! Applies pre-processing checks to transactions.

use {
    crate::account_loader::TransactionCheckResult,
    solana_svm_transaction::svm_transaction::SVMTransaction,
};

pub trait TransactionChecker<T: SVMTransaction> {
    /// Check the transactions before processing.
    fn check_transactions(&self, transactions: &[T]) -> Vec<TransactionCheckResult>;
}
