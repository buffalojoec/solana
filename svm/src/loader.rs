use {
    solana_program_runtime::loaded_programs::ProgramCacheMatchCriteria,
    solana_sdk::{account::AccountSharedData, pubkey::Pubkey},
};

/// The "loader" required by the transaction batch processor, responsible
/// mainly for loading accounts.
pub trait Loader {
    /// Load the account at the provided address.
    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData>;

    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize>;

    fn get_program_match_criteria(&self, _program: &Pubkey) -> ProgramCacheMatchCriteria {
        ProgramCacheMatchCriteria::NoCriteria
    }

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}
}
