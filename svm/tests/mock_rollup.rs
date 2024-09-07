#![allow(unused)]

#[path = "./mock_bank.rs"]
pub mod mock_bank;

use {
    mock_bank::MockBankCallback,
    solana_sdk::{
        account::AccountSharedData, pubkey::Pubkey, rent_collector::RentCollector,
        signature::Signature,
    },
    solana_svm::transaction_processing_callback::{AccountState, TransactionProcessingCallback},
    solana_svm_trace::receipt::SVMTransactionReceipt,
    solana_svm_transaction::svm_transaction::SVMTransaction,
};

// Plugin trait to let each test case define its own "handler" hooks, without
// having to go through all of the annoying setup below.
pub trait TraceHandler: Default {
    fn digest_transaction(&self, transaction: &impl SVMTransaction);
    fn digest_receipt(&self, transaction: &impl SVMTransaction, receipt: &SVMTransactionReceipt);
}

// All the setup is done on `MockRollup`, and we can customize some of the
// callbacks through the plugin traits above.
#[derive(Default)]
pub struct MockRollup<R>
where
    R: TraceHandler,
{
    bank: MockBankCallback,
    rent_collector: RentCollector,
    trace_handler: R,
}

impl<R> MockRollup<R>
where
    R: TraceHandler,
{
    pub fn bank(&self) -> &MockBankCallback {
        &self.bank
    }

    pub fn rent_collector(&self) -> &RentCollector {
        &self.rent_collector
    }

    pub fn trace_handler(&self) -> &R {
        &self.trace_handler
    }

    pub fn add_rent_exempt_account(
        &self,
        pubkey: &Pubkey,
        data: &[u8],
        owner: &Pubkey,
        excess_lamports: u64,
    ) {
        let space = data.len();
        let lamports = self
            .rent_collector
            .rent
            .minimum_balance(space)
            .saturating_add(excess_lamports);
        let mut account = AccountSharedData::new(lamports, space, owner);
        account.set_data_from_slice(data);
        self.bank
            .account_shared_data
            .write()
            .unwrap()
            .insert(*pubkey, account);
    }
}

impl<R> TransactionProcessingCallback for MockRollup<R>
where
    R: TraceHandler,
{
    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
        self.bank.account_matches_owners(account, owners)
    }

    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
        self.bank.get_account_shared_data(pubkey)
    }

    fn add_builtin_account(&self, name: &str, program_id: &Pubkey) {
        self.bank.add_builtin_account(name, program_id)
    }

    fn inspect_account(&self, address: &Pubkey, account_state: AccountState, is_writable: bool) {
        self.bank
            .inspect_account(address, account_state, is_writable)
    }

    // Override.
    fn digest_processed_transaction(&self, transaction: &impl SVMTransaction) {
        self.trace_handler.digest_transaction(transaction);
    }

    // Override.
    fn digest_processed_receipt(
        &self,
        transaction: &impl SVMTransaction,
        receipt: &SVMTransactionReceipt,
    ) {
        self.trace_handler.digest_receipt(transaction, receipt);
    }
}
