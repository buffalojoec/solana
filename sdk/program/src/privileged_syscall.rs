use crate::{account_info::AccountInfo, program_error::ProgramError};

/// Perform a large allocation on an account.
pub fn alloc_large(
    info: &AccountInfo,
    new_size: usize,
) -> Result<(), ProgramError> {
    let var_addr = info as *const _ as *mut u8;

    #[cfg(target_os = "solana")]
    unsafe { crate::syscalls::sol_mem_large_alloc_(var_addr, new_size as u64) }

    #[cfg(not(target_os = "solana"))]
    crate::program_memory::stubs::sol_mem_large_alloc(var_addr, new_size);
    
    Ok(())
}