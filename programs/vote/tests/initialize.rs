use {
    common::{
        VoteProgramFeatures, create_default_clock_account, create_default_rent_account,
        deserialize_vote_state_for_test, process_instruction, process_instruction_with_cu_check,
        vote_state_size_of,
    },
    solana_account::{AccountSharedData, ReadableAccount},
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_sdk_ids::sysvar,
    solana_vote_interface::state::{
        BLS_PROOF_OF_POSSESSION_COMPRESSED_SIZE, BLS_PUBLIC_KEY_COMPRESSED_SIZE,
    },
    solana_vote_program::{
        id,
        vote_instruction::VoteInstruction,
        vote_processor::{BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS, DEFAULT_COMPUTE_UNITS},
        vote_state::{VoteInit, VoteInitV2, VoteStateV4, create_bls_pubkey_and_proof_of_possession},
    },
    test_case::test_matrix,
};

mod common;

#[test_matrix(
    [false, true],
    [false, true]
)]
fn test_initialize_vote_account(block_revenue_sharing: bool, vote_account_initialize_v2: bool) {
    let vote_pubkey = solana_pubkey::new_rand();
    let vote_account = AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
    let node_pubkey = solana_pubkey::new_rand();
    let node_account = AccountSharedData::default();
    let instruction_data = wincode::serialize(&VoteInstruction::InitializeAccount(VoteInit {
        node_pubkey,
        authorized_voter: vote_pubkey,
        authorized_withdrawer: vote_pubkey,
        commission: 0,
    }))
    .unwrap();
    let mut instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: sysvar::rent::id(),
            is_signer: false,
            is_writable: false,
        },
        AccountMeta {
            pubkey: sysvar::clock::id(),
            is_signer: false,
            is_writable: false,
        },
        AccountMeta {
            pubkey: node_pubkey,
            is_signer: true,
            is_writable: false,
        },
    ];

    let features = VoteProgramFeatures {
        block_revenue_sharing,
        vote_account_initialize_v2,
    };

    let accounts = process_instruction(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, vote_account.clone()),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
            (node_pubkey, node_account.clone()),
        ],
        instruction_accounts.clone(),
        Ok(()),
    );

    // reinit should fail
    process_instruction(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, accounts[0].clone()),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
            (node_pubkey, accounts[3].clone()),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::AccountAlreadyInitialized),
    );

    // init should fail, account is too big
    process_instruction(
        features,
        &instruction_data,
        vec![
            (
                vote_pubkey,
                AccountSharedData::new_with_data(100, vec![0; 2 * vote_state_size_of()], &id()),
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
            (node_pubkey, node_account.clone()),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::InvalidAccountData),
    );

    // init should fail, node_pubkey didn't sign the transaction
    instruction_accounts[3].is_signer = false;
    process_instruction(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, vote_account),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
            (node_pubkey, node_account),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
}

#[test_matrix(
    [false, true],
    [false, true]
)]
fn test_initialize_vote_account_v2(block_revenue_sharing: bool, vote_account_initialize_v2: bool) {
    let vote_pubkey = solana_pubkey::new_rand();
    let vote_account = AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
    let node_pubkey = solana_pubkey::new_rand();
    let node_account = AccountSharedData::default();
    let authorized_voter = solana_pubkey::new_rand();
    let authorized_withdrawer = solana_pubkey::new_rand();
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let inflation_rewards_collector = solana_pubkey::new_rand();
    let inflation_rewards_collector_account =
        AccountSharedData::empty(0, &solana_sdk_ids::system_program::id());
    let block_revenue_collector = solana_pubkey::new_rand();
    let block_revenue_collector_account =
        AccountSharedData::empty(0, &solana_sdk_ids::system_program::id());
    let inflation_rewards_commission_bps = 1_234;
    let block_revenue_commission_bps = 5_678;
    let instruction_data = wincode::serialize(&VoteInstruction::InitializeAccountV2(VoteInitV2 {
        node_pubkey,
        authorized_voter,
        authorized_voter_bls_pubkey: bls_pubkey,
        authorized_voter_bls_proof_of_possession: bls_proof_of_possession,
        authorized_withdrawer,
        inflation_rewards_commission_bps,
        block_revenue_commission_bps,
    }))
    .unwrap();
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
            pubkey: inflation_rewards_collector,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: block_revenue_collector,
            is_signer: false,
            is_writable: true,
        },
    ];

    let features = VoteProgramFeatures {
        block_revenue_sharing,
        vote_account_initialize_v2,
    };

    let all_v2_features_enabled = block_revenue_sharing && vote_account_initialize_v2;

    // If any v2 feature is disabled, the new instruction should be rejected
    if !all_v2_features_enabled {
        process_instruction(
            features,
            &instruction_data,
            vec![
                (vote_pubkey, vote_account),
                (node_pubkey, node_account),
                (
                    inflation_rewards_collector,
                    inflation_rewards_collector_account,
                ),
                (block_revenue_collector, block_revenue_collector_account),
                (sysvar::rent::id(), create_default_rent_account()),
                (sysvar::clock::id(), create_default_clock_account()),
            ],
            instruction_accounts.clone(),
            Err(InstructionError::InvalidInstructionData),
        );
        return;
    }

    // Verify every field in the V4 state matches VoteInitV2 and the
    // collector accounts.
    let assert_v4_fields =
        |vote_account: &AccountSharedData,
         expected_inflation_rewards_collector: Pubkey,
         expected_block_revenue_collector: Pubkey| {
            let v4 = deserialize_vote_state_for_test(vote_account.data(), &vote_pubkey);
            let v4 = v4.as_ref_v4();
            assert_eq!(v4.node_pubkey, node_pubkey);
            assert_eq!(v4.authorized_withdrawer, authorized_withdrawer);
            assert_eq!(v4.bls_pubkey_compressed, Some(bls_pubkey));
            assert_eq!(
                v4.inflation_rewards_commission_bps,
                inflation_rewards_commission_bps
            );
            assert_eq!(
                v4.inflation_rewards_collector,
                expected_inflation_rewards_collector
            );
            assert_eq!(v4.block_revenue_commission_bps, block_revenue_commission_bps);
            assert_eq!(v4.block_revenue_collector, expected_block_revenue_collector);
            assert_eq!(v4.pending_delegator_rewards, 0);
            assert!(v4.votes.is_empty());
            assert!(v4.epoch_credits.is_empty());
            assert_eq!(v4.root_slot, None);
        };

    let accounts = process_instruction_with_cu_check(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, vote_account.clone()),
            (node_pubkey, node_account.clone()),
            (
                inflation_rewards_collector,
                inflation_rewards_collector_account.clone(),
            ),
            (
                block_revenue_collector,
                block_revenue_collector_account.clone(),
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        instruction_accounts.clone(),
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    assert_v4_fields(
        &accounts[0],
        inflation_rewards_collector,
        block_revenue_collector,
    );

    // reinit should fail
    process_instruction(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, accounts[0].clone()),
            (node_pubkey, accounts[1].clone()),
            (
                inflation_rewards_collector,
                inflation_rewards_collector_account.clone(),
            ),
            (
                block_revenue_collector,
                block_revenue_collector_account.clone(),
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::AccountAlreadyInitialized),
    );

    // init should fail, account is too big
    process_instruction(
        features,
        &instruction_data,
        vec![
            (
                vote_pubkey,
                AccountSharedData::new_with_data(100, vec![0; 2 * vote_state_size_of()], &id()),
            ),
            (node_pubkey, node_account.clone()),
            (
                inflation_rewards_collector,
                inflation_rewards_collector_account.clone(),
            ),
            (
                block_revenue_collector,
                block_revenue_collector_account.clone(),
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::InvalidAccountData),
    );

    // init should fail, node_pubkey didn't sign the transaction
    instruction_accounts[1].is_signer = false;
    process_instruction(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, vote_account.clone()),
            (node_pubkey, node_account.clone()),
            (
                inflation_rewards_collector,
                inflation_rewards_collector_account.clone(),
            ),
            (
                block_revenue_collector,
                block_revenue_collector_account.clone(),
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
    instruction_accounts[1].is_signer = true;

    // init should fail, fewer than 4 instruction accounts
    process_instruction(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, vote_account.clone()),
            (node_pubkey, node_account.clone()),
            (
                inflation_rewards_collector,
                inflation_rewards_collector_account.clone(),
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        instruction_accounts[..3].to_vec(),
        Err(InstructionError::MissingAccount),
    );

    // init should pass with both collectors aliased to the vote account.
    let mut aliased_instruction_accounts = instruction_accounts.clone();
    aliased_instruction_accounts[2].pubkey = vote_pubkey;
    aliased_instruction_accounts[3].pubkey = vote_pubkey;
    let accounts = process_instruction_with_cu_check(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, vote_account.clone()),
            (node_pubkey, node_account.clone()),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        aliased_instruction_accounts,
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    assert_v4_fields(&accounts[0], vote_pubkey, vote_pubkey);

    // init should pass with both collectors aliased to the same external
    // account.
    let mut aliased_instruction_accounts = instruction_accounts.clone();
    aliased_instruction_accounts[2].pubkey = inflation_rewards_collector;
    aliased_instruction_accounts[3].pubkey = inflation_rewards_collector;
    let accounts = process_instruction_with_cu_check(
        features,
        &instruction_data,
        vec![
            (vote_pubkey, vote_account),
            (node_pubkey, node_account),
            (
                inflation_rewards_collector,
                inflation_rewards_collector_account,
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        aliased_instruction_accounts,
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    assert_v4_fields(
        &accounts[0],
        inflation_rewards_collector,
        inflation_rewards_collector,
    );
}

#[test]
fn test_initialize_vote_account_v2_bad_proof_of_possession() {
    let vote_pubkey = solana_pubkey::new_rand();
    let vote_account =
        AccountSharedData::new_with_data(100, vec![0; VoteStateV4::size_of()], &id());
    let node_pubkey = solana_pubkey::new_rand();
    let node_account = AccountSharedData::default();
    let inflation_rewards_collector = solana_pubkey::new_rand();
    let inflation_rewards_collector_account =
        AccountSharedData::empty(0, &solana_sdk_ids::system_program::id());
    let block_revenue_collector = solana_pubkey::new_rand();
    let block_revenue_collector_account =
        AccountSharedData::empty(0, &solana_sdk_ids::system_program::id());
    let instruction_with_bad_pop =
        wincode::serialize(&VoteInstruction::InitializeAccountV2(VoteInitV2 {
            node_pubkey,
            authorized_voter: vote_pubkey,
            authorized_withdrawer: vote_pubkey,
            authorized_voter_bls_pubkey: [1u8; BLS_PUBLIC_KEY_COMPRESSED_SIZE],
            authorized_voter_bls_proof_of_possession: [2u8; BLS_PROOF_OF_POSSESSION_COMPRESSED_SIZE],
            ..Default::default()
        }))
        .unwrap();
    let instruction_accounts = vec![
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
            pubkey: inflation_rewards_collector,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: block_revenue_collector,
            is_signer: false,
            is_writable: true,
        },
    ];
    process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_with_bad_pop,
        vec![
            (vote_pubkey, vote_account),
            (node_pubkey, node_account),
            (
                inflation_rewards_collector,
                inflation_rewards_collector_account.clone(),
            ),
            (
                block_revenue_collector,
                block_revenue_collector_account.clone(),
            ),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        instruction_accounts,
        Err(InstructionError::InvalidArgument),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
}
