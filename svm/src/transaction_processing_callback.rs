use {
    solana_program_runtime::loaded_programs::ProgramCacheMatchCriteria,
    solana_sdk::{account::AccountSharedData, pubkey::Pubkey},
    solana_vote::vote_account::VoteAccountsHashMap,
};

/// Runtime callbacks for transaction processing.
pub trait TransactionProcessingCallback {
    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize>;

    fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData>;

    fn get_epoch_total_stake(&self) -> Option<u64>;

    fn get_epoch_vote_accounts(&self) -> Option<&VoteAccountsHashMap>;

    fn get_program_match_criteria(&self, _program: &Pubkey) -> ProgramCacheMatchCriteria {
        ProgramCacheMatchCriteria::NoCriteria
    }

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}
}
