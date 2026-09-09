//! Example Rust-based SBF program that CPIs into itself a given number of
//! times, so a caller can drive the instruction stack to a chosen depth.

use {
    solana_account_info::AccountInfo,
    solana_instruction::{AccountMeta, Instruction},
    solana_program::program::invoke,
    solana_program_error::ProgramResult,
    solana_pubkey::Pubkey,
};

solana_program_entrypoint::entrypoint_no_alloc!(process_instruction);
fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let remaining = instruction_data.first().copied().unwrap_or(0);
    if remaining == 0 {
        return Ok(());
    }

    let mut data = instruction_data.to_vec();
    data[0] = remaining.saturating_sub(1);

    let instruction = Instruction {
        program_id: *program_id,
        accounts: accounts
            .iter()
            .map(|account| AccountMeta {
                pubkey: *account.key,
                is_signer: account.is_signer,
                is_writable: account.is_writable,
            })
            .collect(),
        data,
    };
    invoke(&instruction, accounts)
}
