use {
    common::{
        VoteProgramFeatures, create_default_clock_account, create_default_rent_account,
        process_instruction, vote_state_size_of,
    },
    solana_account::AccountSharedData,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_sdk_ids::sysvar,
    solana_vote_program::{
        id,
        vote_instruction::{CreateVoteAccountConfig, create_account_with_config},
        vote_state::{self, VoteInit},
    },
};

mod common;

#[test]
fn test_create_account_vote_state_1_14_11() {
    let node_pubkey = Pubkey::new_unique();
    let vote_pubkey = Pubkey::new_unique();
    let instructions = create_account_with_config(
        &node_pubkey,
        &vote_pubkey,
        &VoteInit {
            node_pubkey,
            authorized_voter: vote_pubkey,
            authorized_withdrawer: vote_pubkey,
            commission: 0,
        },
        101,
        CreateVoteAccountConfig {
            space: vote_state::VoteState1_14_11::size_of() as u64,
            ..CreateVoteAccountConfig::default()
        },
    );
    // grab the `space` value from SystemInstruction::CreateAccount by directly indexing, for
    // expediency
    let space = usize::from_le_bytes(instructions[0].data[12..20].try_into().unwrap());
    assert_eq!(space, vote_state::VoteState1_14_11::size_of());
    let empty_vote_account = AccountSharedData::new_with_data(101, vec![0; space], &id());

    let transaction_accounts = vec![
        (vote_pubkey, empty_vote_account),
        (node_pubkey, AccountSharedData::default()),
        (sysvar::clock::id(), create_default_clock_account()),
        (sysvar::rent::id(), create_default_rent_account()),
    ];

    // should fail, since VoteState1_14_11 isn't supported anymore
    process_instruction(
        VoteProgramFeatures {
            ..Default::default()
        },
        &instructions[1].data,
        transaction_accounts,
        instructions[1].accounts.clone(),
        Err(InstructionError::InvalidAccountData),
    );
}

#[test]
fn test_create_account_vote_state_current() {
    let node_pubkey = Pubkey::new_unique();
    let vote_pubkey = Pubkey::new_unique();
    let instructions = create_account_with_config(
        &node_pubkey,
        &vote_pubkey,
        &VoteInit {
            node_pubkey,
            authorized_voter: vote_pubkey,
            authorized_withdrawer: vote_pubkey,
            commission: 0,
        },
        101,
        CreateVoteAccountConfig {
            space: vote_state_size_of() as u64,
            ..CreateVoteAccountConfig::default()
        },
    );
    // grab the `space` value from SystemInstruction::CreateAccount by directly indexing, for
    // expediency
    let space = usize::from_le_bytes(instructions[0].data[12..20].try_into().unwrap());
    assert_eq!(space, vote_state_size_of());
    let empty_vote_account = AccountSharedData::new_with_data(101, vec![0; space], &id());

    let transaction_accounts = vec![
        (vote_pubkey, empty_vote_account),
        (node_pubkey, AccountSharedData::default()),
        (sysvar::clock::id(), create_default_clock_account()),
        (sysvar::rent::id(), create_default_rent_account()),
    ];

    process_instruction(
        VoteProgramFeatures {
            ..Default::default()
        },
        &instructions[1].data,
        transaction_accounts,
        instructions[1].accounts.clone(),
        Ok(()),
    );
}
