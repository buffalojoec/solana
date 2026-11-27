//! Vote program processor

use {
    crate::vote_state::{self, handler::VoteStateTargetVersion},
    log::*,
    solana_instruction_error::InstructionError,
    solana_packet::PACKET_DATA_WINCODE_CONFIG,
    solana_program_runtime::{
        declare_process_instruction, invoke_context::InvokeContext,
        sysvar_cache::get_sysvar_with_account_check,
    },
    solana_pubkey::Pubkey,
    solana_transaction_context::{
        instruction::InstructionContext, instruction_accounts::BorrowedInstructionAccount,
    },
    solana_vote_interface::{instruction::VoteInstruction, program::id, state::VoteAuthorize},
    std::collections::HashSet,
};

#[allow(clippy::too_many_arguments)]
fn process_authorize_with_seed_instruction<F>(
    invoke_context: &InvokeContext,
    instruction_context: &InstructionContext,
    vote_account: &mut BorrowedInstructionAccount,
    target_version: VoteStateTargetVersion,
    new_authority: &Pubkey,
    authorization_type: VoteAuthorize,
    current_authority_derived_key_owner: &Pubkey,
    current_authority_derived_key_seed: &str,
    consume_pop_compute_units: F,
) -> Result<(), InstructionError>
where
    F: FnOnce() -> Result<(), InstructionError>,
{
    let clock = get_sysvar_with_account_check::clock(invoke_context, instruction_context, 1)?;
    let mut expected_authority_keys: HashSet<Pubkey> = HashSet::default();
    if instruction_context.is_instruction_account_signer(2)? {
        let base_pubkey = instruction_context.get_key_of_instruction_account(2)?;
        // The conversion from `PubkeyError` to `InstructionError` through
        // num-traits is incorrect, but it's the existing behavior.
        expected_authority_keys.insert(
            Pubkey::create_with_seed(
                base_pubkey,
                current_authority_derived_key_seed,
                current_authority_derived_key_owner,
            )
            .map_err(|e| e as u64)?,
        );
    };
    vote_state::authorize(
        vote_account,
        target_version,
        new_authority,
        authorization_type,
        &expected_authority_keys,
        &clock,
        consume_pop_compute_units,
    )
}

fn is_init_account_v2_enabled(invoke_context: &InvokeContext) -> bool {
    let feature_set = invoke_context.get_feature_set();
    feature_set.bls_pubkey_management_in_vote_account
        && feature_set.block_revenue_sharing
        && feature_set.vote_account_initialize_v2
}

// Citing `runtime/src/block_cost_limit.rs`, vote has statically defined 2100
// units; can consume based on instructions in the future like `bpf_loader` does.
pub const DEFAULT_COMPUTE_UNITS: u64 = 2_100;

/// Cost in compute units for BLS proof-of-possession verification (SIMD-0387).
pub const BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS: u64 = 34_500;

declare_process_instruction!(Entrypoint, DEFAULT_COMPUTE_UNITS, |invoke_context| {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let data = instruction_context.get_instruction_data();

    trace!("process_instruction: {data:?}");

    let mut me = instruction_context.try_borrow_instruction_account(0)?;
    if *me.get_owner() != id() {
        return Err(InstructionError::InvalidAccountOwner);
    }

    // Determine the target vote state version to use for all operations.
    let target_version = VoteStateTargetVersion::V4;

    let signers = instruction_context.get_signers()?;
    let is_init_account_v2_enabled = is_init_account_v2_enabled(invoke_context);
    let consume_pop_compute_units = || {
        invoke_context
            .consume_checked(BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS)
            .map_err(|_| InstructionError::ComputationalBudgetExceeded)
    };
    let instruction: VoteInstruction =
        wincode::config::deserialize(data, PACKET_DATA_WINCODE_CONFIG)
            .map_err(|_| InstructionError::InvalidInstructionData)?;
    match instruction {
        VoteInstruction::InitializeAccount(vote_init) => {
            let rent =
                get_sysvar_with_account_check::rent(invoke_context, &instruction_context, 1)?;
            if !rent.is_exempt(me.get_lamports(), me.get_data().len()) {
                return Err(InstructionError::InsufficientFunds);
            }
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, &instruction_context, 2)?;
            vote_state::initialize_account(&mut me, target_version, &vote_init, &signers, &clock)
        }
        VoteInstruction::Authorize(voter_pubkey, vote_authorize) => {
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, &instruction_context, 1)?;
            vote_state::authorize(
                &mut me,
                target_version,
                &voter_pubkey,
                vote_authorize,
                &signers,
                &clock,
                consume_pop_compute_units,
            )
        }
        VoteInstruction::AuthorizeWithSeed(args) => {
            instruction_context.check_number_of_instruction_accounts(3)?;
            process_authorize_with_seed_instruction(
                invoke_context,
                &instruction_context,
                &mut me,
                target_version,
                &args.new_authority,
                args.authorization_type,
                &args.current_authority_derived_key_owner,
                args.current_authority_derived_key_seed.as_str(),
                consume_pop_compute_units,
            )
        }
        VoteInstruction::AuthorizeCheckedWithSeed(args) => {
            instruction_context.check_number_of_instruction_accounts(4)?;
            let new_authority = instruction_context.get_key_of_instruction_account(3)?;
            if !instruction_context.is_instruction_account_signer(3)? {
                return Err(InstructionError::MissingRequiredSignature);
            }
            process_authorize_with_seed_instruction(
                invoke_context,
                &instruction_context,
                &mut me,
                target_version,
                new_authority,
                args.authorization_type,
                &args.current_authority_derived_key_owner,
                args.current_authority_derived_key_seed.as_str(),
                consume_pop_compute_units,
            )
        }
        VoteInstruction::UpdateValidatorIdentity => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let node_pubkey = instruction_context.get_key_of_instruction_account(1)?;
            vote_state::update_validator_identity(&mut me, target_version, node_pubkey, &signers)
        }
        VoteInstruction::UpdateCommission(commission) => {
            vote_state::update_commission(&mut me, target_version, commission, &signers)
        }
        VoteInstruction::Vote(_)
        | VoteInstruction::VoteSwitch(_, _)
        | VoteInstruction::UpdateVoteState(_)
        | VoteInstruction::UpdateVoteStateSwitch(_, _)
        | VoteInstruction::CompactUpdateVoteState(_)
        | VoteInstruction::CompactUpdateVoteStateSwitch(_, _)
        | VoteInstruction::TowerSync(_)
        | VoteInstruction::TowerSyncSwitch(_, _) => Err(InstructionError::InvalidInstructionData),
        VoteInstruction::Withdraw(lamports) => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let rent_sysvar = invoke_context
                .environment_config
                .sysvar_cache()
                .get_rent()?;
            let clock_sysvar = invoke_context
                .environment_config
                .sysvar_cache()
                .get_clock()?;

            drop(me);
            vote_state::withdraw(
                &instruction_context,
                0,
                target_version,
                lamports,
                1,
                &signers,
                &rent_sysvar,
                &clock_sysvar,
            )
        }
        VoteInstruction::AuthorizeChecked(vote_authorize) => {
            instruction_context.check_number_of_instruction_accounts(4)?;
            let voter_pubkey = instruction_context.get_key_of_instruction_account(3)?;
            if !instruction_context.is_instruction_account_signer(3)? {
                return Err(InstructionError::MissingRequiredSignature);
            }
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, &instruction_context, 1)?;
            vote_state::authorize(
                &mut me,
                target_version,
                voter_pubkey,
                vote_authorize,
                &signers,
                &clock,
                consume_pop_compute_units,
            )
        }
        VoteInstruction::InitializeAccountV2(vote_init_v2) => {
            if !is_init_account_v2_enabled {
                return Err(InstructionError::InvalidInstructionData);
            }

            instruction_context.check_number_of_instruction_accounts(4)?;

            let sysvar_cache = invoke_context.environment_config.sysvar_cache();
            let clock = sysvar_cache.get_clock()?;
            let rent = sysvar_cache.get_rent()?;

            drop(me);
            vote_state::initialize_account_v2(
                &instruction_context,
                /* vote_account_index */ 0,
                target_version,
                &vote_init_v2,
                /* inflation_rewards_collector_index */ 2,
                /* block_revenue_collector_index */ 3,
                &signers,
                &clock,
                &rent,
                consume_pop_compute_units,
            )
        }
        VoteInstruction::UpdateCommissionBps {
            commission_bps,
            kind,
        } => vote_state::update_commission_bps(
            &mut me,
            target_version,
            commission_bps,
            kind,
            &signers,
            invoke_context.get_feature_set().block_revenue_sharing,
        ),
        VoteInstruction::UpdateCommissionCollector(kind) => {
            instruction_context.check_number_of_instruction_accounts(3)?;

            let rent = invoke_context
                .environment_config
                .sysvar_cache()
                .get_rent()?;

            drop(me);
            vote_state::update_commission_collector(
                &instruction_context,
                /* vote_account_index */ 0,
                target_version,
                /* new_collector_index */ 1,
                kind,
                &signers,
                &rent,
            )
        }
        VoteInstruction::DepositDelegatorRewards { .. } => {
            return Err(InstructionError::InvalidInstructionData);
        }
    }
});

#[allow(clippy::arithmetic_side_effects)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            vote_error::VoteError,
            vote_instruction::{VoteInstruction, authorize_checked},
            vote_state::{
                self, VoteAuthorize, VoteAuthorizeCheckedWithSeedArgs, VoteAuthorizeWithSeedArgs,
                VoteInit, VoteInitV2, VoteStateV4, VoteStateVersions,
                create_bls_pubkey_and_proof_of_possession, handler::VoteStateHandler,
            },
        },
        solana_account::{
            Account, AccountSharedData, ReadableAccount, WritableAccount,
            state_traits::StateMutWincode as _,
        },
        solana_clock::Clock,
        solana_epoch_schedule::EpochSchedule,
        solana_instruction::{AccountMeta, Instruction},
        solana_program_runtime::{
            invoke_context::mock_process_instruction_with_feature_set,
            program_cache::entry::ProgramCacheEntry,
            solana_sbpf::{program::BuiltinFunctionDefinition, vm::ContextObject},
        },
        solana_pubkey::Pubkey,
        solana_rent::Rent,
        solana_sdk_ids::sysvar,
        solana_slot_hashes::SlotHashes,
        solana_svm_feature_set::SVMFeatureSet,
        solana_sysvar_id::SysvarId,
        solana_vote_interface::state::{
            BLS_PROOF_OF_POSSESSION_COMPRESSED_SIZE, BLS_PUBLIC_KEY_COMPRESSED_SIZE,
            VoterWithBLSArgs,
        },
        std::{cell::RefCell, collections::HashSet, str::FromStr, sync::Arc},
        test_case::test_matrix,
    };

    fn create_sysvar_account<T>(value: &T) -> AccountSharedData
    where
        T: wincode::SchemaWrite<solana_account::WincodeConfig, Src = T>
            + for<'de> wincode::SchemaRead<'de, solana_account::WincodeConfig, Dst = T>
            + SysvarId,
    {
        let serialized_len = wincode::config::serialized_size(value, solana_account::WINCODE_CONFIG)
            .unwrap() as usize;
        let canonical_data_len = match T::id() {
            sysvar::clock::ID => solana_clock::SIZE,
            sysvar::epoch_schedule::ID => solana_epoch_schedule::SIZE,
            sysvar::rent::ID => solana_rent::SIZE,
            sysvar::slot_hashes::ID => solana_slot_hashes::SIZE,
            id => panic!("unsupported sysvar: {id}"),
        };
        let required_data_len = canonical_data_len.max(serialized_len);
        AccountSharedData::new_data_with_space(1, value, required_data_len, &sysvar::id()).unwrap()
    }

    fn vote_state_size_of() -> usize {
        VoteStateV4::size_of()
    }

    fn deserialize_vote_state_for_test(
        account_data: &[u8],
        vote_pubkey: &Pubkey,
    ) -> VoteStateHandler {
        VoteStateHandler::new_v4(VoteStateV4::deserialize(account_data, vote_pubkey).unwrap())
    }

    fn create_default_account() -> AccountSharedData {
        AccountSharedData::empty(0, &Pubkey::new_unique())
    }

    #[derive(Clone, Copy, Default)]
    struct VoteProgramFeatures {
        block_revenue_sharing: bool,
        vote_account_initialize_v2: bool,
    }

    impl VoteProgramFeatures {
        fn all_enabled() -> Self {
            Self {
                block_revenue_sharing: true,
                vote_account_initialize_v2: true,
            }
        }
    }

    fn process_instruction(
        features: VoteProgramFeatures,
        instruction_data: &[u8],
        transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
        instruction_accounts: Vec<AccountMeta>,
        expected_result: Result<(), InstructionError>,
    ) -> Vec<AccountSharedData> {
        process_instruction_with_cu_check(
            features,
            instruction_data,
            transaction_accounts,
            instruction_accounts,
            expected_result,
            DEFAULT_COMPUTE_UNITS,
        )
    }

    fn process_instruction_with_cu_check(
        features: VoteProgramFeatures,
        instruction_data: &[u8],
        transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
        instruction_accounts: Vec<AccountMeta>,
        expected_result: Result<(), InstructionError>,
        expected_cus: u64,
    ) -> Vec<AccountSharedData> {
        let VoteProgramFeatures {
            block_revenue_sharing,
            vote_account_initialize_v2,
        } = features;
        let cu_consumed = RefCell::new(0u64);
        let accounts = mock_process_instruction_with_feature_set(
            &id(),
            instruction_data,
            transaction_accounts,
            instruction_accounts,
            expected_result,
            Entrypoint::register,
            |invoke_context| {
                // Register system program for CPI support.
                invoke_context.program_cache_for_tx_batch.replenish(
                    solana_sdk_ids::system_program::id(),
                    Arc::new(ProgramCacheEntry::new_builtin(
                        solana_system_program::system_processor::Entrypoint::register,
                    )),
                );
                *cu_consumed.borrow_mut() = invoke_context.get_remaining();
            },
            |invoke_context| {
                *cu_consumed.borrow_mut() -= invoke_context.get_remaining();
            },
            &SVMFeatureSet {
                block_revenue_sharing,
                vote_account_initialize_v2,
                ..SVMFeatureSet::all_enabled()
            },
        );
        assert_eq!(
            *cu_consumed.borrow(),
            expected_cus,
            "Expected {} CU consumed, got {}",
            expected_cus,
            *cu_consumed.borrow()
        );
        accounts
    }

    fn process_instruction_as_one_arg(
        features: VoteProgramFeatures,
        instruction: &Instruction,
        expected_result: Result<(), InstructionError>,
    ) -> Vec<AccountSharedData> {
        process_instruction_as_one_arg_with_cu_check(
            features,
            instruction,
            expected_result,
            DEFAULT_COMPUTE_UNITS,
        )
    }

    fn process_instruction_as_one_arg_with_cu_check(
        features: VoteProgramFeatures,
        instruction: &Instruction,
        expected_result: Result<(), InstructionError>,
        expected_cus: u64,
    ) -> Vec<AccountSharedData> {
        let mut pubkeys: HashSet<Pubkey> = instruction
            .accounts
            .iter()
            .map(|meta| meta.pubkey)
            .collect();
        pubkeys.insert(sysvar::clock::id());
        pubkeys.insert(sysvar::epoch_schedule::id());
        pubkeys.insert(sysvar::rent::id());
        pubkeys.insert(sysvar::slot_hashes::id());
        let transaction_accounts: Vec<_> = pubkeys
            .iter()
            .map(|pubkey| {
                (
                    *pubkey,
                    if sysvar::clock::check_id(pubkey) {
                        create_sysvar_account(&Clock::default())
                    } else if sysvar::epoch_schedule::check_id(pubkey) {
                        create_sysvar_account(&EpochSchedule::without_warmup())
                    } else if sysvar::slot_hashes::check_id(pubkey) {
                        create_sysvar_account(&SlotHashes::default())
                    } else if sysvar::rent::check_id(pubkey) {
                        create_sysvar_account(&Rent::free())
                    } else if *pubkey == invalid_vote_state_pubkey() {
                        AccountSharedData::from(Account {
                            owner: invalid_vote_state_pubkey(),
                            ..Account::default()
                        })
                    } else {
                        AccountSharedData::from(Account {
                            owner: id(),
                            ..Account::default()
                        })
                    },
                )
            })
            .collect();
        process_instruction_with_cu_check(
            features,
            &instruction.data,
            transaction_accounts,
            instruction.accounts.clone(),
            expected_result,
            expected_cus,
        )
    }

    fn invalid_vote_state_pubkey() -> Pubkey {
        Pubkey::from_str("BadVote111111111111111111111111111111111111").unwrap()
    }

    fn create_default_rent_account() -> AccountSharedData {
        create_sysvar_account(&Rent::free())
    }

    fn create_default_clock_account() -> AccountSharedData {
        create_sysvar_account(&Clock::default())
    }

    fn create_test_account() -> (Pubkey, AccountSharedData) {
        let rent = Rent::default();
        let vote_pubkey = solana_pubkey::new_rand();
        let node_pubkey = solana_pubkey::new_rand();

        let balance = rent.minimum_balance(VoteStateV4::size_of());
        let account = vote_state::create_v4_account_with_authorized(
            &node_pubkey,
            &vote_pubkey,
            [0u8; BLS_PUBLIC_KEY_COMPRESSED_SIZE],
            &vote_pubkey,
            0,
            &vote_pubkey,
            0,
            &node_pubkey,
            balance,
        );

        (vote_pubkey, account)
    }

    /// Create a V4 vote account with `bls_pubkey_compressed: None`, mirroring
    /// what an account looks like after `InitializeAccount` (the v1
    /// instruction) but before any BLS pubkey has been registered.
    fn create_test_account_no_bls_key() -> (Pubkey, AccountSharedData) {
        let rent = Rent::default();
        let vote_pubkey = solana_pubkey::new_rand();
        let node_pubkey = solana_pubkey::new_rand();
        let balance = rent.minimum_balance(VoteStateV4::size_of());

        let vote_state = VoteStateV4::new_with_defaults(
            &vote_pubkey,
            &VoteInit {
                node_pubkey,
                authorized_voter: vote_pubkey,
                authorized_withdrawer: vote_pubkey,
                commission: 0,
            },
            &Clock::default(),
        );
        let account = AccountSharedData::new_data_with_space(
            balance,
            &VoteStateVersions::V4(Box::new(vote_state)),
            VoteStateV4::size_of(),
            &id(),
        )
        .unwrap();

        (vote_pubkey, account)
    }

    fn create_test_account_with_authorized() -> (Pubkey, Pubkey, Pubkey, AccountSharedData) {
        let vote_pubkey = solana_pubkey::new_rand();
        let authorized_voter = solana_pubkey::new_rand();
        let authorized_withdrawer = solana_pubkey::new_rand();
        let account =
            create_test_account_with_provided_authorized(&authorized_voter, &authorized_withdrawer);

        (
            vote_pubkey,
            authorized_voter,
            authorized_withdrawer,
            account,
        )
    }

    fn create_test_account_with_provided_authorized(
        authorized_voter: &Pubkey,
        authorized_withdrawer: &Pubkey,
    ) -> AccountSharedData {
        let node_pubkey = solana_pubkey::new_rand();

        vote_state::create_v4_account_with_authorized(
            &node_pubkey,
            authorized_voter,
            [0u8; BLS_PUBLIC_KEY_COMPRESSED_SIZE],
            authorized_withdrawer,
            0,
            authorized_withdrawer,
            0,
            &node_pubkey,
            100,
        )
    }

    #[test_matrix(
        [false, true],
        [false, true]
    )]
    fn test_initialize_vote_account(block_revenue_sharing: bool, vote_account_initialize_v2: bool) {
        let vote_pubkey = solana_pubkey::new_rand();
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
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
    fn test_initialize_vote_account_v2(
        block_revenue_sharing: bool,
        vote_account_initialize_v2: bool,
    ) {
        let vote_pubkey = solana_pubkey::new_rand();
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());
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
        let instruction_data =
            wincode::serialize(&VoteInstruction::InitializeAccountV2(VoteInitV2 {
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
                assert_eq!(
                    v4.block_revenue_commission_bps,
                    block_revenue_commission_bps
                );
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
                authorized_voter_bls_proof_of_possession: [2u8;
                    BLS_PROOF_OF_POSSESSION_COMPRESSED_SIZE],
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
                    create_sysvar_account(&solana_epoch_schedule::EpochSchedule::without_warmup()),
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

        let collector_account = AccountSharedData::empty(
            rent.minimum_balance(0),
            &solana_sdk_ids::system_program::id(),
        );

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
        let vote_account =
            AccountSharedData::new_with_data(100, vec![0; vote_state_size_of()], &id());

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

            let instruction_data =
                wincode::serialize(&VoteInstruction::UpdateCommission(42)).unwrap();

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
                wincode::serialize(&VoteInstruction::AuthorizeChecked(VoteAuthorize::Voter))
                    .unwrap();

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

}
