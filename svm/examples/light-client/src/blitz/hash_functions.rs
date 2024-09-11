//! Public module for hash functions, so light clients can ensure they're
//! using the same hashing functions for transaction data.

use {
    solana_sdk::keccak::Hasher,
    solana_svm_trace::{receipt::SVMTransactionReceipt, stf::STFTrace},
    solana_svm_transaction::svm_transaction::SVMTransaction,
};

pub fn hash_transaction(hasher: &mut Hasher, transaction: &impl SVMTransaction) {
    hasher.hash(transaction.signature().as_ref());
}

pub fn hash_receipt(
    hasher: &mut Hasher,
    transaction: &impl SVMTransaction,
    receipt: &SVMTransactionReceipt,
) {
    hasher.hash(transaction.signature().as_ref());
    solana_svm_trace::receipt::hash_receipt(hasher, receipt);
}

pub fn hash_trace(hasher: &mut Hasher, trace: &STFTrace<impl SVMTransaction>) {
    solana_svm_trace::stf::hash_trace(hasher, trace);
}
