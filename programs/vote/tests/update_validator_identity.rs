use {
    common::{
        VoteProgramFeatures, create_test_account_with_authorized,
        deserialize_vote_state_for_test, process_instruction,
    },
    solana_account::{AccountSharedData, ReadableAccount},
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_vote_program::vote_instruction::VoteInstruction,
};

mod common;

#[test]
fn test_vote_update_validator_identity() {
    let (vote_pubkey, _authorized_voter, authorized_withdrawer, vote_account) =
        create_test_account_with_authorized();

    let original_block_revenue_collector = {
        let vote_state = deserialize_vote_state_for_test(vote_account.data(), &vote_pubkey);
        vote_state.as_ref_v4().block_revenue_collector
    };

    let node_pubkey = solana_pubkey::new_rand();
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateValidatorIdentity).unwrap();
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (node_pubkey, AccountSharedData::default()),
        (authorized_withdrawer, AccountSharedData::default()),
    ];
    let mut instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: node_pubkey,
            is_signer: true,
            is_writable: false,
        },
        AccountMeta {
            pubkey: authorized_withdrawer,
            is_signer: true,
            is_writable: false,
        },
    ];

    let features = VoteProgramFeatures::default();

    // should fail, node_pubkey didn't sign the transaction
    instruction_accounts[1].is_signer = false;
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
    instruction_accounts[1].is_signer = true;
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_ne!(*vote_state.node_pubkey(), node_pubkey);

    // should fail, authorized_withdrawer didn't sign the transaction
    instruction_accounts[2].is_signer = false;
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
    instruction_accounts[2].is_signer = true;
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_ne!(*vote_state.node_pubkey(), node_pubkey);

    // should pass
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts,
        instruction_accounts,
        Ok(()),
    );
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(*vote_state.node_pubkey(), node_pubkey);
    assert_eq!(
        vote_state.as_ref_v4().block_revenue_collector,
        original_block_revenue_collector,
    );
}
