//! Simple tree store for Blitz Merkle trees.

use {solana_sdk::clock::Slot, solana_svm_trace::trie::Trie, std::collections::HashMap};

pub struct TrieStoreEntry {
    /// Merkle tree of transaction receipts.
    pub receipts_trie: Trie,
    /// Merkle tree of STF traces.
    pub traces_trie: Trie,
    /// Merkle tree of transactions.
    pub transactions_trie: Trie,
}

#[derive(Default)]
pub struct BlitzTrieStore {
    store: HashMap<Slot, TrieStoreEntry>,
}

impl BlitzTrieStore {
    pub fn entry_mut(&mut self, slot: &Slot) -> &mut TrieStoreEntry {
        self.store.entry(*slot).or_insert_with(|| TrieStoreEntry {
            receipts_trie: Trie::default(),
            traces_trie: Trie::default(),
            transactions_trie: Trie::default(),
        })
    }

    pub fn get(&self, slot: &Slot) -> Option<&TrieStoreEntry> {
        self.store.get(slot)
    }
}
