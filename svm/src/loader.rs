use {
    crate::{account_rent_state::RentState, transaction_error_metrics::TransactionErrorMetrics},
    solana_compute_budget::compute_budget_processor::process_compute_budget_instructions,
    solana_program_runtime::loaded_programs::{ProgramCacheEntry, ProgramCacheMatchCriteria},
    solana_sdk::{
        account::{Account, AccountSharedData, ReadableAccount, WritableAccount},
        feature_set::{self, FeatureSet},
        message::SanitizedMessage,
        nonce::State as NonceState,
        pubkey::Pubkey,
        rent::RentDue,
        rent_collector::{CollectedInfo, RentCollector, RENT_EXEMPT_RENT_EPOCH},
        saturating_add_assign,
        sysvar::{self, instructions::construct_instructions_data},
        transaction::{Result, TransactionError},
        transaction_context::IndexOfAccount,
    },
    solana_system_program::{get_system_account_kind, SystemAccountKind},
    std::num::NonZeroUsize,
};

/// The "loader" required by the transaction batch processor, responsible
/// mainly for loading accounts.
pub trait Loader {
    /// Load the account at the provided address.
    fn load_account(&self, pubkey: &Pubkey) -> Option<AccountSharedData>;

    /// Determine whether or not an account is owned by one of the programs in
    /// the provided set.
    ///
    /// This function has a default implementation, but projects can override
    /// it if they want to provide a more efficient implementation, such as
    /// checking account ownership without fully loading.
    fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
        self.load_account(account)
            .and_then(|account| owners.iter().position(|entry| account.owner() == entry))
    }

    /// Collect rent from an account if rent is still enabled and regardless of
    /// whether rent is enabled, set the rent epoch to u64::MAX if the account is
    /// rent exempt.
    ///
    /// This function has a default implementation, but projects can override
    /// it if they want to provide a custom implementation.
    fn collect_rent_from_account(
        &self,
        feature_set: &FeatureSet,
        rent_collector: &RentCollector,
        address: &Pubkey,
        account: &mut AccountSharedData,
    ) -> CollectedInfo {
        if !feature_set.is_active(&feature_set::disable_rent_fees_collection::id()) {
            rent_collector.collect_from_existing_account(address, account)
        } else {
            // When rent fee collection is disabled, we won't collect rent for any account. If there
            // are any rent paying accounts, their `rent_epoch` won't change either. However, if the
            // account itself is rent-exempted but its `rent_epoch` is not u64::MAX, we will set its
            // `rent_epoch` to u64::MAX. In such case, the behavior stays the same as before.
            if account.rent_epoch() != RENT_EXEMPT_RENT_EPOCH
                && rent_collector.get_rent_due(
                    account.lamports(),
                    account.data().len(),
                    account.rent_epoch(),
                ) == RentDue::Exempt
            {
                account.set_rent_epoch(RENT_EXEMPT_RENT_EPOCH);
            }

            CollectedInfo::default()
        }
    }

    /// Check whether the payer_account is capable of paying the fee. The
    /// side effect is to subtract the fee amount from the payer_account
    /// balance of lamports. If the payer_acount is not able to pay the
    /// fee, the error_metrics is incremented, and a specific error is
    /// returned.
    ///
    /// This function has a default implementation, but projects can override
    /// it if they want to provide a custom implementation.
    fn validate_fee_payer(
        &self,
        payer_address: &Pubkey,
        payer_account: &mut AccountSharedData,
        payer_index: IndexOfAccount,
        error_metrics: &mut TransactionErrorMetrics,
        rent_collector: &RentCollector,
        fee: u64,
    ) -> Result<()> {
        if payer_account.lamports() == 0 {
            error_metrics.account_not_found += 1;
            return Err(TransactionError::AccountNotFound);
        }
        let system_account_kind = get_system_account_kind(payer_account).ok_or_else(|| {
            error_metrics.invalid_account_for_fee += 1;
            TransactionError::InvalidAccountForFee
        })?;
        let min_balance = match system_account_kind {
            SystemAccountKind::System => 0,
            SystemAccountKind::Nonce => {
                // Should we ever allow a fees charge to zero a nonce account's
                // balance. The state MUST be set to uninitialized in that case
                rent_collector.rent.minimum_balance(NonceState::size())
            }
        };

        payer_account
            .lamports()
            .checked_sub(min_balance)
            .and_then(|v| v.checked_sub(fee))
            .ok_or_else(|| {
                error_metrics.insufficient_funds += 1;
                TransactionError::InsufficientFundsForFee
            })?;

        let payer_pre_rent_state = RentState::from_account(payer_account, &rent_collector.rent);
        payer_account
            .checked_sub_lamports(fee)
            .map_err(|_| TransactionError::InsufficientFundsForFee)?;

        let payer_post_rent_state = RentState::from_account(payer_account, &rent_collector.rent);
        RentState::check_rent_state_with_account(
            &payer_pre_rent_state,
            &payer_post_rent_state,
            payer_address,
            payer_account,
            payer_index,
        )
    }

    fn get_program_match_criteria(&self, _program: &Pubkey) -> ProgramCacheMatchCriteria {
        ProgramCacheMatchCriteria::NoCriteria
    }

    fn add_builtin_account(&self, _name: &str, _program_id: &Pubkey) {}
}

// Temporarily `pub(crate)`.
/// Total accounts data a transaction can load is limited to
///   if `set_tx_loaded_accounts_data_size` instruction is not activated or not used, then
///     default value of 64MiB to not break anyone in Mainnet-beta today
///   else
///     user requested loaded accounts size.
///     Note, requesting zero bytes will result transaction error
pub(crate) fn get_requested_loaded_accounts_data_size_limit(
    sanitized_message: &SanitizedMessage,
) -> Result<Option<NonZeroUsize>> {
    let compute_budget_limits =
        process_compute_budget_instructions(sanitized_message.program_instructions_iter())
            .unwrap_or_default();
    // sanitize against setting size limit to zero
    NonZeroUsize::new(
        usize::try_from(compute_budget_limits.loaded_accounts_bytes).unwrap_or_default(),
    )
    .map_or(
        Err(TransactionError::InvalidLoadedAccountsDataSizeLimit),
        |v| Ok(Some(v)),
    )
}

// Temporarily `pub(crate)`.
pub(crate) fn account_shared_data_from_program(
    loaded_program: &ProgramCacheEntry,
) -> AccountSharedData {
    // It's an executable program account. The program is already loaded in the cache.
    // So the account data is not needed. Return a dummy AccountSharedData with meta
    // information.
    let mut program_account = AccountSharedData::default();
    program_account.set_owner(loaded_program.account_owner());
    program_account.set_executable(true);
    program_account
}

// Temporarily `pub(crate)`.
/// Accumulate loaded account data size into `accumulated_accounts_data_size`.
/// Returns TransactionErr::MaxLoadedAccountsDataSizeExceeded if
/// `requested_loaded_accounts_data_size_limit` is specified and
/// `accumulated_accounts_data_size` exceeds it.
pub(crate) fn accumulate_and_check_loaded_account_data_size(
    accumulated_loaded_accounts_data_size: &mut usize,
    account_data_size: usize,
    requested_loaded_accounts_data_size_limit: Option<NonZeroUsize>,
    error_metrics: &mut TransactionErrorMetrics,
) -> Result<()> {
    if let Some(requested_loaded_accounts_data_size) = requested_loaded_accounts_data_size_limit {
        saturating_add_assign!(*accumulated_loaded_accounts_data_size, account_data_size);
        if *accumulated_loaded_accounts_data_size > requested_loaded_accounts_data_size.get() {
            error_metrics.max_loaded_accounts_data_size_exceeded += 1;
            Err(TransactionError::MaxLoadedAccountsDataSizeExceeded)
        } else {
            Ok(())
        }
    } else {
        Ok(())
    }
}

// Temporarily `pub(crate)`.
pub(crate) fn construct_instructions_account(message: &SanitizedMessage) -> AccountSharedData {
    AccountSharedData::from(Account {
        data: construct_instructions_data(&message.decompile_instructions()),
        owner: sysvar::id(),
        ..Account::default()
    })
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        nonce::state::Versions as NonceVersions,
        solana_compute_budget::compute_budget_processor,
        solana_sdk::{
            epoch_schedule::EpochSchedule,
            hash::Hash,
            message::{
                v0::{LoadedAddresses, LoadedMessage},
                Message,
            },
            nonce,
            rent::Rent,
            signature::{Keypair, Signer},
            system_program,
            transaction::{SanitizedTransaction, Transaction},
        },
        std::borrow::Cow,
    };

    struct SimpleMockLoader;

    impl Loader for SimpleMockLoader {
        fn load_account(&self, _pubkey: &Pubkey) -> Option<AccountSharedData> {
            None
        }
    }

    /// get a feature set with all features activated
    /// with the optional except of 'exclude'
    fn all_features_except(exclude: Option<&[Pubkey]>) -> FeatureSet {
        let mut features = FeatureSet::all_enabled();
        if let Some(exclude) = exclude {
            features.active.retain(|k, _v| !exclude.contains(k));
        }
        features
    }

    #[test]
    fn test_collect_rent_from_account() {
        let feature_set = FeatureSet::all_enabled();
        let rent_collector = RentCollector {
            epoch: 1,
            ..RentCollector::default()
        };

        let address = Pubkey::new_unique();
        let min_exempt_balance = rent_collector.rent.minimum_balance(0);
        let mut account = AccountSharedData::from(Account {
            lamports: min_exempt_balance,
            ..Account::default()
        });

        assert_eq!(
            SimpleMockLoader.collect_rent_from_account(
                &feature_set,
                &rent_collector,
                &address,
                &mut account
            ),
            CollectedInfo::default()
        );
        assert_eq!(account.rent_epoch(), RENT_EXEMPT_RENT_EPOCH);
    }

    #[test]
    fn test_collect_rent_from_account_rent_paying() {
        let feature_set = FeatureSet::all_enabled();
        let rent_collector = RentCollector {
            epoch: 1,
            ..RentCollector::default()
        };

        let address = Pubkey::new_unique();
        let mut account = AccountSharedData::from(Account {
            lamports: 1,
            ..Account::default()
        });

        assert_eq!(
            SimpleMockLoader.collect_rent_from_account(
                &feature_set,
                &rent_collector,
                &address,
                &mut account
            ),
            CollectedInfo::default()
        );
        assert_eq!(account.rent_epoch(), 0);
        assert_eq!(account.lamports(), 1);
    }

    #[test]
    fn test_collect_rent_from_account_rent_enabled() {
        let feature_set =
            all_features_except(Some(&[feature_set::disable_rent_fees_collection::id()]));
        let rent_collector = RentCollector {
            epoch: 1,
            ..RentCollector::default()
        };

        let address = Pubkey::new_unique();
        let mut account = AccountSharedData::from(Account {
            lamports: 1,
            data: vec![0],
            ..Account::default()
        });

        assert_eq!(
            SimpleMockLoader.collect_rent_from_account(
                &feature_set,
                &rent_collector,
                &address,
                &mut account
            ),
            CollectedInfo {
                rent_amount: 1,
                account_data_len_reclaimed: 1
            }
        );
        assert_eq!(account.rent_epoch(), 0);
        assert_eq!(account.lamports(), 0);
    }

    struct ValidateFeePayerTestParameter {
        is_nonce: bool,
        payer_init_balance: u64,
        fee: u64,
        expected_result: Result<()>,
        payer_post_balance: u64,
    }

    fn validate_fee_payer_account(
        test_parameter: ValidateFeePayerTestParameter,
        rent_collector: &RentCollector,
    ) {
        let payer_account_keys = Keypair::new();
        let mut account = if test_parameter.is_nonce {
            AccountSharedData::new_data(
                test_parameter.payer_init_balance,
                &NonceVersions::new(NonceState::Initialized(nonce::state::Data::default())),
                &system_program::id(),
            )
            .unwrap()
        } else {
            AccountSharedData::new(test_parameter.payer_init_balance, 0, &system_program::id())
        };

        let result = SimpleMockLoader.validate_fee_payer(
            &payer_account_keys.pubkey(),
            &mut account,
            0,
            &mut TransactionErrorMetrics::default(),
            rent_collector,
            test_parameter.fee,
        );

        assert_eq!(result, test_parameter.expected_result);
        assert_eq!(account.lamports(), test_parameter.payer_post_balance);
    }

    #[test]
    fn test_validate_fee_payer() {
        let rent_collector = RentCollector::new(
            0,
            EpochSchedule::default(),
            500_000.0,
            Rent {
                lamports_per_byte_year: 1,
                ..Rent::default()
            },
        );
        let min_balance = rent_collector.rent.minimum_balance(NonceState::size());
        let fee = 5_000;

        // If payer account has sufficient balance, expect successful fee deduction,
        // regardless feature gate status, or if payer is nonce account.
        {
            for (is_nonce, min_balance) in [(true, min_balance), (false, 0)] {
                validate_fee_payer_account(
                    ValidateFeePayerTestParameter {
                        is_nonce,
                        payer_init_balance: min_balance + fee,
                        fee,
                        expected_result: Ok(()),
                        payer_post_balance: min_balance,
                    },
                    &rent_collector,
                );
            }
        }

        // If payer account has no balance, expected AccountNotFound Error
        // regardless feature gate status, or if payer is nonce account.
        {
            for is_nonce in [true, false] {
                validate_fee_payer_account(
                    ValidateFeePayerTestParameter {
                        is_nonce,
                        payer_init_balance: 0,
                        fee,
                        expected_result: Err(TransactionError::AccountNotFound),
                        payer_post_balance: 0,
                    },
                    &rent_collector,
                );
            }
        }

        // If payer account has insufficient balance, expect InsufficientFundsForFee error
        // regardless feature gate status, or if payer is nonce account.
        {
            for (is_nonce, min_balance) in [(true, min_balance), (false, 0)] {
                validate_fee_payer_account(
                    ValidateFeePayerTestParameter {
                        is_nonce,
                        payer_init_balance: min_balance + fee - 1,
                        fee,
                        expected_result: Err(TransactionError::InsufficientFundsForFee),
                        payer_post_balance: min_balance + fee - 1,
                    },
                    &rent_collector,
                );
            }
        }

        // normal payer account has balance of u64::MAX, so does fee; since it does not  require
        // min_balance, expect successful fee deduction, regardless of feature gate status
        {
            validate_fee_payer_account(
                ValidateFeePayerTestParameter {
                    is_nonce: false,
                    payer_init_balance: u64::MAX,
                    fee: u64::MAX,
                    expected_result: Ok(()),
                    payer_post_balance: 0,
                },
                &rent_collector,
            );
        }
    }

    #[test]
    fn test_validate_nonce_fee_payer_with_checked_arithmetic() {
        let rent_collector = RentCollector::new(
            0,
            EpochSchedule::default(),
            500_000.0,
            Rent {
                lamports_per_byte_year: 1,
                ..Rent::default()
            },
        );

        // nonce payer account has balance of u64::MAX, so does fee; due to nonce account
        // requires additional min_balance, expect InsufficientFundsForFee error if feature gate is
        // enabled
        validate_fee_payer_account(
            ValidateFeePayerTestParameter {
                is_nonce: true,
                payer_init_balance: u64::MAX,
                fee: u64::MAX,
                expected_result: Err(TransactionError::InsufficientFundsForFee),
                payer_post_balance: u64::MAX,
            },
            &rent_collector,
        );
    }

    #[test]
    fn test_accumulate_and_check_loaded_account_data_size() {
        let mut error_metrics = TransactionErrorMetrics::default();

        // assert check is OK if data limit is not enabled
        {
            let mut accumulated_data_size: usize = 0;
            let data_size = usize::MAX;
            let requested_data_size_limit = None;

            assert!(accumulate_and_check_loaded_account_data_size(
                &mut accumulated_data_size,
                data_size,
                requested_data_size_limit,
                &mut error_metrics
            )
            .is_ok());
        }

        // assert check will fail with correct error if loaded data exceeds limit
        {
            let mut accumulated_data_size: usize = 0;
            let data_size: usize = 123;
            let requested_data_size_limit = NonZeroUsize::new(data_size);

            // OK - loaded data size is up to limit
            assert!(accumulate_and_check_loaded_account_data_size(
                &mut accumulated_data_size,
                data_size,
                requested_data_size_limit,
                &mut error_metrics
            )
            .is_ok());
            assert_eq!(data_size, accumulated_data_size);

            // fail - loading more data that would exceed limit
            let another_byte: usize = 1;
            assert_eq!(
                accumulate_and_check_loaded_account_data_size(
                    &mut accumulated_data_size,
                    another_byte,
                    requested_data_size_limit,
                    &mut error_metrics
                ),
                Err(TransactionError::MaxLoadedAccountsDataSizeExceeded)
            );
        }
    }

    #[test]
    fn test_get_requested_loaded_accounts_data_size_limit() {
        // an prrivate helper function
        fn test(
            instructions: &[solana_sdk::instruction::Instruction],
            expected_result: &Result<Option<NonZeroUsize>>,
        ) {
            let payer_keypair = Keypair::new();
            let tx = SanitizedTransaction::from_transaction_for_tests(Transaction::new(
                &[&payer_keypair],
                Message::new(instructions, Some(&payer_keypair.pubkey())),
                Hash::default(),
            ));
            assert_eq!(
                *expected_result,
                get_requested_loaded_accounts_data_size_limit(tx.message())
            );
        }

        let tx_not_set_limit = &[solana_sdk::instruction::Instruction::new_with_bincode(
            Pubkey::new_unique(),
            &0_u8,
            vec![],
        )];
        let tx_set_limit_99 =
            &[
                solana_sdk::compute_budget::ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(99u32),
                solana_sdk::instruction::Instruction::new_with_bincode(Pubkey::new_unique(), &0_u8, vec![]),
            ];
        let tx_set_limit_0 =
            &[
                solana_sdk::compute_budget::ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(0u32),
                solana_sdk::instruction::Instruction::new_with_bincode(Pubkey::new_unique(), &0_u8, vec![]),
            ];

        let result_default_limit = Ok(Some(
            NonZeroUsize::new(
                usize::try_from(compute_budget_processor::MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES)
                    .unwrap(),
            )
            .unwrap(),
        ));
        let result_requested_limit: Result<Option<NonZeroUsize>> =
            Ok(Some(NonZeroUsize::new(99).unwrap()));
        let result_invalid_limit = Err(TransactionError::InvalidLoadedAccountsDataSizeLimit);

        // the results should be:
        //    if tx doesn't set limit, then default limit (64MiB)
        //    if tx sets limit, then requested limit
        //    if tx sets limit to zero, then TransactionError::InvalidLoadedAccountsDataSizeLimit
        test(tx_not_set_limit, &result_default_limit);
        test(tx_set_limit_99, &result_requested_limit);
        test(tx_set_limit_0, &result_invalid_limit);
    }

    #[test]
    fn test_construct_instructions_account() {
        let loaded_message = LoadedMessage {
            message: Cow::Owned(solana_sdk::message::v0::Message::default()),
            loaded_addresses: Cow::Owned(LoadedAddresses::default()),
            is_writable_account_cache: vec![false],
        };
        let message = SanitizedMessage::V0(loaded_message);
        let shared_data = construct_instructions_account(&message);
        let expected = AccountSharedData::from(Account {
            data: construct_instructions_data(&message.decompile_instructions()),
            owner: sysvar::id(),
            ..Account::default()
        });
        assert_eq!(shared_data, expected);
    }
}
