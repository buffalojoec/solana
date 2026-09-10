//! Proxy authority program, modelled on a multisig such as Squads.
//!
//! Owns a PDA that holds another program's upgrade authority, and forwards a
//! loader instruction to that loader, signing for the PDA. `accounts[0]` is the
//! loader to call, the rest are forwarded to it, and the instruction data is
//! passed through untouched.

use {
    solana_account_info::AccountInfo,
    solana_instruction::{AccountMeta, Instruction},
    solana_program::program::invoke_signed,
    solana_program_error::{ProgramError, ProgramResult},
    solana_pubkey::Pubkey,
    solana_sbf_rust_squads_dep::{AUTHORITY, AUTHORITY_BUMP, AUTHORITY_SEED},
};

solana_program_entrypoint::entrypoint_no_alloc!(process_instruction);
fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let (loader, accounts) = accounts
        .split_first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    let instruction = Instruction {
        program_id: *loader.key,
        accounts: accounts
            .iter()
            .map(|account| AccountMeta {
                pubkey: *account.key,
                // The PDA can't sign at the top level, so grant it here.
                is_signer: account.is_signer || *account.key == AUTHORITY,
                is_writable: account.is_writable,
            })
            .collect(),
        data: instruction_data.to_vec(),
    };

    invoke_signed(
        &instruction,
        accounts,
        &[&[AUTHORITY_SEED, &[AUTHORITY_BUMP]]],
    )
}
