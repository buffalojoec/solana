use {
    common::{
        VoteProgramFeatures, create_default_clock_account, create_default_rent_account,
        create_sysvar_account, create_test_account_with_authorized,
        deserialize_vote_state_for_test, process_instruction, process_instruction_with_cu_check,
        vote_state_size_of,
    },
    solana_account::{
        AccountSharedData, ReadableAccount, WritableAccount, state_traits::StateMutWincode,
    },
    solana_clock::Clock,
    solana_epoch_schedule::EpochSchedule,
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_sdk_ids::sysvar,
    solana_vote_program::{
        id,
        vote_instruction::VoteInstruction,
        vote_processor::{BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS, DEFAULT_COMPUTE_UNITS},
        vote_state::{
            VoteAuthorize, VoteAuthorizeCheckedWithSeedArgs, VoteAuthorizeWithSeedArgs, VoteInit,
            VoteInitV2, VoteStateVersions, create_bls_pubkey_and_proof_of_possession,
        },
    },
};

mod common;

#[test]
fn test_deinitialized_account_full_lifecycle_v4() {
    // Full lifecycle: withdraw all lamports to deinitialize a V4
    // account, verify instructions are rejected on the zeroed
    // account, then re-initialize and confirm no residual state.
    let (vote_pubkey, _authorized_voter, authorized_withdrawer, vote_account) =
        create_test_account_with_authorized();
    let lamports = vote_account.lamports();

    let features = VoteProgramFeatures {
        ..Default::default()
    };

    let recipient_pubkey = solana_pubkey::new_rand();
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (recipient_pubkey, AccountSharedData::default()),
        (authorized_withdrawer, AccountSharedData::default()),
        (sysvar::rent::id(), create_default_rent_account()),
        (sysvar::clock::id(), create_default_clock_account()),
    ];
    let instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: recipient_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: authorized_withdrawer,
            is_signer: true,
            is_writable: false,
        },
    ];

    // Withdraw all lamports to deinitialize.
    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports)).unwrap(),
        transaction_accounts,
        instruction_accounts,
        Ok(()),
    );
    let deinitialized_vote_account = &accounts[0];

    // Account data should be all zeros.
    assert!(deinitialized_vote_account.data().iter().all(|&b| b == 0));

    // Authorize should fail on the deinitialized account.
    let clock_account = create_sysvar_account(&Clock {
        epoch: 100,
        ..Clock::default()
    });
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Authorize(
            solana_pubkey::new_rand(),
            VoteAuthorize::Voter,
        ))
        .unwrap(),
        vec![
            (vote_pubkey, deinitialized_vote_account.clone()),
            (sysvar::clock::id(), clock_account),
            (authorized_withdrawer, AccountSharedData::default()),
        ],
        vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
        ],
        Err(InstructionError::InvalidAccountData),
    );

    // Re-initialize with new fields.
    let new_node_pubkey = solana_pubkey::new_rand();
    let new_vote_init = VoteInit {
        node_pubkey: new_node_pubkey,
        authorized_voter: solana_pubkey::new_rand(),
        authorized_withdrawer: solana_pubkey::new_rand(),
        commission: 10,
    };
    // Fund the account for rent exemption.
    let mut funded_account = deinitialized_vote_account.clone();
    let rent = Rent::default();
    funded_account.set_lamports(rent.minimum_balance(funded_account.data().len()));

    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::InitializeAccount(new_vote_init)).unwrap(),
        vec![
            (vote_pubkey, funded_account),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
            (new_node_pubkey, AccountSharedData::default()),
        ],
        vec![
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
                pubkey: new_node_pubkey,
                is_signer: true,
                is_writable: false,
            },
        ],
        Ok(()),
    );

    // Verify the re-initialized account is a clean V4.
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(*vote_state.node_pubkey(), new_node_pubkey);
    assert_eq!(
        *vote_state.authorized_withdrawer(),
        new_vote_init.authorized_withdrawer
    );
    assert_eq!(vote_state.commission(), 10);
    assert!(vote_state.latest_vote_slot().is_none());
    assert!(vote_state.epoch_credits().is_empty());
}

#[test]
fn test_uninitialized_v3_blocked_under_v4() {
    // An uninitialized V3 account (empty authorized_voters, padded
    // to V4 size) is rejected by all instructions under V4.
    let vote_pubkey = solana_pubkey::new_rand();

    // Create an uninitialized V3: discriminant 2, empty authorized_voters.
    let uninitialized_v3 = VoteStateVersions::V3(Box::default());
    let serialized = wincode::serialize(&uninitialized_v3).unwrap();
    let target_len = vote_state_size_of();
    let mut data = vec![0u8; target_len];
    data[..serialized.len()].copy_from_slice(&serialized);

    let rent = Rent::default();
    let lamports = rent.minimum_balance(target_len);
    let vote_account = AccountSharedData::new_with_data(lamports, data, &id());

    let authorized_withdrawer = solana_pubkey::new_rand();
    let features = VoteProgramFeatures::all_enabled();

    // Authorize should fail.
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Authorize(
            solana_pubkey::new_rand(),
            VoteAuthorize::Voter,
        ))
        .unwrap(),
        vec![
            (vote_pubkey, vote_account.clone()),
            (sysvar::clock::id(), create_default_clock_account()),
            (authorized_withdrawer, AccountSharedData::default()),
        ],
        vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
            AccountMeta {
                pubkey: authorized_withdrawer,
                is_signer: true,
                is_writable: false,
            },
        ],
        Err(InstructionError::UninitializedAccount),
    );

    // UpdateCommission should fail.
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::UpdateCommission(50)).unwrap(),
        vec![
            (vote_pubkey, vote_account.clone()),
            (authorized_withdrawer, AccountSharedData::default()),
            (sysvar::clock::id(), create_default_clock_account()),
            (
                sysvar::epoch_schedule::id(),
                create_sysvar_account(&EpochSchedule::without_warmup()),
            ),
        ],
        vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: authorized_withdrawer,
                is_signer: true,
                is_writable: false,
            },
        ],
        Err(InstructionError::UninitializedAccount),
    );

    // Re-initialize escape hatch: InitializeAccount should succeed.
    let new_node = solana_pubkey::new_rand();
    let vote_init = VoteInit {
        node_pubkey: new_node,
        authorized_voter: solana_pubkey::new_rand(),
        authorized_withdrawer: solana_pubkey::new_rand(),
        commission: 5,
    };
    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::InitializeAccount(vote_init)).unwrap(),
        vec![
            (vote_pubkey, vote_account.clone()),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
            (new_node, AccountSharedData::default()),
        ],
        vec![
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
                pubkey: new_node,
                is_signer: true,
                is_writable: false,
            },
        ],
        Ok(()),
    );

    // Verify re-initialized as V4.
    let versioned: VoteStateVersions = accounts[0].state().unwrap();
    assert!(matches!(versioned, VoteStateVersions::V4(_)));
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(*vote_state.node_pubkey(), new_node);
    assert_eq!(vote_state.commission(), 5);

    // InitializeAccountV2 should also work for the escape hatch.
    let new_node = solana_pubkey::new_rand();
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let inflation_rewards_collector = solana_pubkey::new_rand();
    let block_revenue_collector = solana_pubkey::new_rand();
    let vote_init_v2 = VoteInitV2 {
        node_pubkey: new_node,
        authorized_voter: solana_pubkey::new_rand(),
        authorized_voter_bls_pubkey: bls_pubkey,
        authorized_voter_bls_proof_of_possession: bls_proof_of_possession,
        authorized_withdrawer: solana_pubkey::new_rand(),
        inflation_rewards_commission_bps: 1_234,
        block_revenue_commission_bps: 5_678,
    };

    let collector_account =
        AccountSharedData::empty(rent.minimum_balance(0), &solana_sdk_ids::system_program::id());

    let accounts = process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::InitializeAccountV2(vote_init_v2)).unwrap(),
        vec![
            (vote_pubkey, vote_account),
            (new_node, AccountSharedData::default()),
            (inflation_rewards_collector, collector_account.clone()),
            (block_revenue_collector, collector_account),
            (sysvar::rent::id(), create_default_rent_account()),
            (sysvar::clock::id(), create_default_clock_account()),
        ],
        vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: new_node,
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
        ],
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );

    // Verify re-initialized as V4 with the v2-specific fields.
    let versioned: VoteStateVersions = accounts[0].state().unwrap();
    assert!(matches!(versioned, VoteStateVersions::V4(_)));
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    let v4 = vote_state.as_ref_v4();
    assert_eq!(v4.node_pubkey, new_node);
    assert_eq!(v4.bls_pubkey_compressed, Some(bls_pubkey));
    assert_eq!(v4.inflation_rewards_commission_bps, 1_234);
    assert_eq!(v4.block_revenue_commission_bps, 5_678);
    assert_eq!(v4.inflation_rewards_collector, inflation_rewards_collector);
    assert_eq!(v4.block_revenue_collector, block_revenue_collector);
}

// Explicitly covers uninitialized vote accounts for instructions.
// `test_vote_signature` above covered:
// * Vote
// * UpdateVoteState
// * CompactUpdateVoteState
// * TowerSync
#[test]
fn test_uninitialized_vote_account() {
    // Set up uninitialized vote account.
    let vote_pubkey = solana_pubkey::new_rand();
    let vote_account = AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());

    let expected_error = InstructionError::InvalidAccountData;

    let features = VoteProgramFeatures {
        ..Default::default()
    };

    // VoteInstruction::Authorize
    {
        let new_authorized_pubkey = solana_pubkey::new_rand();

        let instruction_data = wincode::serialize(&VoteInstruction::Authorize(
            new_authorized_pubkey,
            VoteAuthorize::Voter,
        ))
        .unwrap();

        let transaction_accounts = vec![
            (vote_pubkey, vote_account),
            (sysvar::clock::id(), create_default_clock_account()),
        ];

        let instruction_accounts = vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
        ];

        process_instruction(
            features,
            &instruction_data,
            transaction_accounts,
            instruction_accounts,
            Err(expected_error.clone()),
        );
    }

    // VoteInstruction::AuthorizeWithSeed
    {
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
        let current_authority_base_key = Pubkey::new_unique();
        let current_authority_owner = Pubkey::new_unique();
        let new_authority_pubkey = Pubkey::new_unique();

        let instruction_data = wincode::serialize(&VoteInstruction::AuthorizeWithSeed(
            VoteAuthorizeWithSeedArgs {
                authorization_type: VoteAuthorize::Voter,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: String::from("SEED"),
                new_authority: new_authority_pubkey,
            },
        ))
        .unwrap();

        let transaction_accounts = vec![
            (vote_pubkey, vote_account),
            (sysvar::clock::id(), create_default_clock_account()),
            (current_authority_base_key, AccountSharedData::default()),
        ];

        let instruction_accounts = vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
            AccountMeta {
                pubkey: current_authority_base_key,
                is_signer: true,
                is_writable: false,
            },
        ];

        process_instruction(
            features,
            &instruction_data,
            transaction_accounts,
            instruction_accounts,
            Err(expected_error.clone()),
        );
    }

    // VoteInstruction::AuthorizeCheckedWithSeed
    {
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
        let current_authority_base_key = Pubkey::new_unique();
        let current_authority_owner = Pubkey::new_unique();
        let new_authority_pubkey = Pubkey::new_unique();

        let instruction_data = wincode::serialize(&VoteInstruction::AuthorizeCheckedWithSeed(
            VoteAuthorizeCheckedWithSeedArgs {
                authorization_type: VoteAuthorize::Voter,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: String::from("SEED"),
            },
        ))
        .unwrap();

        let transaction_accounts = vec![
            (vote_pubkey, vote_account),
            (sysvar::clock::id(), create_default_clock_account()),
            (current_authority_base_key, AccountSharedData::default()),
            (new_authority_pubkey, AccountSharedData::default()),
        ];

        let instruction_accounts = vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
            AccountMeta {
                pubkey: current_authority_base_key,
                is_signer: true,
                is_writable: false,
            },
            AccountMeta {
                pubkey: new_authority_pubkey,
                is_signer: true,
                is_writable: false,
            },
        ];

        process_instruction(
            features,
            &instruction_data,
            transaction_accounts,
            instruction_accounts,
            Err(expected_error.clone()),
        );
    }

    // VoteInstruction::UpdateValidatorIdentity
    {
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
        let node_pubkey = Pubkey::new_unique();
        let authorized_withdrawer = Pubkey::new_unique();

        let instruction_data =
            wincode::serialize(&VoteInstruction::UpdateValidatorIdentity).unwrap();

        let transaction_accounts = vec![
            (vote_pubkey, vote_account),
            (node_pubkey, AccountSharedData::default()),
            (authorized_withdrawer, AccountSharedData::default()),
        ];

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
                pubkey: authorized_withdrawer,
                is_signer: true,
                is_writable: false,
            },
        ];

        process_instruction(
            features,
            &instruction_data,
            transaction_accounts,
            instruction_accounts,
            Err(expected_error.clone()),
        );
    }

    // VoteInstruction::UpdateCommission
    {
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
        let authorized_withdrawer = Pubkey::new_unique();

        let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommission(42)).unwrap();

        let transaction_accounts = vec![
            (vote_pubkey, vote_account),
            (authorized_withdrawer, AccountSharedData::default()),
            (
                sysvar::clock::id(),
                create_sysvar_account(&Clock::default()),
            ),
            (
                sysvar::epoch_schedule::id(),
                create_sysvar_account(&EpochSchedule::without_warmup()),
            ),
        ];

        let instruction_accounts = vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: authorized_withdrawer,
                is_signer: true,
                is_writable: false,
            },
        ];

        process_instruction(
            features,
            &instruction_data,
            transaction_accounts,
            instruction_accounts,
            Err(expected_error.clone()),
        );
    }

    // VoteInstruction::Withdraw
    {
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
        let recipient = Pubkey::new_unique();

        let instruction_data = wincode::serialize(&VoteInstruction::Withdraw(10)).unwrap();

        let transaction_accounts = vec![
            (vote_pubkey, vote_account),
            (recipient, AccountSharedData::default()),
            (sysvar::clock::id(), create_default_clock_account()),
            (sysvar::rent::id(), create_default_rent_account()),
        ];

        let instruction_accounts = vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: recipient,
                is_signer: false,
                is_writable: true,
            },
        ];

        process_instruction(
            features,
            &instruction_data,
            transaction_accounts,
            instruction_accounts,
            Err(expected_error.clone()),
        );
    }

    // VoteInstruction::AuthorizeChecked
    {
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
        let authorized_pubkey = Pubkey::new_unique();
        let new_authorized_pubkey = Pubkey::new_unique();

        let instruction_data =
            wincode::serialize(&VoteInstruction::AuthorizeChecked(VoteAuthorize::Voter)).unwrap();

        let transaction_accounts = vec![
            (vote_pubkey, vote_account),
            (sysvar::clock::id(), create_default_clock_account()),
            (authorized_pubkey, AccountSharedData::default()),
            (new_authorized_pubkey, AccountSharedData::default()),
        ];

        let instruction_accounts = vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
            AccountMeta {
                pubkey: authorized_pubkey,
                is_signer: true,
                is_writable: false,
            },
            AccountMeta {
                pubkey: new_authorized_pubkey,
                is_signer: true,
                is_writable: false,
            },
        ];

        process_instruction(
            features,
            &instruction_data,
            transaction_accounts,
            instruction_accounts,
            Err(expected_error),
        );
    }
}
