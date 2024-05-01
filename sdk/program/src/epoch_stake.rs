use crate::{program_error::ProgramError, pubkey::Pubkey};

/// Get the current epoch stake for a given vote account.
/// Returns `0` for any provided pubkey that isn't a vote account.
pub fn get_epoch_stake(vote_address: &Pubkey) -> Result<u64, ProgramError> {
    let mut var = 0u64;
    let var_addr = &mut var as *mut _ as *mut u8;

    let vote_address = vote_address as *const _ as *const u8;

    #[cfg(target_os = "solana")]
    let result = unsafe { crate::syscalls::sol_syscall_get_epoch_stake(var_addr, vote_address) };

    #[cfg(not(target_os = "solana"))]
    let result = crate::program_stubs::sol_syscall_get_epoch_stake(var_addr, vote_address);

    match result {
        crate::entrypoint::SUCCESS => Ok(var),
        e => Err(e.into()),
    }
}
