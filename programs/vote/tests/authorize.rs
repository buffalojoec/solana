use {
    common::{
        VoteProgramFeatures, create_default_account, create_default_clock_account,
        create_sysvar_account, create_test_account, create_test_account_no_bls_key,
        deserialize_vote_state_for_test, process_instruction, process_instruction_as_one_arg,
        process_instruction_with_cu_check,
    },
    solana_account::{AccountSharedData, ReadableAccount, state_traits::StateMutWincode},
    solana_clock::Clock,
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_sdk_ids::sysvar,
    solana_vote_interface::state::{
        BLS_PROOF_OF_POSSESSION_COMPRESSED_SIZE, BLS_PUBLIC_KEY_COMPRESSED_SIZE, VoterWithBLSArgs,
    },
    solana_vote_program::{
        id,
        vote_error::VoteError,
        vote_instruction::{VoteInstruction, authorize_checked},
        vote_processor::{BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS, DEFAULT_COMPUTE_UNITS},
        vote_state::{
            VoteAuthorize, VoteInit, VoteStateV4, VoteStateVersions,
            create_bls_pubkey_and_proof_of_possession,
        },
    },
};

mod common;

#[test]
fn test_authorize_voter() {
    let (vote_pubkey, vote_account) = create_test_account_no_bls_key();
    let authorized_voter_pubkey = solana_pubkey::new_rand();
    let clock = Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let instruction_data = wincode::serialize(&VoteInstruction::Authorize(
        authorized_voter_pubkey,
        VoteAuthorize::Voter,
    ))
    .unwrap();

    let mut transaction_accounts = vec![
        (vote_pubkey, vote_account.clone()),
        (sysvar::clock::id(), clock_account.clone()),
        (authorized_voter_pubkey, AccountSharedData::default()),
    ];
    let mut instruction_accounts = vec![
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

    let features = VoteProgramFeatures::default();

    // should fail, unsigned
    instruction_accounts[0].is_signer = false;
    process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
    instruction_accounts[0].is_signer = true;

    // should pass
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );

    // should fail, already set an authorized voter earlier for leader_schedule_epoch == 2
    transaction_accounts[0] = (vote_pubkey, accounts[0].clone());
    process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(VoteError::TooSoonToReauthorize.into()),
    );

    // should pass, verify authorized_voter_pubkey can authorize authorized_voter_pubkey ;)
    instruction_accounts[0].is_signer = false;
    instruction_accounts.push(AccountMeta {
        pubkey: authorized_voter_pubkey,
        is_signer: true,
        is_writable: false,
    });
    let clock = Clock {
        // The authorized voter was set when leader_schedule_epoch == 2, so will
        // take effect when epoch == 3
        epoch: 3,
        leader_schedule_epoch: 4,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    transaction_accounts[1] = (sysvar::clock::id(), clock_account);
    process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    instruction_accounts[0].is_signer = true;
    instruction_accounts.pop();
}

#[test]
fn test_authorize_voter_with_bls() {
    agave_logger::setup();
    let (vote_pubkey, vote_account) = create_test_account();
    let authorized_voter_pubkey = solana_pubkey::new_rand();
    let clock = Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let instruction_data = wincode::serialize(&VoteInstruction::Authorize(
        authorized_voter_pubkey,
        VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
            bls_pubkey,
            bls_proof_of_possession,
        }),
    ))
    .unwrap();

    let mut transaction_accounts = vec![
        (vote_pubkey, vote_account.clone()),
        (sysvar::clock::id(), clock_account.clone()),
        (authorized_voter_pubkey, AccountSharedData::default()),
    ];
    let mut instruction_accounts = vec![
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

    let features = VoteProgramFeatures::default();

    // Legacy authorization is accepted when the account has no BLS key.
    let (new_vote_pubkey, vote_account_no_bls_key) = create_test_account_no_bls_key();
    let new_authorized_voter_pubkey = solana_pubkey::new_rand();
    let old_instruction_data = wincode::serialize(&VoteInstruction::Authorize(
        new_authorized_voter_pubkey,
        VoteAuthorize::Voter,
    ))
    .unwrap();
    process_instruction(
        features,
        &old_instruction_data,
        vec![
            (new_vote_pubkey, vote_account_no_bls_key),
            (sysvar::clock::id(), clock_account.clone()),
            (new_authorized_voter_pubkey, AccountSharedData::default()),
        ],
        vec![
            AccountMeta {
                pubkey: new_vote_pubkey,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
        ],
        Ok(()),
    );
    // However, once the BLS key is set, the old instruction should be rejected
    let (new_vote_pubkey, vote_account_with_bls_key) = create_test_account();
    let new_authorized_voter_pubkey = solana_pubkey::new_rand();
    let old_instruction_data = wincode::serialize(&VoteInstruction::Authorize(
        new_authorized_voter_pubkey,
        VoteAuthorize::Voter,
    ))
    .unwrap();
    process_instruction(
        features,
        &old_instruction_data,
        vec![
            (new_vote_pubkey, vote_account_with_bls_key),
            (sysvar::clock::id(), clock_account),
            (new_authorized_voter_pubkey, AccountSharedData::default()),
        ],
        vec![
            AccountMeta {
                pubkey: new_vote_pubkey,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: sysvar::clock::id(),
                is_signer: false,
                is_writable: false,
            },
        ],
        Err(InstructionError::InvalidInstructionData),
    );

    // should fail, unsigned
    instruction_accounts[0].is_signer = false;
    process_instruction_with_cu_check(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    instruction_accounts[0].is_signer = true;

    // should pass
    let accounts = process_instruction_with_cu_check(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );

    // should fail, already set an authorized voter earlier for leader_schedule_epoch == 2
    transaction_accounts[0] = (vote_pubkey, accounts[0].clone());
    process_instruction_with_cu_check(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(VoteError::TooSoonToReauthorize.into()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );

    // should pass, verify authorized_voter_pubkey can authorize authorized_voter_pubkey ;)
    instruction_accounts[0].is_signer = false;
    instruction_accounts.push(AccountMeta {
        pubkey: authorized_voter_pubkey,
        is_signer: true,
        is_writable: false,
    });
    let clock = Clock {
        // The authorized voter was set when leader_schedule_epoch == 2, so will
        // take effect when epoch == 3
        epoch: 3,
        leader_schedule_epoch: 4,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    transaction_accounts[1] = (sysvar::clock::id(), clock_account);
    process_instruction_with_cu_check(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    instruction_accounts[0].is_signer = true;
    instruction_accounts.pop();
}

#[test]
fn test_authorize_voter_with_bls_bad_proof_of_possession() {
    let (vote_pubkey, vote_account) = create_test_account();
    let authorized_voter_pubkey = solana_pubkey::new_rand();
    let clock = Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (sysvar::clock::id(), clock_account),
        (authorized_voter_pubkey, AccountSharedData::default()),
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

    // Test that bad proof of possession fails authorization
    let instruction_data = wincode::serialize(&VoteInstruction::Authorize(
        authorized_voter_pubkey,
        VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
            bls_pubkey: [1u8; BLS_PUBLIC_KEY_COMPRESSED_SIZE],
            bls_proof_of_possession: [2u8; BLS_PROOF_OF_POSSESSION_COMPRESSED_SIZE],
        }),
    ))
    .unwrap();
    process_instruction_with_cu_check(
        VoteProgramFeatures::default(),
        &instruction_data,
        transaction_accounts,
        instruction_accounts,
        Err(InstructionError::InvalidArgument),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
}

// Tests (and essentially documents) BLS pubkey rotation characteristics
// by testing the end-to-end behavior of `Authorize::VoterWithBls` in the
// program.
//
// All rotations reuse the existing authorized voter to simulate an
// operator's desire to only change their BLS pubkey.
//
// TL;DR: Since authorized voter rotations are capped at once per epoch,
// so too are BLS pubkey rotations.
#[test]
fn test_bls_pubkey_rotation() {
    let features = VoteProgramFeatures::default();

    // Start out with no BLS pubkey set.
    // Authorized voter is already set to vote pubkey.
    let (vote_pubkey, vote_account) = create_test_account_no_bls_key();
    let authorized_voter = vote_pubkey;

    // Two distinct BLS keypairs for rotation.
    let (bls_1, pop_1) = create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let (bls_2, pop_2) = create_bls_pubkey_and_proof_of_possession(&vote_pubkey);

    let ix_accounts = vec![
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

    // Epoch: 1
    // Leader Schedule Epoch: 2
    //   Rotation schedules for target epoch 3
    //   BLS pubkey is set immediately
    let clock_epoch_1 = create_sysvar_account(&Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    });
    let accounts = process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::Authorize(
            authorized_voter,
            VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
                bls_pubkey: bls_1,
                bls_proof_of_possession: pop_1,
            }),
        ))
        .unwrap(),
        vec![
            (vote_pubkey, vote_account),
            (sysvar::clock::id(), clock_epoch_1.clone()),
        ],
        ix_accounts.clone(),
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    let v4 = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(v4.as_ref_v4().bls_pubkey_compressed, Some(bls_1));

    // == Same epoch again ==
    // Epoch: 1
    // Leader Schedule Epoch: 2
    //   Rotation fails with `TooSoonToReauthorize`
    //   BLS pubkey is NOT set
    //   34,500 CUs still charged
    let accounts = process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::Authorize(
            authorized_voter,
            VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
                bls_pubkey: bls_2,
                bls_proof_of_possession: pop_2,
            }),
        ))
        .unwrap(),
        vec![
            (vote_pubkey, accounts[0].clone()),
            (sysvar::clock::id(), clock_epoch_1),
        ],
        ix_accounts.clone(),
        Err(VoteError::TooSoonToReauthorize.into()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    let v4 = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(v4.as_ref_v4().bls_pubkey_compressed, Some(bls_1)); // <-- Still BLS key 1

    // == New epoch ==
    // Epoch: 2
    // Leader Schedule Epoch: 3
    //   Rotation schedules for target epoch 4
    //   BLS pubkey is set immediately
    let clock_epoch_2 = create_sysvar_account(&Clock {
        epoch: 2,
        leader_schedule_epoch: 3,
        ..Clock::default()
    });
    let accounts = process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::Authorize(
            authorized_voter,
            VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
                bls_pubkey: bls_2,
                bls_proof_of_possession: pop_2,
            }),
        ))
        .unwrap(),
        vec![
            (vote_pubkey, accounts[0].clone()),
            (sysvar::clock::id(), clock_epoch_2),
        ],
        ix_accounts.clone(),
        Ok(()),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    let v4 = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(v4.as_ref_v4().bls_pubkey_compressed, Some(bls_2)); // <-- Now BLS key 2
}

#[test]
fn test_authorize_withdrawer() {
    let (vote_pubkey, vote_account) = create_test_account_no_bls_key();
    let authorized_withdrawer_pubkey = solana_pubkey::new_rand();
    let instruction_data = wincode::serialize(&VoteInstruction::Authorize(
        authorized_withdrawer_pubkey,
        VoteAuthorize::Withdrawer,
    ))
    .unwrap();
    let mut transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (sysvar::clock::id(), create_default_clock_account()),
        (authorized_withdrawer_pubkey, AccountSharedData::default()),
    ];
    let mut instruction_accounts = vec![
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

    let features = VoteProgramFeatures {
        ..Default::default()
    };

    // should fail, unsigned
    instruction_accounts[0].is_signer = false;
    process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
    instruction_accounts[0].is_signer = true;

    // should pass
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );

    // should pass, verify authorized_withdrawer can authorize authorized_withdrawer ;)
    instruction_accounts[0].is_signer = false;
    instruction_accounts.push(AccountMeta {
        pubkey: authorized_withdrawer_pubkey,
        is_signer: true,
        is_writable: false,
    });
    transaction_accounts[0] = (vote_pubkey, accounts[0].clone());
    process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );

    // should pass, verify authorized_withdrawer can authorize a new authorized_voter
    let authorized_voter_pubkey = solana_pubkey::new_rand();
    transaction_accounts.push((authorized_voter_pubkey, AccountSharedData::default()));
    let instruction_data = wincode::serialize(&VoteInstruction::Authorize(
        authorized_voter_pubkey,
        VoteAuthorize::Voter,
    ))
    .unwrap();
    process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
}

#[test]
fn test_vote_authorize_checked() {
    let vote_pubkey = Pubkey::new_unique();
    let authorized_pubkey = Pubkey::new_unique();
    let new_authorized_pubkey = Pubkey::new_unique();

    let features = VoteProgramFeatures::default();
    // Test with vanilla authorize accounts
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let mut instruction = authorize_checked(
        &vote_pubkey,
        &authorized_pubkey,
        &new_authorized_pubkey,
        VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
            bls_pubkey,
            bls_proof_of_possession,
        }),
    );
    instruction.accounts = instruction.accounts[0..2].to_vec();
    process_instruction_as_one_arg(
        features,
        &instruction,
        Err(InstructionError::MissingAccount),
    );

    let mut instruction = authorize_checked(
        &vote_pubkey,
        &authorized_pubkey,
        &new_authorized_pubkey,
        VoteAuthorize::Withdrawer,
    );
    instruction.accounts = instruction.accounts[0..2].to_vec();
    process_instruction_as_one_arg(
        features,
        &instruction,
        Err(InstructionError::MissingAccount),
    );

    // Test with non-signing new_authorized_pubkey
    let mut instruction = authorize_checked(
        &vote_pubkey,
        &authorized_pubkey,
        &new_authorized_pubkey,
        VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
            bls_pubkey,
            bls_proof_of_possession,
        }),
    );
    instruction.accounts[3] = AccountMeta::new_readonly(new_authorized_pubkey, false);
    process_instruction_as_one_arg(
        features,
        &instruction,
        Err(InstructionError::MissingRequiredSignature),
    );

    let mut instruction = authorize_checked(
        &vote_pubkey,
        &authorized_pubkey,
        &new_authorized_pubkey,
        VoteAuthorize::Withdrawer,
    );
    instruction.accounts[3] = AccountMeta::new_readonly(new_authorized_pubkey, false);
    process_instruction_as_one_arg(
        features,
        &instruction,
        Err(InstructionError::MissingRequiredSignature),
    );

    // Test with new_authorized_pubkey signer
    let default_authorized_pubkey = Pubkey::default();
    let vote_state = VoteStateV4::new_with_defaults(
        &vote_pubkey,
        &VoteInit {
            node_pubkey: Pubkey::new_unique(),
            authorized_voter: default_authorized_pubkey,
            authorized_withdrawer: default_authorized_pubkey,
            commission: 0,
        },
        &Clock::default(),
    );
    let vote_account = AccountSharedData::new_data_with_space(
        100,
        &VoteStateVersions::V4(Box::new(vote_state)),
        VoteStateV4::size_of(),
        &id(),
    )
    .unwrap();
    let clock_address = sysvar::clock::id();
    let clock_account = create_sysvar_account(&Clock::default());
    let authorized_account = create_default_account();
    let new_authorized_account = create_default_account();
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (clock_address, clock_account),
        (default_authorized_pubkey, authorized_account),
        (new_authorized_pubkey, new_authorized_account),
    ];
    let instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: clock_address,
            is_signer: false,
            is_writable: false,
        },
        AccountMeta {
            pubkey: default_authorized_pubkey,
            is_signer: true,
            is_writable: false,
        },
        AccountMeta {
            pubkey: new_authorized_pubkey,
            is_signer: true,
            is_writable: false,
        },
    ];
    let (authorize_type, expected_cus) = (
        VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
            bls_pubkey,
            bls_proof_of_possession,
        }),
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS,
    );
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeChecked(authorize_type)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
        expected_cus,
    );
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeChecked(
            VoteAuthorize::Withdrawer,
        ))
        .unwrap(),
        transaction_accounts,
        instruction_accounts,
        Ok(()),
    );
}
