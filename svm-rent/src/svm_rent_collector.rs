use solana_sdk::{
    account::AccountSharedData,
    clock::Epoch,
    pubkey::Pubkey,
    rent::{Rent, RentDue},
    rent_collector::CollectedInfo,
};

mod rent_collector;

/// Rent collector trait.
///
/// Implementors are responsible for evaluating rent due and collecting rent
/// from accounts, if required.
pub trait SVMRentCollector {
    /// Collect rent from an account.
    fn collect_rent(&self, address: &Pubkey, account: &mut AccountSharedData) -> CollectedInfo;

    /// Get the rent collector's rent instance.
    fn get_rent(&self) -> &Rent;

    /// Get the rent due for an account.
    fn get_rent_due(&self, lamports: u64, data_len: usize, account_rent_epoch: Epoch) -> RentDue;
}
