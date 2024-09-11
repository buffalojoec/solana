//! Blitz layer 2 blockchain.
//!
//! Everything in the `blitz` module is suggested to be what a full node would
//! run. The full node stores ledger and tree data internally, and exposes
//! access to proofs created from its tree store through a public API. This can
//! be considered analogous to the full node's RPC API.
//!
//! Blitz very simply packs blocks once the number of processed transactions
//! has reached some threshold constant. Transactions are processed using the
//! SVM API.
//!
//! Each full node offers a public API for processing tranactions (TPU) and for
//! requesting proofs from its tree store (RPC).

mod account_store;
mod batch_processor;
pub mod blockstore;
pub mod hash_functions;
mod trie_store;

use {
    account_store::BlitzAccountStore,
    batch_processor::BlitzTransactionBatchProcessor,
    blockstore::{Block, BlockHeader, BlockRoots},
    solana_merkle_tree::{merkle_tree::Proof, MerkleTree},
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        clock::Slot,
        keccak::{Hash, Hasher},
        pubkey::Pubkey,
        transaction::{SanitizedTransaction, Transaction},
    },
    solana_svm::transaction_processing_callback::TransactionProcessingCallback,
    solana_svm_trace::{receipt::SVMTransactionReceipt, stf::STFTrace},
    solana_svm_transaction::svm_transaction::SVMTransaction,
    std::sync::RwLock,
    trie_store::{BlitzTreeStore, Merklizer, TreeStoreEntry},
};

/// Blitz protocol full node.
pub struct Blitz {
    /// Account store.
    account_store: BlitzAccountStore,
    /// The merklizer for the pending block.
    merklizer: RwLock<Merklizer>,
    /// Transaction batch processor (SVM API).
    processor: BlitzTransactionBatchProcessor,
    /// The already processed transactions for the pending block, ordered by
    /// execution.
    processed_transactions: Vec<Transaction>,
    /// The current slot.
    slot: Slot,
    /// Cached hasher for STF entries.
    stf_hasher: RwLock<Hasher>,
    /// Merkle tree store.
    tree_store: BlitzTreeStore,
    /// Ledger.
    pub ledger: Vec<Block>,
}

impl Blitz {
    const TRANSACTIONS_PER_BLOCK: usize = 10;

    pub fn add_accounts(&mut self, accounts: &[(Pubkey, AccountSharedData)]) {
        self.account_store.update(accounts)
    }

    fn block_space(&self) -> usize {
        Self::TRANSACTIONS_PER_BLOCK - self.processed_transactions.len()
    }

    fn get_proof(
        &self,
        slot: &Slot,
        candidate: &Hash,
        get_tree: impl Fn(&TreeStoreEntry) -> &MerkleTree,
    ) -> Option<Proof<'_>> {
        self.tree_store.get(slot).and_then(|trees| {
            let tree = get_tree(trees);
            tree.get_leaf_index(candidate)
                .and_then(|index| tree.find_path(index))
        })
    }

    /// Get a transaction inclusion proof from the full node's transaction
    /// tree.
    pub fn get_transaction_inclusion_proof(
        &self,
        slot: &Slot,
        candidate: &Hash,
    ) -> Option<Proof<'_>> {
        self.get_proof(slot, candidate, |trees| &trees.transactions_tree)
    }

    /// Get a transaction receipt proof from the full node's receipt tree.
    pub fn get_transaction_receipt_proof(
        &self,
        slot: &Slot,
        candidate: &Hash,
    ) -> Option<Proof<'_>> {
        self.get_proof(slot, candidate, |trees| &trees.receipts_tree)
    }

    /// Get a STF trace proof from the full node's trace tree.
    pub fn get_transaction_stf_trace_proof(
        &self,
        slot: &Slot,
        candidate: &Hash,
    ) -> Option<Proof<'_>> {
        self.get_proof(slot, candidate, |trees| &trees.traces_tree)
    }

    fn pack_block(&mut self) {
        let get_root = |tree: &MerkleTree| tree.get_root().cloned().unwrap_or_default();
        let trees = std::mem::take(&mut self.merklizer)
            .into_inner()
            .unwrap()
            .merklize();

        let new_block = Block {
            header: BlockHeader {
                roots: BlockRoots {
                    receipts_root: get_root(&trees.receipts_tree),
                    traces_root: get_root(&trees.traces_tree),
                    transactions_root: get_root(&trees.transactions_tree),
                },
                slot: self.slot,
            },
            transactions: std::mem::take(&mut self.processed_transactions),
        };

        self.ledger.push(new_block);
        self.tree_store.insert(self.slot, trees);

        self.slot += 1;
    }

    /// Process a batch of Solana transactions.
    pub fn process_transactions(&mut self, transactions: &[SanitizedTransaction]) {
        let mut offset = 0;

        // Chunk batches by `Self::TRANSACTIONS_PER_BLOCK`, creating a new
        // block per chunk.
        while offset < transactions.len() {
            let batch = transactions
                .get(offset..offset + self.block_space())
                .unwrap_or(&transactions[offset..]);
            offset += batch.len();

            // This is a bit weird, but process transactions in each batch one
            // at a time, so we can update accounts (commit) after each one.
            for i in 0..batch.len() {
                self.processor
                    .process_transaction_batch(self, &batch[i..i + 1])
                    .processing_results
                    .iter()
                    .flatten()
                    .for_each(|res| {
                        if let Some(tx) = res.executed_transaction() {
                            self.account_store.update(&tx.loaded_transaction.accounts);
                        }
                    })
            }

            self.pack_block();
        }
    }
}

impl Default for Blitz {
    fn default() -> Self {
        let mut blitz = Self {
            account_store: BlitzAccountStore::new(),
            merklizer: RwLock::<Merklizer>::default(),
            processor: BlitzTransactionBatchProcessor::new(),
            processed_transactions: Vec::new(),
            slot: 0,
            stf_hasher: RwLock::<Hasher>::default(),
            tree_store: BlitzTreeStore::default(),
            ledger: Vec::new(),
        };
        blitz.processor.add_system_program(&blitz);
        blitz.account_store.add_system_program();
        blitz
    }
}

// SVM API callback plugin implementation.
impl TransactionProcessingCallback for Blitz {
    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
        self.account_store.get(pubkey).cloned()
    }

    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
        self.get_account_shared_data(account)
            .and_then(|account| owners.iter().position(|key| account.owner().eq(key)))
    }

    // Digest a processed transaction by adding it to the transactions trie.
    fn digest_processed_transaction(&self, transaction: &impl SVMTransaction) {
        self.merklizer
            .write()
            .unwrap()
            .transactions_trie
            .append(|hasher: &mut Hasher| hash_functions::hash_transaction(hasher, transaction));
    }

    // Digest a processed receipt by adding it to the receipts trie.
    fn digest_processed_receipt(
        &self,
        transaction: &impl SVMTransaction,
        receipt: &SVMTransactionReceipt,
    ) {
        self.merklizer
            .write()
            .unwrap()
            .receipts_trie
            .append(|hasher: &mut Hasher| {
                hash_functions::hash_receipt(hasher, transaction, receipt)
            });
    }

    // Digest a processed STF trace by adding it to the traces trie.
    fn digest_processed_stf_trace(&self, trace: &STFTrace<impl SVMTransaction>) {
        let stf_hasher = &mut *self.stf_hasher.write().unwrap();
        hash_functions::hash_trace(stf_hasher, trace);

        // Only update the trie when we've received the new state (complete STF hash).
        if let STFTrace::NewState(_) = trace {
            self.merklizer
                .write()
                .unwrap()
                .traces_trie
                .push(stf_hasher.result_reset());
        }
    }
}
