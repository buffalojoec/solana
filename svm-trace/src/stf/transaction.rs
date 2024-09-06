use {solana_sdk::keccak::Hasher, solana_svm_transaction::svm_transaction::SVMTransaction};

pub fn hash_transaction(hasher: &mut Hasher, _transaction: &impl SVMTransaction) {
    // NOT doing all of that right now...
    hasher.hash(&[0u8]);
}
