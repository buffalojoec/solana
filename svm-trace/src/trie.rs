//! A custom trie structure for storing SVM execution traces.
//!
//! TODO: This is a temporary mock-up of the intended data structure.
//! What we need here is a Merkle-Patricia Trie, which will allow us to add
//! new entries and re-hash the tree incrementally.
//!
//! For now, I've just wrapped a Merkle tree by storing the leaves in a vector,
//! then calling `merklize` to create a new Merkle tree. Highly inefficient!

use {
    solana_merkle_tree::MerkleTree,
    solana_sdk::keccak::{Hash, Hasher},
};

/// Trie structure for SVM execution traces.
#[derive(Default)]
pub struct Trie {
    hasher: Hasher,
    // Naive approach - store the leaves, then when finished, create a new
    // Merkle tree.
    leaves: Vec<Hash>,
}

impl Trie {
    /// Append to the trie.
    pub fn append(&mut self, hash_fn: impl Fn(&mut Hasher)) {
        hash_fn(&mut self.hasher);
        let hash = self.hasher.result_reset();
        self.leaves.push(hash)
    }

    /// Push a hash into the trie's leaves.
    pub fn push(&mut self, hash: Hash) {
        self.leaves.push(hash);
    }

    /// Merklize the trie.
    pub fn merklize(&self) -> MerkleTree {
        MerkleTree::new(&self.leaves)
    }
}
