//! "Blitz" example L2, for light client demonstration purposes.
//!
//! To keep things super simple, the emulated "L2" (Blitz) is going to create
//! a very simple block with Solana transactions. After N number of
//! transactions, it will advance the slot, finalizing the current block and
//! beginning a new one.
//!
//! Blitz full nodes also generate proofs based on the Merkle tree structures
//! used by the node's "Merklizer", which stores transaction receipts and STF
//! traces in Merkle trees, generating proofs and roots when the block is
//! completed.

mod account_store;
mod batch_processor;
pub mod blockstore;
mod trie_store;

use {
    account_store::BlitzAccountStore,
    batch_processor::BlitzTransactionBatchProcessor,
    blockstore::{Block, BlockHeader, BlockRoots},
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        clock::Slot,
        keccak::Hasher,
        pubkey::Pubkey,
        transaction::{SanitizedTransaction, Transaction},
    },
    solana_svm::transaction_processing_callback::TransactionProcessingCallback,
    solana_svm_trace::{
        receipt::{hash_receipt, SVMTransactionReceipt},
        stf::{hash_account, hash_environment, hash_transaction, STFTrace},
        trie::Trie,
    },
    solana_svm_transaction::svm_transaction::SVMTransaction,
    std::sync::RwLock,
    trie_store::BlitzTrieStore,
};

/// The Blitz L2, in all its glory.
pub struct Blitz {
    /// Account store.
    account_store: BlitzAccountStore,
    /// Transaction batch processor (SVM API).
    processor: BlitzTransactionBatchProcessor,
    /// The current block's transactions, ordered by execution.
    processed_transactions: Vec<Transaction>,
    /// The current slot.
    slot: Slot,
    /// Cached hasher for STF entries.
    stf_hasher: RwLock<Hasher>,
    /// Merkle tree (or trie structure) store.
    trie_store: RwLock<BlitzTrieStore>,
    /// Ledger.
    pub ledger: Vec<Block>,
}

impl Blitz {
    const TRANSACTIONS_PER_BLOCK: usize = 10;

    fn block_space(&self) -> usize {
        Self::TRANSACTIONS_PER_BLOCK - self.processed_transactions.len()
    }

    fn pack_block(&mut self) {
        // Get the Merkle roots from the trie store.
        let (receipts_root, traces_root) = self
            .trie_store
            .read()
            .unwrap()
            .get(&self.slot)
            .map(|entry| {
                let get_root =
                    |trie: &Trie| trie.merklize().get_root().cloned().unwrap_or_default();
                (get_root(&entry.receipts_trie), get_root(&entry.traces_trie))
            })
            .unwrap();

        // Create a new block.
        let new_block = Block {
            header: BlockHeader {
                roots: BlockRoots {
                    receipts_root,
                    traces_root,
                },
                slot: self.slot,
            },
            transactions: std::mem::take(&mut self.processed_transactions),
        };

        // Push the new block onto the ledger and increment the slot.
        self.ledger.push(new_block);
        self.slot += 1;
    }

    pub fn process_transactions(&mut self, transactions: &[SanitizedTransaction]) {
        let next_batch = |offset: usize, block_space: usize| {
            transactions
                .get(offset..block_space)
                .unwrap_or(&transactions[offset..])
        };

        let mut offset = 0;

        while offset < transactions.len() {
            let batch = next_batch(offset, self.block_space());
            offset += batch.len();

            // Process the transaction batch.
            let result = self.processor.process_transaction_batch(self, batch);

            // Update the accounts store.
            for res in result.processing_results.iter().flatten() {
                if let Some(tx) = res.executed_transaction() {
                    self.account_store
                        .update(tx.loaded_transaction.accounts.iter());
                }
            }

            // Finish creating the block and move to a new one.
            self.pack_block();
        }
    }
}

impl Default for Blitz {
    fn default() -> Self {
        let blitz = Self {
            account_store: BlitzAccountStore::new(),
            processor: BlitzTransactionBatchProcessor::new(),
            processed_transactions: Vec::new(),
            slot: 0,
            stf_hasher: RwLock::<Hasher>::default(),
            trie_store: RwLock::<BlitzTrieStore>::default(),
            ledger: Vec::new(),
        };
        blitz.processor.configure_builtins(&blitz);
        blitz
    }
}

// This trait implementation is key here.
//
// We're implementing the callbacks on the `Blitz` struct itself. This way, we
// can use the current blocks' trie structures within the trace hooks.
impl TransactionProcessingCallback for Blitz {
    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
        self.account_store.get(pubkey).cloned()
    }

    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
        self.get_account_shared_data(account)
            .and_then(|account| owners.iter().position(|key| account.owner().eq(key)))
    }

    fn digest_processed_transaction(&self, transaction: &impl SVMTransaction) {
        let hash_fn = |hasher: &mut Hasher| {
            hasher.hash(transaction.signature().as_ref());
        };
        self.trie_store
            .write()
            .unwrap()
            .entry_mut(&self.slot)
            .transactions_trie
            .append(hash_fn);
    }

    fn digest_processed_receipt(
        &self,
        transaction: &impl SVMTransaction,
        receipt: &SVMTransactionReceipt,
    ) {
        let hash_fn = |hasher: &mut Hasher| {
            hasher.hash(transaction.signature().as_ref());
            hash_receipt(hasher, receipt);
        };
        self.trie_store
            .write()
            .unwrap()
            .entry_mut(&self.slot)
            .receipts_trie
            .append(hash_fn);
    }

    fn digest_processed_trace(&self, trace: &STFTrace<impl SVMTransaction>) {
        let stf_hasher = &mut *self.stf_hasher.write().unwrap();
        match trace {
            STFTrace::State(state) => {
                for (pubkey, account) in state.accounts {
                    hash_account(stf_hasher, pubkey, account);
                }
            }
            STFTrace::Directive(directive) => {
                hash_environment(stf_hasher, directive.environment);
                hash_transaction(stf_hasher, directive.transaction);
            }
            STFTrace::NewState(state) => {
                for (pubkey, account) in state.accounts {
                    hash_account(stf_hasher, pubkey, account);
                }
                self.trie_store
                    .write()
                    .unwrap()
                    .entry_mut(&self.slot)
                    .traces_trie
                    .push(stf_hasher.result_reset());
            }
        }
    }
}
