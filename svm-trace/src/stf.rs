//! Docs!

pub mod environment;
pub mod state;
pub mod transaction;

pub use {environment::*, transaction::*};
use {
    solana_sdk::{
        account::AccountSharedData,
        keccak::{Hash, Hasher},
        pubkey::Pubkey,
    },
    solana_svm_transaction::svm_transaction::SVMTransaction,
    state::hash_accounts,
};

pub struct STFHasher {
    // Nothing special for now, just keccak.
    hasher: Hasher,
}

impl STFHasher {
    pub fn new() -> Self {
        Self {
            hasher: Hasher::default(),
        }
    }

    pub fn hash_pre_state(&mut self, accounts: &[(Pubkey, AccountSharedData)]) {
        hash_accounts(&mut self.hasher, accounts);
    }

    pub fn hash_directive(
        &mut self,
        transaction: &impl SVMTransaction,
        environment: &STFEnvironment,
    ) {
        hash_transaction(&mut self.hasher, transaction);
        hash_environment(&mut self.hasher, environment);
    }

    pub fn hash_post_state(&mut self, accounts: &[(Pubkey, AccountSharedData)]) {
        hash_accounts(&mut self.hasher, accounts);
    }

    pub fn result(&mut self) -> Hash {
        std::mem::take(&mut self.hasher).result()
    }
}
