//! Simple trie store for Blitz Merkle-Patricia tries.

use {
    solana_merkle_tree::MerkleTree, solana_sdk::clock::Slot, solana_svm_trace::trie::Trie,
    std::collections::HashMap,
};

#[derive(Default)]
pub struct Merklizer {
    /// Merkle-Patricia Trie of transaction receipts.
    pub receipts_trie: Trie,
    /// Merkle-Patricia Trie of STF traces.
    pub traces_trie: Trie,
    /// Merkle-Patricia Trie of transactions.
    pub transactions_trie: Trie,
}

impl Merklizer {
    pub fn merklize(&self) -> TreeStoreEntry {
        TreeStoreEntry {
            receipts_tree: self.receipts_trie.merklize(),
            traces_tree: self.traces_trie.merklize(),
            transactions_tree: self.transactions_trie.merklize(),
        }
    }
}

pub struct TreeStoreEntry {
    /// Merkle tree of transaction receipts.
    pub receipts_tree: MerkleTree,
    /// Merkle tree of STF traces.
    pub traces_tree: MerkleTree,
    /// Merkle tree of transactions.
    pub transactions_tree: MerkleTree,
}

#[derive(Default)]
pub struct BlitzTreeStore {
    store: HashMap<Slot, TreeStoreEntry>,
}

impl BlitzTreeStore {
    pub fn get(&self, slot: &Slot) -> Option<&TreeStoreEntry> {
        self.store.get(slot)
    }

    pub(crate) fn insert(&mut self, slot: Slot, entry: TreeStoreEntry) {
        self.store.insert(slot, entry);
    }
}
