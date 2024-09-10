//! Simple account store for Blitz accounts.

use {
    solana_sdk::{account::AccountSharedData, pubkey::Pubkey},
    std::collections::HashMap,
};

pub struct BlitzAccountStore {
    store: HashMap<Pubkey, AccountSharedData>,
}

impl BlitzAccountStore {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    pub fn get(&self, pubkey: &Pubkey) -> Option<&AccountSharedData> {
        self.store.get(pubkey)
    }

    pub fn update<'a>(
        &mut self,
        updated_accounts: impl Iterator<Item = &'a (Pubkey, AccountSharedData)>,
    ) {
        updated_accounts.for_each(|(pubkey, account)| {
            self.store
                .entry(*pubkey)
                .and_modify(|a| *a = account.clone())
                .or_insert(account.clone());
        })
    }
}
