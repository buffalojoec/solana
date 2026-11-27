use {
    common::{
        VoteProgramFeatures, create_test_account_v3, create_test_account_with_authorized,
        deserialize_vote_state_for_test, process_instruction, process_instruction_with_cu_check,
    },
    solana_account::{AccountSharedData, ReadableAccount},
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_system_program::system_processor::DEFAULT_COMPUTE_UNITS as SYSTEM_PROGRAM_COMPUTE_UNITS,
    solana_vote_program::{
        id,
        vote_instruction::VoteInstruction,
        vote_processor::DEFAULT_COMPUTE_UNITS,
        vote_state::{VoteStateV4, handler::VoteStateHandler},
    },
};

mod common;

#[test]
fn test_deposit_delegator_rewards_fails() {
    let (vote_pubkey, _authorized_voter, _authorized_withdrawer, vote_account_v4) =
        create_test_account_with_authorized();

    // Create source account with enough lamports to transfer.
    let source_pubkey = Pubkey::new_unique();
    let source_lamports = 1_000_000;
    let source_account =
        AccountSharedData::empty(source_lamports, &solana_sdk_ids::system_program::id());

    let deposit_amount = 100_000;

    let instruction_data = wincode::serialize(&VoteInstruction::DepositDelegatorRewards {
        deposit: deposit_amount,
    })
    .unwrap();

    let instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: source_pubkey,
            is_signer: true,
            is_writable: true,
        },
        AccountMeta {
            pubkey: solana_sdk_ids::system_program::id(),
            is_signer: false,
            is_writable: false,
        },
    ];

    let transaction_accounts = vec![
        (vote_pubkey, vote_account_v4.clone()),
        (source_pubkey, source_account.clone()),
        (
            solana_sdk_ids::system_program::id(),
            AccountSharedData::empty(0, &solana_sdk_ids::native_loader::id()),
        ),
    ];

    process_instruction(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InvalidInstructionData),
    );
}

// Test DepositDelegatorRewards instruction (SIMD-0123).
#[ignore]
#[test]
fn test_deposit_delegator_rewards() {
    const DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS: u64 =
        DEFAULT_COMPUTE_UNITS + SYSTEM_PROGRAM_COMPUTE_UNITS;

    let (vote_pubkey, _authorized_voter, _authorized_withdrawer, vote_account_v4) =
        create_test_account_with_authorized();
    let (vote_pubkey_v3, vote_account_v3) = create_test_account_v3();

    // Create source account with enough lamports to transfer.
    let source_pubkey = Pubkey::new_unique();
    let source_lamports = 1_000_000;
    let source_account =
        AccountSharedData::empty(source_lamports, &solana_sdk_ids::system_program::id());

    let deposit_amount = 100_000;

    let instruction_data = wincode::serialize(&VoteInstruction::DepositDelegatorRewards {
        deposit: deposit_amount,
    })
    .unwrap();

    let instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: source_pubkey,
            is_signer: true,
            is_writable: true,
        },
        AccountMeta {
            pubkey: solana_sdk_ids::system_program::id(),
            is_signer: false,
            is_writable: false,
        },
    ];

    let transaction_accounts = vec![
        (vote_pubkey, vote_account_v4.clone()),
        (source_pubkey, source_account.clone()),
        (
            solana_sdk_ids::system_program::id(),
            AccountSharedData::empty(0, &solana_sdk_ids::native_loader::id()),
        ),
    ];

    // Fail - SIMD-0123: block_revenue_sharing disabled.
    process_instruction(
        VoteProgramFeatures {
            block_revenue_sharing: false,
            ..Default::default()
        },
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InvalidInstructionData),
    );

    // Fail - Not enough accounts (less than 2).
    let single_account_instruction_accounts = vec![AccountMeta {
        pubkey: vote_pubkey,
        is_signer: false,
        is_writable: true,
    }];
    process_instruction(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        transaction_accounts.clone(),
        single_account_instruction_accounts,
        Err(InstructionError::MissingAccount),
    );

    // Fail - Source account is not a signer.
    let non_signer_instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: source_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: solana_sdk_ids::system_program::id(),
            is_signer: false,
            is_writable: false,
        },
    ];
    process_instruction(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        transaction_accounts.clone(),
        non_signer_instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );

    // Fail - Vote account fails to deserialize (zeroed/uninitialized data).
    let invalid_vote_account =
        AccountSharedData::new_with_data(1_000_000, vec![0; VoteStateV4::size_of()], &id());
    process_instruction(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        vec![
            (vote_pubkey, invalid_vote_account),
            (source_pubkey, source_account.clone()),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::InvalidAccountData),
    );

    // Fail - Vote account is initialized but V3 (not V4).
    let instruction_accounts_v3 = vec![
        AccountMeta {
            pubkey: vote_pubkey_v3,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: source_pubkey,
            is_signer: true,
            is_writable: true,
        },
    ];
    process_instruction(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        vec![
            (vote_pubkey_v3, vote_account_v3),
            (source_pubkey, source_account.clone()),
        ],
        instruction_accounts_v3,
        Err(InstructionError::InvalidAccountData),
    );

    // Fail - non-system-owned source account.
    let non_system_source_account = AccountSharedData::empty(1_000_000, &Pubkey::new_unique());
    process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        vec![
            (vote_pubkey, vote_account_v4.clone()),
            (source_pubkey, non_system_source_account),
            (
                solana_sdk_ids::system_program::id(),
                AccountSharedData::empty(0, &solana_sdk_ids::native_loader::id()),
            ),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::ExternalAccountLamportSpend),
        DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS,
    );

    // Fail - source account == destination account.
    process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        vec![
            (vote_pubkey, vote_account_v4.clone()),
            (
                solana_sdk_ids::system_program::id(),
                AccountSharedData::empty(0, &solana_sdk_ids::native_loader::id()),
            ),
        ],
        vec![
            AccountMeta {
                pubkey: vote_pubkey,
                is_signer: false,
                is_writable: true,
            },
            AccountMeta {
                pubkey: vote_pubkey, // Duplicated
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: solana_sdk_ids::system_program::id(),
                is_signer: false,
                is_writable: false,
            },
        ],
        Err(InstructionError::InvalidArgument),
        DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS,
    );

    // Fail - source account has fewer lamports than the deposit.
    let underfunded_source_account =
        AccountSharedData::empty(deposit_amount - 1, &solana_sdk_ids::system_program::id());
    process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        vec![
            (vote_pubkey, vote_account_v4.clone()),
            (source_pubkey, underfunded_source_account),
            (
                solana_sdk_ids::system_program::id(),
                AccountSharedData::empty(0, &solana_sdk_ids::native_loader::id()),
            ),
        ],
        instruction_accounts.clone(),
        // SystemError::ResultWithNegativeLamports.
        Err(InstructionError::Custom(1)),
        DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS,
    );

    // Fail - deposit overflow.
    let deposit_amount = 100_000;
    let mut vote_account_near_max = vote_account_v4.clone();
    {
        let mut vote_state =
            VoteStateV4::deserialize(vote_account_near_max.data(), &vote_pubkey).unwrap();
        vote_state.pending_delegator_rewards = u64::MAX - deposit_amount + 1;
        vote_account_near_max.set_data_from_slice(&VoteStateHandler::new_v4(vote_state).serialize());
    }

    let instruction_data = wincode::serialize(&VoteInstruction::DepositDelegatorRewards {
        deposit: deposit_amount,
    })
    .unwrap();

    process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        vec![
            (vote_pubkey, vote_account_near_max),
            (source_pubkey, source_account.clone()),
            (
                solana_sdk_ids::system_program::id(),
                AccountSharedData::empty(0, &solana_sdk_ids::native_loader::id()),
            ),
        ],
        instruction_accounts.clone(),
        Err(InstructionError::ArithmeticOverflow),
        DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS,
    );

    // Success
    let resulting_accounts = process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
        DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS,
    );

    // Vote account should have been credited `deposit_amount`.
    // Source account should have been debited `deposit_amount`.
    // Vote state's `pending_delegator_rewards` should be updated.
    let vote_account_starting_lamports = vote_account_v4.lamports();
    let source_account_starting_lamports = source_lamports;
    let resulting_vote_account = &resulting_accounts[0];
    let resulting_source_account = &resulting_accounts[1];
    let vote_state = deserialize_vote_state_for_test(resulting_vote_account.data(), &vote_pubkey);
    assert_eq!(
        resulting_vote_account.lamports(),
        vote_account_starting_lamports + deposit_amount,
    );
    assert_eq!(
        resulting_source_account.lamports(),
        source_account_starting_lamports - deposit_amount,
    );
    assert_eq!(
        vote_state.as_ref_v4().pending_delegator_rewards,
        deposit_amount,
    );

    // Run it again with a new deposit amount.
    let first_deposit_amount = deposit_amount;
    let second_deposit_amount = 250_000;
    let vote_account_starting_lamports = resulting_vote_account.lamports();
    let source_account_starting_lamports = resulting_source_account.lamports();

    let instruction_data = wincode::serialize(&VoteInstruction::DepositDelegatorRewards {
        deposit: second_deposit_amount,
    })
    .unwrap();

    let resulting_accounts = process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        vec![
            (vote_pubkey, resulting_vote_account.clone()),
            (source_pubkey, resulting_source_account.clone()),
            (
                solana_sdk_ids::system_program::id(),
                AccountSharedData::empty(0, &solana_sdk_ids::native_loader::id()),
            ),
        ],
        instruction_accounts.clone(),
        Ok(()),
        DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS,
    );

    let resulting_vote_account = &resulting_accounts[0];
    let resulting_source_account = &resulting_accounts[1];
    let vote_state = deserialize_vote_state_for_test(resulting_vote_account.data(), &vote_pubkey);
    assert_eq!(
        resulting_vote_account.lamports(),
        vote_account_starting_lamports + second_deposit_amount,
    );
    assert_eq!(
        resulting_source_account.lamports(),
        source_account_starting_lamports - second_deposit_amount,
    );
    assert_eq!(
        vote_state.as_ref_v4().pending_delegator_rewards,
        first_deposit_amount + second_deposit_amount,
    );

    // Success - zero-lamport deposit.
    let vote_account_starting_lamports = vote_account_v4.lamports();
    let source_account_starting_lamports = source_lamports;
    let instruction_data =
        wincode::serialize(&VoteInstruction::DepositDelegatorRewards { deposit: 0 }).unwrap();

    let resulting_accounts = process_instruction_with_cu_check(
        VoteProgramFeatures::all_enabled(),
        &instruction_data,
        transaction_accounts,
        instruction_accounts.clone(),
        Ok(()),
        DEPOSIT_DELEGATOR_REWARDS_COMPUTE_UNITS,
    );

    let resulting_vote_account = &resulting_accounts[0];
    let resulting_source_account = &resulting_accounts[1];
    let vote_state = deserialize_vote_state_for_test(resulting_vote_account.data(), &vote_pubkey);
    assert_eq!(
        resulting_vote_account.lamports(),
        vote_account_starting_lamports, // No-op
    );
    assert_eq!(
        resulting_source_account.lamports(),
        source_account_starting_lamports, // No-op
    );
    assert_eq!(
        vote_state.as_ref_v4().pending_delegator_rewards,
        0, // No-op
    );
}
