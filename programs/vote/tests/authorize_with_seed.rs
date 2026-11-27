use {
    common::{
        VoteAccountTestFixtureWithAuthorities, VoteProgramFeatures, create_sysvar_account,
        create_test_account_with_authorized_from_seed, process_instruction,
        process_instruction_with_cu_check,
    },
    solana_account::AccountSharedData,
    solana_clock::Clock,
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_sdk_ids::sysvar,
    solana_vote_interface::state::VoterWithBLSArgs,
    solana_vote_program::{
        vote_instruction::VoteInstruction,
        vote_processor::{BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS, DEFAULT_COMPUTE_UNITS},
        vote_state::{
            VoteAuthorize, VoteAuthorizeCheckedWithSeedArgs, VoteAuthorizeWithSeedArgs,
            create_bls_pubkey_and_proof_of_possession,
        },
    },
};

mod common;

#[test]
fn test_voter_base_key_can_authorize_new_voter() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        voter_base_key,
        voter_owner,
        voter_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_voter_pubkey = Pubkey::new_unique();
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let authorize_type = VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
        bls_pubkey,
        bls_proof_of_possession,
    });
    perform_authorize_with_seed_test(
        authorize_type,
        vote_pubkey,
        vote_account,
        voter_base_key,
        voter_seed,
        voter_owner,
        new_voter_pubkey,
    );
}

#[test]
fn test_withdrawer_base_key_can_authorize_new_voter() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        withdrawer_base_key,
        withdrawer_owner,
        withdrawer_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_voter_pubkey = Pubkey::new_unique();
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let authorize_type = VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
        bls_pubkey,
        bls_proof_of_possession,
    });
    perform_authorize_with_seed_test(
        authorize_type,
        vote_pubkey,
        vote_account,
        withdrawer_base_key,
        withdrawer_seed,
        withdrawer_owner,
        new_voter_pubkey,
    );
}

#[test]
fn test_voter_base_key_can_not_authorize_new_withdrawer() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        voter_base_key,
        voter_owner,
        voter_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_withdrawer_pubkey = Pubkey::new_unique();
    let clock = Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (sysvar::clock::id(), clock_account),
        (voter_base_key, AccountSharedData::default()),
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
            pubkey: voter_base_key,
            is_signer: true,
            is_writable: false,
        },
    ];
    // Despite having Voter authority, you may not change the Withdrawer authority.
    process_instruction(
        VoteProgramFeatures {
            ..Default::default()
        },
        &wincode::serialize(&VoteInstruction::AuthorizeWithSeed(
            VoteAuthorizeWithSeedArgs {
                authorization_type: VoteAuthorize::Withdrawer,
                current_authority_derived_key_owner: voter_owner,
                current_authority_derived_key_seed: voter_seed,
                new_authority: new_withdrawer_pubkey,
            },
        ))
        .unwrap(),
        transaction_accounts,
        instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );
}

#[test]
fn test_withdrawer_base_key_can_authorize_new_withdrawer() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        withdrawer_base_key,
        withdrawer_owner,
        withdrawer_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_withdrawer_pubkey = Pubkey::new_unique();
    perform_authorize_with_seed_test(
        VoteAuthorize::Withdrawer,
        vote_pubkey,
        vote_account,
        withdrawer_base_key,
        withdrawer_seed,
        withdrawer_owner,
        new_withdrawer_pubkey,
    );
}

#[test]
fn test_voter_base_key_can_authorize_new_voter_checked() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        voter_base_key,
        voter_owner,
        voter_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_voter_pubkey = Pubkey::new_unique();
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let authorize_type = VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
        bls_pubkey,
        bls_proof_of_possession,
    });
    perform_authorize_checked_with_seed_test(
        authorize_type,
        vote_pubkey,
        vote_account,
        voter_base_key,
        voter_seed,
        voter_owner,
        new_voter_pubkey,
    );
}

#[test]
fn test_withdrawer_base_key_can_authorize_new_voter_checked() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        withdrawer_base_key,
        withdrawer_owner,
        withdrawer_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_voter_pubkey = Pubkey::new_unique();
    let (bls_pubkey, bls_proof_of_possession) =
        create_bls_pubkey_and_proof_of_possession(&vote_pubkey);
    let authorize_type = VoteAuthorize::VoterWithBLS(VoterWithBLSArgs {
        bls_pubkey,
        bls_proof_of_possession,
    });
    perform_authorize_checked_with_seed_test(
        authorize_type,
        vote_pubkey,
        vote_account,
        withdrawer_base_key,
        withdrawer_seed,
        withdrawer_owner,
        new_voter_pubkey,
    );
}

#[test]
fn test_voter_base_key_can_not_authorize_new_withdrawer_checked() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        voter_base_key,
        voter_owner,
        voter_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_withdrawer_pubkey = Pubkey::new_unique();
    let clock = Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (sysvar::clock::id(), clock_account),
        (voter_base_key, AccountSharedData::default()),
        (new_withdrawer_pubkey, AccountSharedData::default()),
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
            pubkey: voter_base_key,
            is_signer: true,
            is_writable: false,
        },
        AccountMeta {
            pubkey: new_withdrawer_pubkey,
            is_signer: true,
            is_writable: false,
        },
    ];
    // Despite having Voter authority, you may not change the Withdrawer authority.
    process_instruction(
        VoteProgramFeatures {
            ..Default::default()
        },
        &wincode::serialize(&VoteInstruction::AuthorizeCheckedWithSeed(
            VoteAuthorizeCheckedWithSeedArgs {
                authorization_type: VoteAuthorize::Withdrawer,
                current_authority_derived_key_owner: voter_owner,
                current_authority_derived_key_seed: voter_seed,
            },
        ))
        .unwrap(),
        transaction_accounts,
        instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );
}

#[test]
fn test_withdrawer_base_key_can_authorize_new_withdrawer_checked() {
    let VoteAccountTestFixtureWithAuthorities {
        vote_pubkey,
        withdrawer_base_key,
        withdrawer_owner,
        withdrawer_seed,
        vote_account,
        ..
    } = create_test_account_with_authorized_from_seed();
    let new_withdrawer_pubkey = Pubkey::new_unique();
    perform_authorize_checked_with_seed_test(
        VoteAuthorize::Withdrawer,
        vote_pubkey,
        vote_account,
        withdrawer_base_key,
        withdrawer_seed,
        withdrawer_owner,
        new_withdrawer_pubkey,
    );
}

fn perform_authorize_with_seed_test(
    authorization_type: VoteAuthorize,
    vote_pubkey: Pubkey,
    vote_account: AccountSharedData,
    current_authority_base_key: Pubkey,
    current_authority_seed: String,
    current_authority_owner: Pubkey,
    new_authority_pubkey: Pubkey,
) {
    let clock = Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (sysvar::clock::id(), clock_account),
        (current_authority_base_key, AccountSharedData::default()),
    ];
    let mut instruction_accounts = vec![
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

    let features = VoteProgramFeatures::default();
    let expected_cus = if matches!(authorization_type, VoteAuthorize::VoterWithBLS(_)) {
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS
    } else {
        DEFAULT_COMPUTE_UNITS
    };

    // Can't change authority unless base key signs.
    instruction_accounts[2].is_signer = false;
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeWithSeed(
            VoteAuthorizeWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: current_authority_seed.clone(),
                new_authority: new_authority_pubkey,
            },
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
        expected_cus,
    );
    instruction_accounts[2].is_signer = true;

    // Can't change authority if seed doesn't match.
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeWithSeed(
            VoteAuthorizeWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: String::from("WRONG_SEED"),
                new_authority: new_authority_pubkey,
            },
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
        expected_cus,
    );

    // Can't change authority if owner doesn't match.
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeWithSeed(
            VoteAuthorizeWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: Pubkey::new_unique(), // Wrong owner.
                current_authority_derived_key_seed: current_authority_seed.clone(),
                new_authority: new_authority_pubkey,
            },
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
        expected_cus,
    );

    // Can change authority when base key signs for related derived key.
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeWithSeed(
            VoteAuthorizeWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: current_authority_seed,
                new_authority: new_authority_pubkey,
            },
        ))
        .unwrap(),
        transaction_accounts,
        instruction_accounts,
        Ok(()),
        expected_cus,
    );
}

fn perform_authorize_checked_with_seed_test(
    authorization_type: VoteAuthorize,
    vote_pubkey: Pubkey,
    vote_account: AccountSharedData,
    current_authority_base_key: Pubkey,
    current_authority_seed: String,
    current_authority_owner: Pubkey,
    new_authority_pubkey: Pubkey,
) {
    let clock = Clock {
        epoch: 1,
        leader_schedule_epoch: 2,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (sysvar::clock::id(), clock_account),
        (current_authority_base_key, AccountSharedData::default()),
        (new_authority_pubkey, AccountSharedData::default()),
    ];
    let mut instruction_accounts = vec![
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

    let features = VoteProgramFeatures::default();
    let expected_cus = if matches!(authorization_type, VoteAuthorize::VoterWithBLS(_)) {
        DEFAULT_COMPUTE_UNITS + BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS
    } else {
        DEFAULT_COMPUTE_UNITS
    };

    // Can't change authority unless base key signs.
    instruction_accounts[2].is_signer = false;
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeCheckedWithSeed(
            VoteAuthorizeCheckedWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: current_authority_seed.clone(),
            },
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
        expected_cus,
    );
    instruction_accounts[2].is_signer = true;

    // Can't change authority unless new authority signs.
    // This check happens before authorize(), so no BLS CUs are consumed.
    instruction_accounts[3].is_signer = false;
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeCheckedWithSeed(
            VoteAuthorizeCheckedWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: current_authority_seed.clone(),
            },
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
    instruction_accounts[3].is_signer = true;

    // Can't change authority if seed doesn't match.
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeCheckedWithSeed(
            VoteAuthorizeCheckedWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: String::from("WRONG_SEED"),
            },
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
        expected_cus,
    );

    // Can't change authority if owner doesn't match.
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeCheckedWithSeed(
            VoteAuthorizeCheckedWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: Pubkey::new_unique(), // Wrong owner.
                current_authority_derived_key_seed: current_authority_seed.clone(),
            },
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
        expected_cus,
    );

    // Can change authority when base key signs for related derived key and new authority signs.
    process_instruction_with_cu_check(
        features,
        &wincode::serialize(&VoteInstruction::AuthorizeCheckedWithSeed(
            VoteAuthorizeCheckedWithSeedArgs {
                authorization_type,
                current_authority_derived_key_owner: current_authority_owner,
                current_authority_derived_key_seed: current_authority_seed,
            },
        ))
        .unwrap(),
        transaction_accounts,
        instruction_accounts,
        Ok(()),
        expected_cus,
    );
}
