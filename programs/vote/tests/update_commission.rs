use {
    common::{
        VoteProgramFeatures, create_sysvar_account, create_test_account_with_authorized,
        create_test_account_with_provided_authorized, deserialize_vote_state_for_test,
        process_instruction,
    },
    solana_account::{AccountSharedData, ReadableAccount},
    solana_clock::Clock,
    solana_epoch_schedule::EpochSchedule,
    solana_instruction::AccountMeta,
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_sdk_ids::sysvar,
    solana_vote_interface::instruction::CommissionKind,
    solana_vote_program::vote_instruction::VoteInstruction,
};

mod common;

#[test]
fn test_vote_update_commission() {
    let (vote_pubkey, _authorized_voter, authorized_withdrawer, vote_account) =
        create_test_account_with_authorized();
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommission(42)).unwrap();
    let transaction_accounts = vec![
        (vote_pubkey, vote_account),
        (authorized_withdrawer, AccountSharedData::default()),
        // Add the sysvar accounts so they're in the cache for mock processing
        (
            sysvar::clock::id(),
            create_sysvar_account(&Clock::default()),
        ),
        (
            sysvar::epoch_schedule::id(),
            create_sysvar_account(&EpochSchedule::without_warmup()),
        ),
    ];
    let mut instruction_accounts = vec![
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

    let features = VoteProgramFeatures {
        ..Default::default()
    };

    // should pass
    let accounts = process_instruction(
        features,
        &wincode::serialize(&VoteInstruction::UpdateCommission(200)).unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(vote_state.commission(), 200);

    // should pass
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(vote_state.commission(), 42);

    // should fail, authorized_withdrawer didn't sign the transaction
    instruction_accounts[1].is_signer = false;
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts,
        instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );
    let vote_state = deserialize_vote_state_for_test(accounts[0].data(), &vote_pubkey);
    assert_eq!(vote_state.commission(), 0);
}

#[test]
fn test_vote_update_commission_bps() {
    // Test UpdateCommissionBps instruction (SIMD-0291).
    let (vote_pubkey, _authorized_voter, authorized_withdrawer, vote_account) =
        create_test_account_with_authorized();

    let transaction_accounts = vec![
        (vote_pubkey, vote_account.clone()),
        (authorized_withdrawer, AccountSharedData::default()),
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

    let features = VoteProgramFeatures::all_enabled();

    let get_commission_bps = |vote_account: &AccountSharedData, kind: &CommissionKind| {
        let vote_state = deserialize_vote_state_for_test(vote_account.data(), &vote_pubkey);
        match kind {
            CommissionKind::InflationRewards => {
                vote_state.as_ref_v4().inflation_rewards_commission_bps
            }
            CommissionKind::BlockRevenue => vote_state.as_ref_v4().block_revenue_commission_bps,
        }
    };

    let original_commission_bps =
        get_commission_bps(&vote_account, &CommissionKind::InflationRewards);

    let commission_bps = 200; // 2%

    for kind in [
        CommissionKind::InflationRewards,
        CommissionKind::BlockRevenue,
    ] {
        let other_kind = match kind {
            CommissionKind::InflationRewards => CommissionKind::BlockRevenue,
            CommissionKind::BlockRevenue => CommissionKind::InflationRewards,
        };

        // Get the original commission for the other kind.
        let original_other_commission_bps = get_commission_bps(&vote_account, &other_kind);

        let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionBps {
            commission_bps,
            kind: kind.clone(),
        })
        .unwrap();

        // Should pass.
        let accounts = process_instruction(
            features,
            &instruction_data,
            transaction_accounts.clone(),
            instruction_accounts.clone(),
            Ok(()),
        );
        assert_eq!(get_commission_bps(&accounts[0], &kind), commission_bps);

        // Verify the other commission kind was not affected.
        assert_eq!(
            get_commission_bps(&accounts[0], &other_kind),
            original_other_commission_bps,
        );

        // Same value - should pass.
        let accounts = process_instruction(
            features,
            &instruction_data,
            vec![
                (vote_pubkey, accounts[0].clone()),
                (authorized_withdrawer, accounts[1].clone()),
            ],
            instruction_accounts.clone(),
            Ok(()),
        );
        assert_eq!(get_commission_bps(&accounts[0], &kind), commission_bps);

        // Verify the other commission kind is still unchanged.
        assert_eq!(
            get_commission_bps(&accounts[0], &other_kind),
            original_other_commission_bps,
        );
    }

    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionBps {
        commission_bps,
        kind: CommissionKind::InflationRewards,
    })
    .unwrap();

    // Should fail - `CommissionKind::BlockRevenue` disallowed (SIMD-0123 disabled).
    let accounts = process_instruction(
        VoteProgramFeatures {
            block_revenue_sharing: false,
            ..features
        },
        &wincode::serialize(&VoteInstruction::UpdateCommissionBps {
            commission_bps,
            kind: CommissionKind::BlockRevenue,
        })
        .unwrap(),
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Err(InstructionError::InvalidInstructionData),
    );
    let stored_commission_bps = get_commission_bps(&accounts[0], &CommissionKind::BlockRevenue);
    assert_eq!(stored_commission_bps, 0); // BlockRevenue starts at 0
    assert_ne!(stored_commission_bps, commission_bps); // New value not set

    // Should fail - authorized withdrawer didn't sign the transaction.
    let mut unsigned_instruction_accounts = instruction_accounts;
    unsigned_instruction_accounts[1].is_signer = false;
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        unsigned_instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );
    let stored_commission_bps =
        get_commission_bps(&accounts[0], &CommissionKind::InflationRewards);
    assert_eq!(stored_commission_bps, original_commission_bps); // Matches original
    assert_ne!(stored_commission_bps, commission_bps); // New value not set

    // Should fail - wrong signature for authorized withdrawer.
    let wrong_signer = Pubkey::new_unique();
    let mut wrong_signer_transaction_accounts = transaction_accounts;
    wrong_signer_transaction_accounts.push((wrong_signer, AccountSharedData::default()));
    let wrong_signer_instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: wrong_signer,
            is_signer: true,
            is_writable: false,
        },
    ];
    let accounts = process_instruction(
        features,
        &instruction_data,
        wrong_signer_transaction_accounts,
        wrong_signer_instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );
    let stored_commission_bps =
        get_commission_bps(&accounts[0], &CommissionKind::InflationRewards);
    assert_eq!(stored_commission_bps, original_commission_bps); // Matches original
    assert_ne!(stored_commission_bps, commission_bps); // New value not set
}

#[test]
fn test_vote_update_commission_collector() {
    let (vote_pubkey, _authorized_voter, authorized_withdrawer, vote_account) =
        create_test_account_with_authorized();

    // Create a valid collector account: system-owned and rent-exempt.
    let new_collector_pubkey = Pubkey::new_unique();
    let rent = Rent::default();
    let rent_sysvar_account = create_sysvar_account(&rent);
    let collector_lamports = rent.minimum_balance(0);
    let new_collector_account =
        AccountSharedData::empty(collector_lamports, &solana_sdk_ids::system_program::id());

    let transaction_accounts = vec![
        (vote_pubkey, vote_account.clone()),
        (new_collector_pubkey, new_collector_account),
        (authorized_withdrawer, AccountSharedData::default()),
        (sysvar::rent::id(), rent_sysvar_account),
    ];

    let instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: new_collector_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: authorized_withdrawer,
            is_signer: true,
            is_writable: false,
        },
    ];

    let features = VoteProgramFeatures::default();

    let get_commission_collector = |vote_account: &AccountSharedData, kind: CommissionKind| {
        let vote_state = deserialize_vote_state_for_test(vote_account.data(), &vote_pubkey)
            .as_ref_v4()
            .clone();
        match kind {
            CommissionKind::InflationRewards => vote_state.inflation_rewards_collector,
            CommissionKind::BlockRevenue => vote_state.block_revenue_collector,
        }
    };

    let original_inflation_collector =
        get_commission_collector(&vote_account, CommissionKind::InflationRewards);
    let original_block_revenue_collector =
        get_commission_collector(&vote_account, CommissionKind::BlockRevenue);

    // Should pass - InflationRewards kind.
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionCollector(
        CommissionKind::InflationRewards,
    ))
    .unwrap();
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        new_collector_pubkey,
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );

    // Should pass - BlockRevenue kind.
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionCollector(
        CommissionKind::BlockRevenue,
    ))
    .unwrap();
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        instruction_accounts.clone(),
        Ok(()),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector, // Unchanged
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        new_collector_pubkey,
    );

    // Should pass - setting collector to vote account (InflationRewards).
    let vote_as_collector_instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: vote_pubkey, // Collector is the vote account.
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: authorized_withdrawer,
            is_signer: true,
            is_writable: false,
        },
    ];
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionCollector(
        CommissionKind::InflationRewards,
    ))
    .unwrap();
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        vote_as_collector_instruction_accounts.clone(),
        Ok(()),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        vote_pubkey
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );

    // Should pass - setting collector to vote account (BlockRevenue).
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionCollector(
        CommissionKind::BlockRevenue,
    ))
    .unwrap();
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        vote_as_collector_instruction_accounts,
        Ok(()),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector, // Unchanged
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        vote_pubkey
    );

    // Should pass - all three accounts can alias.
    let aliased_vote_account =
        create_test_account_with_provided_authorized(&vote_pubkey, &vote_pubkey);
    let aliased_original_inflation_collector =
        get_commission_collector(&aliased_vote_account, CommissionKind::InflationRewards);
    let aliased_original_block_revenue_collector =
        get_commission_collector(&aliased_vote_account, CommissionKind::BlockRevenue);
    let aliased_transaction_accounts = vec![
        (vote_pubkey, aliased_vote_account),
        (sysvar::rent::id(), create_sysvar_account(&rent)),
    ];
    let aliased_instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey, // Vote account
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: vote_pubkey, // New collector
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: vote_pubkey, // Authorized withdrawer
            is_signer: true,     // (Signer)
            is_writable: false,
        },
    ];

    // InflationRewards (triple alias)
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionCollector(
        CommissionKind::InflationRewards,
    ))
    .unwrap();
    let accounts = process_instruction(
        features,
        &instruction_data,
        aliased_transaction_accounts.clone(),
        aliased_instruction_accounts.clone(),
        Ok(()),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        vote_pubkey
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        aliased_original_block_revenue_collector, // Unchanged
    );

    // BlockRevenue (triple alias)
    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionCollector(
        CommissionKind::BlockRevenue,
    ))
    .unwrap();
    let accounts = process_instruction(
        features,
        &instruction_data,
        aliased_transaction_accounts,
        aliased_instruction_accounts,
        Ok(()),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        aliased_original_inflation_collector, // Unchanged
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        vote_pubkey
    );

    let instruction_data = wincode::serialize(&VoteInstruction::UpdateCommissionCollector(
        CommissionKind::InflationRewards,
    ))
    .unwrap();

    // Should fail - fewer than 3 account inputs (SIMD-0232).
    let too_few_instruction_accounts = vec![AccountMeta {
        pubkey: vote_pubkey,
        is_signer: false,
        is_writable: true,
    }];
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        too_few_instruction_accounts,
        Err(InstructionError::MissingAccount),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector, // Unchanged
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );

    // Should fail - authorized withdrawer didn't sign.
    let mut unsigned_instruction_accounts = instruction_accounts.clone();
    unsigned_instruction_accounts[2].is_signer = false;
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts.clone(),
        unsigned_instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );

    // Should fail - wrong signer (not the authorized withdrawer).
    let wrong_signer = Pubkey::new_unique();
    let mut wrong_signer_transaction_accounts = transaction_accounts.clone();
    wrong_signer_transaction_accounts.push((wrong_signer, AccountSharedData::default()));
    let wrong_signer_instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: new_collector_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: wrong_signer,
            is_signer: true,
            is_writable: false,
        },
    ];
    let accounts = process_instruction(
        features,
        &instruction_data,
        wrong_signer_transaction_accounts,
        wrong_signer_instruction_accounts,
        Err(InstructionError::MissingRequiredSignature),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );

    // Should fail - new collector not system program owned.
    let non_system_owner = Pubkey::new_unique();
    let non_system_collector_pubkey = Pubkey::new_unique();
    let non_system_collector_account =
        AccountSharedData::empty(collector_lamports, &non_system_owner);
    let mut non_system_transaction_accounts = transaction_accounts.clone();
    non_system_transaction_accounts[1] =
        (non_system_collector_pubkey, non_system_collector_account);
    let non_system_instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: non_system_collector_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: authorized_withdrawer,
            is_signer: true,
            is_writable: false,
        },
    ];
    let accounts = process_instruction(
        features,
        &instruction_data,
        non_system_transaction_accounts,
        non_system_instruction_accounts,
        Err(InstructionError::InvalidAccountOwner),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );

    // Should fail - new collector not rent-exempt.
    let not_rent_exempt_collector_pubkey = Pubkey::new_unique();
    let not_rent_exempt_collector_account =
        AccountSharedData::empty(0, &solana_sdk_ids::system_program::id()); // 0 lamports
    let mut not_rent_exempt_transaction_accounts = transaction_accounts.clone();
    not_rent_exempt_transaction_accounts[1] = (
        not_rent_exempt_collector_pubkey,
        not_rent_exempt_collector_account,
    );
    let not_rent_exempt_instruction_accounts = vec![
        AccountMeta {
            pubkey: vote_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: not_rent_exempt_collector_pubkey,
            is_signer: false,
            is_writable: true,
        },
        AccountMeta {
            pubkey: authorized_withdrawer,
            is_signer: true,
            is_writable: false,
        },
    ];
    let accounts = process_instruction(
        features,
        &instruction_data,
        not_rent_exempt_transaction_accounts,
        not_rent_exempt_instruction_accounts,
        Err(InstructionError::InsufficientFunds),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );

    // Should fail - new collector not writable (reserved account check).
    let mut not_writable_instruction_accounts = instruction_accounts;
    not_writable_instruction_accounts[1].is_writable = false;
    let accounts = process_instruction(
        features,
        &instruction_data,
        transaction_accounts,
        not_writable_instruction_accounts,
        Err(InstructionError::InvalidArgument),
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::InflationRewards),
        original_inflation_collector
    );
    assert_eq!(
        get_commission_collector(&accounts[0], CommissionKind::BlockRevenue),
        original_block_revenue_collector, // Unchanged
    );
}
