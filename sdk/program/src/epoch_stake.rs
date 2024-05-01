use crate::pubkey::Pubkey;

/// Get the current epoch stake for a given vote address.
///
/// If the provided vote address corresponds to an account that is not a vote
/// account or does not exist, returns `0` for active stake.
pub fn get_epoch_stake(vote_address: &Pubkey) -> u64 {
    let vote_address = vote_address as *const _ as *const u8;

    #[cfg(target_os = "solana")]
    let result = unsafe { crate::syscalls::sol_get_epoch_stake(vote_address) };

    #[cfg(not(target_os = "solana"))]
    let result = crate::program_stubs::sol_get_epoch_stake(vote_address);

    result
}
