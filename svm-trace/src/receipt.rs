//! SVM transaction receipts.

use solana_merkle_tree::MerkleTree;
use solana_sdk::{
    fee::FeeDetails,
    keccak::{Hash, Hasher},
    transaction,
    transaction_context::TransactionReturnData,
};

/// An SVM transaction receipt. Stores resulting data from the execution of a
/// transaction in SVM.
pub struct SVMTransactionReceipt<'a> {
    pub compute_units_consumed: u64,
    pub fee_details: &'a FeeDetails,
    pub log_messages: Option<&'a Vec<String>>,
    pub return_data: Option<&'a TransactionReturnData>,
    pub status: &'a transaction::Result<()>,
}

fn hash_receipt(hasher: &mut Hasher, receipt: &SVMTransactionReceipt) {
    // `compute_units_consumed`
    hasher.hash(&receipt.compute_units_consumed.to_le_bytes());

    // `fee_details`
    hasher.hashv(&[
        &receipt.fee_details.transaction_fee().to_le_bytes(),
        &receipt.fee_details.prioritization_fee().to_le_bytes(),
        // TODO: `remove_rounding_in_fee_calculation` omitted.
    ]);

    // `log_messages`
    receipt.log_messages.map(|messages| {
        for m in messages {
            hasher.hash(m.as_bytes());
        }
    });

    // `return_data`
    receipt.return_data.map(|data| {
        hasher.hashv(&[&data.program_id.as_ref(), &data.data]);
    });

    // `status`
    hasher.hash(&[match receipt.status {
        Ok(()) => 0,
        Err(_) => 1, // TODO: Error codes.
    }]);
}

/// Trie structure of hashed transaction receipts. Can be used to generate
/// proofs.
pub struct SVMTransactionReceiptsTrie {
    hasher: Hasher,
    // Naive approach - store the leaves, then when finished, create a new
    // Merkle tree. TODO: Optimize this by instead using a Merkle-Patricia
    // Trie from rETH.
    leaves: Vec<Hash>,
}

impl SVMTransactionReceiptsTrie {
    /// Create a new empty trie.
    pub fn new() -> Self {
        Self {
            hasher: Hasher::default(),
            leaves: Vec::new(),
        }
    }

    /// Append to the trie.
    pub fn append(&mut self, receipt: &SVMTransactionReceipt) {
        hash_receipt(&mut self.hasher, receipt);
        let hash = self.hasher.result_reset();
        self.leaves.push(hash)
    }

    /// Finalize the trie and get the root.
    pub fn finalize(&self) -> Hash {
        let merkle_tree = MerkleTree::new(&self.leaves);
        merkle_tree.get_root().unwrap().clone()
    }
}
