use {
    crate::{account_rent_state::RentState, transaction_error_metrics::TransactionErrorMetrics},
    solana_program_runtime::loaded_programs::ProgramCacheMatchCriteria,
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        feature_set::{self, FeatureSet},
        nonce::State as NonceState,
        pubkey::Pubkey,
        rent::RentDue,
        rent_collector::{CollectedInfo, RentCollector, RENT_EXEMPT_RENT_EPOCH},
        transaction::{Result, TransactionError},
        transaction_context::IndexOfAccount,
    },
    solana_system_program::{get_system_account_kind, SystemAccountKind},
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

#[cfg(test)]
mod tests {
    use {
        super::*,
        nonce::state::Versions as NonceVersions,
        solana_sdk::{
            account::Account,
            epoch_schedule::EpochSchedule,
            nonce,
            rent::Rent,
            signature::{Keypair, Signer},
            system_program,
        },
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
}
