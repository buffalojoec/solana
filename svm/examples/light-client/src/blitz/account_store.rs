//! Simple account store for Blitz accounts.

use {
    solana_sdk::{account::AccountSharedData, native_loader, pubkey::Pubkey, system_program},
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

    pub fn add_system_program(&mut self) {
        self.update(&[(
            system_program::id(),
            native_loader::create_loadable_account_with_fields("system_program", (5000, 0)),
        )]);
    }

    pub fn get(&self, pubkey: &Pubkey) -> Option<&AccountSharedData> {
        self.store.get(pubkey)
    }

    pub fn update(&mut self, updated_accounts: &[(Pubkey, AccountSharedData)]) {
        updated_accounts.iter().for_each(|(pubkey, account)| {
            self.store
                .entry(*pubkey)
                .and_modify(|a| *a = account.clone())
                .or_insert(account.clone());
        })
    }
}
