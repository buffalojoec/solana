use {
    crate::{rent_state::RentState, svm_rent_state::SVMRentState},
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        pubkey::Pubkey,
        rent::Rent,
        rent_collector::RentCollector,
        transaction::{Result, TransactionError},
        transaction_context::IndexOfAccount,
    },
};

impl SVMRentState for RentCollector {
    type RentState = RentState;

    fn check_rent_state_with_account(
        pre_rent_state: &Self::RentState,
        post_rent_state: &Self::RentState,
        address: &Pubkey,
        _account_state: &AccountSharedData,
        account_index: IndexOfAccount,
    ) -> Result<()> {
        if !solana_sdk::incinerator::check_id(address)
            && !Self::transition_allowed(pre_rent_state, post_rent_state)
        {
            let account_index = account_index as u8;
            Err(TransactionError::InsufficientFundsForRent { account_index })
        } else {
            Ok(())
        }
    }

    fn get_account_rent_state(account: &AccountSharedData, rent: &Rent) -> Self::RentState {
        if account.lamports() == 0 {
            RentState::Uninitialized
        } else if rent.is_exempt(account.lamports(), account.data().len()) {
            RentState::RentExempt
        } else {
            RentState::RentPaying {
                data_size: account.data().len(),
                lamports: account.lamports(),
            }
        }
    }

    fn transition_allowed(
        pre_rent_state: &Self::RentState,
        post_rent_state: &Self::RentState,
    ) -> bool {
        match post_rent_state {
            RentState::Uninitialized | RentState::RentExempt => true,
            RentState::RentPaying {
                data_size: post_data_size,
                lamports: post_lamports,
            } => {
                match pre_rent_state {
                    RentState::Uninitialized | RentState::RentExempt => false,
                    RentState::RentPaying {
                        data_size: pre_data_size,
                        lamports: pre_lamports,
                    } => {
                        // Cannot remain RentPaying if resized or credited.
                        post_data_size == pre_data_size && post_lamports <= pre_lamports
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        solana_sdk::{
            clock::Epoch, epoch_schedule::EpochSchedule, pubkey::Pubkey,
            transaction_context::TransactionContext,
        },
    };

    #[test]
    fn test_get_account_rent_state() {
        let program_id = Pubkey::new_unique();
        let uninitialized_account = AccountSharedData::new(0, 0, &Pubkey::default());

        let account_data_size = 100;

        let rent = Rent::free();
        let rent_collector =
            RentCollector::new(Epoch::default(), EpochSchedule::default(), 0.0, rent);

        let rent_exempt_account = AccountSharedData::new(1, account_data_size, &program_id); // if rent is free, all accounts with non-zero lamports and non-empty data are rent-exempt

        assert_eq!(
            RentCollector::get_account_rent_state(&uninitialized_account, &rent_collector.rent),
            RentState::Uninitialized
        );
        assert_eq!(
            RentCollector::get_account_rent_state(&rent_exempt_account, &rent_collector.rent),
            RentState::RentExempt
        );

        let rent = Rent::default();
        let rent_minimum_balance = rent.minimum_balance(account_data_size);
        let rent_paying_account = AccountSharedData::new(
            rent_minimum_balance.saturating_sub(1),
            account_data_size,
            &program_id,
        );
        let rent_exempt_account = AccountSharedData::new(
            rent.minimum_balance(account_data_size),
            account_data_size,
            &program_id,
        );

        assert_eq!(
            RentCollector::get_account_rent_state(&uninitialized_account, &rent),
            RentState::Uninitialized
        );
        assert_eq!(
            RentCollector::get_account_rent_state(&rent_paying_account, &rent),
            RentState::RentPaying {
                data_size: account_data_size,
                lamports: rent_paying_account.lamports(),
            }
        );
        assert_eq!(
            RentCollector::get_account_rent_state(&rent_exempt_account, &rent),
            RentState::RentExempt
        );
    }

    #[test]
    fn test_transition_allowed_from() {
        let post_rent_state = RentState::Uninitialized;
        assert!(RentCollector::transition_allowed(
            &RentState::Uninitialized,
            &post_rent_state
        ));
        assert!(RentCollector::transition_allowed(
            &RentState::RentExempt,
            &post_rent_state
        ));
        assert!(RentCollector::transition_allowed(
            &RentState::RentPaying {
                data_size: 0,
                lamports: 1,
            },
            &post_rent_state
        ));

        let post_rent_state = RentState::RentExempt;
        assert!(RentCollector::transition_allowed(
            &RentState::Uninitialized,
            &post_rent_state
        ));
        assert!(RentCollector::transition_allowed(
            &RentState::RentExempt,
            &post_rent_state
        ));
        assert!(RentCollector::transition_allowed(
            &RentState::RentPaying {
                data_size: 0,
                lamports: 1,
            },
            &post_rent_state
        ));

        let post_rent_state = RentState::RentPaying {
            data_size: 2,
            lamports: 5,
        };
        assert!(RentCollector::transition_allowed(
            &RentState::Uninitialized,
            &post_rent_state
        ));
        assert!(RentCollector::transition_allowed(
            &RentState::RentExempt,
            &post_rent_state
        ));
        assert!(RentCollector::transition_allowed(
            &RentState::RentPaying {
                data_size: 3,
                lamports: 5,
            },
            &post_rent_state
        ));
        assert!(RentCollector::transition_allowed(
            &RentState::RentPaying {
                data_size: 1,
                lamports: 5,
            },
            &post_rent_state
        ));

        // Transition is always allowed if there is no account data resize or
        // change in account's lamports.
        assert!(RentCollector::transition_allowed(
            &RentState::RentPaying {
                data_size: 2,
                lamports: 5,
            },
            &post_rent_state
        ));
        // Transition is always allowed if there is no account data resize and
        // account's lamports is reduced.
        assert!(RentCollector::transition_allowed(
            &RentState::RentPaying {
                data_size: 2,
                lamports: 7,
            },
            &post_rent_state
        ));
        // Transition is not allowed if the account is credited with more
        // lamports and remains rent-paying.
        assert!(RentCollector::transition_allowed(
            &RentState::RentPaying {
                data_size: 2,
                lamports: 3,
            },
            &post_rent_state
        ));
    }

    #[test]
    fn test_check_rent_state_with_account() {
        let pre_rent_state = RentState::RentPaying {
            data_size: 2,
            lamports: 3,
        };

        let post_rent_state = RentState::RentPaying {
            data_size: 2,
            lamports: 5,
        };
        let account_index = 2 as IndexOfAccount;
        let key = Pubkey::new_unique();
        let result = RentCollector::check_rent_state_with_account(
            &pre_rent_state,
            &post_rent_state,
            &key,
            &AccountSharedData::default(),
            account_index,
        );
        assert_eq!(
            result.err(),
            Some(TransactionError::InsufficientFundsForRent {
                account_index: account_index as u8
            })
        );

        let result = RentCollector::check_rent_state_with_account(
            &pre_rent_state,
            &post_rent_state,
            &solana_sdk::incinerator::id(),
            &AccountSharedData::default(),
            account_index,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_rent_state() {
        let context = TransactionContext::new(
            vec![(Pubkey::new_unique(), AccountSharedData::default())],
            Rent::default(),
            20,
            20,
        );

        let pre_rent_state = RentState::RentPaying {
            data_size: 2,
            lamports: 3,
        };

        let post_rent_state = RentState::RentPaying {
            data_size: 2,
            lamports: 5,
        };

        let result = RentCollector::check_rent_state(
            Some(&pre_rent_state),
            Some(&post_rent_state),
            &context,
            0,
        );
        assert_eq!(
            result.err(),
            Some(TransactionError::InsufficientFundsForRent { account_index: 0 })
        );

        let result = RentCollector::check_rent_state(None, Some(&post_rent_state), &context, 0);
        assert!(result.is_ok());
    }
}
