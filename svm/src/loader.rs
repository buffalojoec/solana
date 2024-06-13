use {
    solana_program_runtime::loaded_programs::ProgramCacheMatchCriteria,
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        pubkey::Pubkey,
    },
};

/// The "loader" required by the transaction batch processor, responsible
/// mainly for loading accounts.
pub trait Loader {
    /// Load the account at the provided address.
    fn load_account(&self, pubkey: &Pubkey) -> Option<AccountSharedData>;

    /// Determine whether or not an account is owned by one of the programs in
    /// the provided set.
    ///
    /// This function has a default implementation, but projects can override
    /// it if they want to provide a more efficient implementation, such as
    /// checking account ownership without fully loading.
    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
        self.load_account(account)
            .and_then(|account| owners.iter().position(|entry| account.owner() == entry))
    }

    fn get_program_match_criteria(&self, _program: &Pubkey) -> ProgramCacheMatchCriteria {
        ProgramCacheMatchCriteria::NoCriteria
    }

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}
}
