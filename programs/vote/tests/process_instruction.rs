use {
    common::{
        VoteProgramFeatures, invalid_vote_state_pubkey, process_instruction,
        process_instruction_as_one_arg,
    },
    solana_hash::Hash,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_vote_interface::instruction::{tower_sync, tower_sync_switch},
    solana_vote_program::{
        vote_instruction::{
            CreateVoteAccountConfig, authorize, compact_update_vote_state,
            compact_update_vote_state_switch, create_account_with_config, update_commission,
            update_validator_identity, update_vote_state, update_vote_state_switch, vote,
            vote_switch, withdraw,
        },
        vote_state::{TowerSync, Vote, VoteAuthorize, VoteInit, VoteStateUpdate},
    },
};

mod common;

#[test]
fn test_vote_process_instruction_decode_bail() {
    process_instruction(
        VoteProgramFeatures {
            ..Default::default()
        },
        &[],
        Vec::new(),
        Vec::new(),
        Err(InstructionError::MissingAccount),
    );
}

#[test]
fn test_spoofed_vote() {
    let features = VoteProgramFeatures {
        ..Default::default()
    };
    process_instruction_as_one_arg(
        features,
        &vote(
            &invalid_vote_state_pubkey(),
            &Pubkey::new_unique(),
            Vote::default(),
        ),
        Err(InstructionError::InvalidAccountOwner),
    );
    process_instruction_as_one_arg(
        features,
        &update_vote_state(
            &invalid_vote_state_pubkey(),
            &Pubkey::default(),
            VoteStateUpdate::default(),
        ),
        Err(InstructionError::InvalidAccountOwner),
    );
    process_instruction_as_one_arg(
        features,
        &compact_update_vote_state(
            &invalid_vote_state_pubkey(),
            &Pubkey::default(),
            VoteStateUpdate::default(),
        ),
        Err(InstructionError::InvalidAccountOwner),
    );
    process_instruction_as_one_arg(
        features,
        &tower_sync(
            &invalid_vote_state_pubkey(),
            &Pubkey::default(),
            TowerSync::default(),
        ),
        Err(InstructionError::InvalidAccountOwner),
    );
}

#[test]
fn test_vote_process_instruction() {
    agave_logger::setup();
    let instructions = create_account_with_config(
        &Pubkey::new_unique(),
        &Pubkey::new_unique(),
        &VoteInit::default(),
        101,
        CreateVoteAccountConfig::default(),
    );
    let features = VoteProgramFeatures {
        ..Default::default()
    };
    // this case fails regardless of CreateVoteAccountConfig::space, because
    // process_instruction_as_one_arg passes a default (empty) account
    process_instruction_as_one_arg(
        features,
        &instructions[1],
        Err(InstructionError::InvalidAccountData),
    );
    process_instruction_as_one_arg(
        features,
        &vote(
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            Vote::default(),
        ),
        Err(InstructionError::InvalidInstructionData),
    );
    process_instruction_as_one_arg(
        features,
        &vote_switch(
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            Vote::default(),
            Hash::default(),
        ),
        Err(InstructionError::InvalidInstructionData),
    );
    process_instruction_as_one_arg(
        features,
        &authorize(
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            VoteAuthorize::Voter,
        ),
        Err(InstructionError::InvalidAccountData),
    );
    process_instruction_as_one_arg(
        features,
        &update_vote_state(
            &Pubkey::default(),
            &Pubkey::default(),
            VoteStateUpdate::default(),
        ),
        Err(InstructionError::InvalidInstructionData),
    );

    process_instruction_as_one_arg(
        features,
        &update_vote_state_switch(
            &Pubkey::default(),
            &Pubkey::default(),
            VoteStateUpdate::default(),
            Hash::default(),
        ),
        Err(InstructionError::InvalidInstructionData),
    );
    process_instruction_as_one_arg(
        features,
        &compact_update_vote_state(
            &Pubkey::default(),
            &Pubkey::default(),
            VoteStateUpdate::default(),
        ),
        Err(InstructionError::InvalidInstructionData),
    );
    process_instruction_as_one_arg(
        features,
        &compact_update_vote_state_switch(
            &Pubkey::default(),
            &Pubkey::default(),
            VoteStateUpdate::default(),
            Hash::default(),
        ),
        Err(InstructionError::InvalidInstructionData),
    );
    process_instruction_as_one_arg(
        features,
        &tower_sync(&Pubkey::default(), &Pubkey::default(), TowerSync::default()),
        Err(InstructionError::InvalidInstructionData),
    );
    process_instruction_as_one_arg(
        features,
        &tower_sync_switch(
            &Pubkey::default(),
            &Pubkey::default(),
            TowerSync::default(),
            Hash::default(),
        ),
        Err(InstructionError::InvalidInstructionData),
    );

    process_instruction_as_one_arg(
        features,
        &update_validator_identity(
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
        ),
        Err(InstructionError::InvalidAccountData),
    );
    process_instruction_as_one_arg(
        features,
        &update_commission(&Pubkey::new_unique(), &Pubkey::new_unique(), 0),
        Err(InstructionError::InvalidAccountData),
    );

    process_instruction_as_one_arg(
        features,
        &withdraw(
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            0,
            &Pubkey::new_unique(),
        ),
        Err(InstructionError::InvalidAccountData),
    );
}
