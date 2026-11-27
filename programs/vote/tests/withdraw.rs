use {
    common::{
        VoteProgramFeatures, create_default_clock_account, create_default_rent_account,
        create_sysvar_account, create_test_account, create_test_account_with_authorized,
        create_test_account_with_epoch_credits, process_instruction,
    },
    solana_account::{
        AccountSharedData, ReadableAccount, WritableAccount, state_traits::StateMutWincode,
    },
    solana_clock::Clock,
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_rent::Rent,
    solana_sdk_ids::sysvar,
    solana_vote_program::{
        vote_error::VoteError,
        vote_instruction::VoteInstruction,
        vote_state::{VoteAuthorize, VoteStateV4, VoteStateVersions, handler::VoteStateHandler},
    },
};

mod common;

#[test]
fn test_vote_withdraw() {
    let (vote_pubkey, vote_account) = create_test_account();
    let lamports = vote_account.lamports();
    let authorized_withdrawer_pubkey = solana_pubkey::new_rand();
    let mut transaction_accounts = vec![
        (vote_pubkey, vote_account.clone()),
        (sysvar::clock::id(), create_default_clock_account()),
        (sysvar::rent::id(), create_default_rent_account()),
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

    // should pass, withdraw using authorized_withdrawer to authorized_withdrawer's account
    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Authorize(
            authorized_withdrawer_pubkey,
            VoteAuthorize::Withdrawer,
        ))
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    instruction_accounts[0].is_signer = false;
    instruction_accounts[1] = AccountMeta {
        pubkey: authorized_withdrawer_pubkey,
        is_signer: true,
        is_writable: true,
    };
    transaction_accounts[0] = (vote_pubkey, accounts[0].clone());
    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    assert_eq!(accounts[0].lamports(), 0);
    assert_eq!(accounts[3].lamports(), lamports);
    let post_state: VoteStateVersions = accounts[0].state().unwrap();
    // State has been deinitialized since balance is zero
    assert!(post_state.is_uninitialized());

    // should fail, unsigned
    transaction_accounts[0] = (vote_pubkey, vote_account);
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::MissingRequiredSignature),
    );
    instruction_accounts[0].is_signer = true;

    // should pass
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );

    // should fail, insufficient funds
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports + 1)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InsufficientFunds),
    );

    // should pass, partial withdraw
    let withdraw_lamports = 42;
    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(withdraw_lamports)).unwrap(),
        transaction_accounts,
        instruction_accounts,
        Ok(()),
    );
    assert_eq!(accounts[0].lamports(), lamports - withdraw_lamports);
    assert_eq!(accounts[3].lamports(), withdraw_lamports);
}

#[test]
fn test_vote_state_withdraw() {
    let authorized_withdrawer_pubkey = solana_pubkey::new_rand();
    let (vote_pubkey_1, vote_account_with_epoch_credits_1) =
        create_test_account_with_epoch_credits(&[2, 1]);
    let (vote_pubkey_2, vote_account_with_epoch_credits_2) =
        create_test_account_with_epoch_credits(&[2, 1, 3]);
    let clock = Clock {
        epoch: 3,
        ..Clock::default()
    };
    let clock_account = create_sysvar_account(&clock);
    let rent_sysvar = Rent::default();
    let minimum_balance = rent_sysvar
        .minimum_balance(vote_account_with_epoch_credits_1.data().len())
        .max(1);
    let lamports = vote_account_with_epoch_credits_1.lamports();
    let transaction_accounts = vec![
        (vote_pubkey_1, vote_account_with_epoch_credits_1),
        (vote_pubkey_2, vote_account_with_epoch_credits_2),
        (sysvar::clock::id(), clock_account),
        (sysvar::rent::id(), create_sysvar_account(&rent_sysvar)),
        (authorized_withdrawer_pubkey, AccountSharedData::default()),
    ];
    let mut instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey_1,
            is_signer: true,
            is_writable: true,
        },
        AccountMeta {
            pubkey: authorized_withdrawer_pubkey,
            is_signer: false,
            is_writable: true,
        },
    ];

    let features = VoteProgramFeatures {
        ..Default::default()
    };

    // non rent exempt withdraw, with 0 credit epoch
    instruction_accounts[0].pubkey = vote_pubkey_1;
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports - minimum_balance + 1)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InsufficientFunds),
    );

    // non rent exempt withdraw, without 0 credit epoch
    instruction_accounts[0].pubkey = vote_pubkey_2;
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports - minimum_balance + 1)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InsufficientFunds),
    );

    // full withdraw, with 0 credit epoch
    instruction_accounts[0].pubkey = vote_pubkey_1;
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );

    // full withdraw, without 0 credit epoch
    instruction_accounts[0].pubkey = vote_pubkey_2;
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(lamports)).unwrap(),
        transaction_accounts,
        instruction_accounts,
        Err(VoteError::ActiveVoteAccountClose.into()),
    );
}

#[test]
#[allow(clippy::arithmetic_side_effects)]
fn test_withdraw_pending_delegator_rewards() {
    let rent_sysvar = Rent::default();
    let rent_minimum_balance = rent_sysvar.minimum_balance(VoteStateV4::size_of());

    let pending_rewards = 500_000;
    let extra_for_withdraw = 100_000;
    let vote_account_lamports = rent_minimum_balance + pending_rewards + extra_for_withdraw;

    let (vote_pubkey, _authorized_voter, authorized_withdrawer, mut vote_account) =
        create_test_account_with_authorized();

    // Set some pending delegator rewards.
    {
        let mut vote_state = VoteStateV4::deserialize(vote_account.data(), &vote_pubkey).unwrap();
        vote_state.pending_delegator_rewards = pending_rewards;
        vote_account.set_data_from_slice(&VoteStateHandler::new_v4(vote_state).serialize());
        vote_account.set_lamports(vote_account_lamports);
    };

    let features = VoteProgramFeatures::all_enabled();

    let instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: authorized_withdrawer,
            is_signer: true,
            is_writable: true,
        },
    ];

    let rent_account = create_sysvar_account(&rent_sysvar);
    let transaction_accounts = vec![
        (vote_pubkey, vote_account.clone()),
        (authorized_withdrawer, AccountSharedData::default()),
        (sysvar::clock::id(), create_default_clock_account()),
        (sysvar::rent::id(), rent_account.clone()),
    ];

    // Should fail, can't close vote account when
    // pending_delegator_rewards > 0.
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(vote_account_lamports)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InsufficientFunds),
    );

    // Should fail, can't withdraw more than
    // (lamports - pending_delegator_rewards - rent_exempt).
    process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(vote_account_lamports + 1)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InsufficientFunds),
    );

    // Should pass, can withdraw up to the max withdrawable amount.
    for i in 1..10 {
        let withdraw_amount = 1 + i * extra_for_withdraw / 10;

        let accounts = process_instruction(
            features,
            &wincode::serialize(&VoteInstruction::Withdraw(withdraw_amount)).unwrap(),
            transaction_accounts.clone(),
            instruction_accounts.clone(),
            Ok(()),
        );

        assert_eq!(
            accounts[0].lamports(),
            vote_account_lamports - withdraw_amount
        );
        assert!(accounts[0].lamports() >= rent_minimum_balance + pending_rewards);
        assert_eq!(accounts[1].lamports(), withdraw_amount);
    }

    // Now clear pending delegator rewards.
    {
        let mut vote_state = VoteStateV4::deserialize(vote_account.data(), &vote_pubkey).unwrap();
        vote_state.pending_delegator_rewards = 0;
        vote_account.set_data_from_slice(&VoteStateHandler::new_v4(vote_state).serialize());
        vote_account.set_lamports(vote_account_lamports);
    };

    // Should pass, no pending delegator rewards, so we can close the whole
    // thing out.
    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::Withdraw(vote_account_lamports)).unwrap(),
        vec![
            (vote_pubkey, vote_account.clone()),
            (authorized_withdrawer, AccountSharedData::default()),
            (sysvar::clock::id(), create_default_clock_account()),
            (sysvar::rent::id(), rent_account),
        ],
        instruction_accounts.clone(),
        Ok(()),
    );

    assert_eq!(accounts[0].lamports(), 0);
    assert_eq!(accounts[0].data(), vec![0; VoteStateV4::size_of()]);
    assert_eq!(accounts[1].lamports(), vote_account_lamports);
}
