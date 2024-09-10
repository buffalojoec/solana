//! The Blitz blockstore. Essentially the structure of Blitz blocks, including
//! headers.

use solana_sdk::{clock::Slot, keccak::Hash, transaction::Transaction};

/// Merkle roots of the block tries.
pub struct BlockRoots {
    /// Merkle root of the block's transaction receipts trie.
    pub receipts_root: Hash,
    /// Merkle root of the block's STF traces trie.
    pub traces_root: Hash,
}

/// A Blitz block headeer.
pub struct BlockHeader {
    /// Block roots.
    pub roots: BlockRoots,
    /// Slot the block was produced.
    pub slot: Slot,
}

/// A Blitz block.
pub struct Block {
    /// The block's header.
    pub header: BlockHeader,
    /// The block's transactions, ordered by execution.
    pub transactions: Vec<Transaction>,
}
