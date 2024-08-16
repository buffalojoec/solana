//! SVM rent manager. A plugin for managing account rent state.
//!
//! Consumers of the SVM API can configure custom rent behavior by implementing
//! the `SVMRentManager` trait.

use {
    log::*,
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        clock::Epoch,
        pubkey::Pubkey,
        rent::{Rent, RentDue},
        rent_collector::CollectedInfo,
        transaction::{Result, TransactionError},
        transaction_context::{IndexOfAccount, TransactionContext},
    },
};

/// Account rent state.
#[derive(Debug, PartialEq, Eq)]
pub enum RentState {
    /// account.lamports == 0
    Uninitialized,
    /// 0 < account.lamports < rent-exempt-minimum
    RentPaying {
        lamports: u64,    // account.lamports()
        data_size: usize, // account.data().len()
    },
    /// account.lamports >= rent-exempt-minimum
    RentExempt,
}

impl RentState {
    /// Return a new RentState instance for a given account and rent.
    pub fn from_account(account: &AccountSharedData, rent: &Rent) -> Self {
        if account.lamports() == 0 {
            Self::Uninitialized
        } else if rent.is_exempt(account.lamports(), account.data().len()) {
            Self::RentExempt
        } else {
            Self::RentPaying {
                data_size: account.data().len(),
                lamports: account.lamports(),
            }
        }
    }
}

/// Rent manager trait.
pub trait SVMRentManager {
    /// Check rent state transition for an account in a transaction.
    ///
    /// This method has a default implementation that calls into
    /// `check_rent_state_with_account`.
    fn check_rent_state(
        &self,
        pre_rent_state: Option<&RentState>,
        post_rent_state: Option<&RentState>,
        transaction_context: &TransactionContext,
        index: IndexOfAccount,
    ) -> Result<()> {
        if let Some((pre_rent_state, post_rent_state)) = pre_rent_state.zip(post_rent_state) {
            let expect_msg =
                "account must exist at TransactionContext index if rent-states are Some";
            self.check_rent_state_with_account(
                pre_rent_state,
                post_rent_state,
                transaction_context
                    .get_key_of_account_at_index(index)
                    .expect(expect_msg),
                &transaction_context
                    .get_account_at_index(index)
                    .expect(expect_msg)
                    .borrow(),
                index,
            )?;
        }
        Ok(())
    }

    /// Check rent state transition for an account directly.
    ///
    /// This method has a default implementation that checks whether the
    /// account is rent exempt.
    fn check_rent_state_with_account(
        &self,
        pre_rent_state: &RentState,
        post_rent_state: &RentState,
        address: &Pubkey,
        account_state: &AccountSharedData,
        account_index: IndexOfAccount,
    ) -> Result<()> {
        if !solana_sdk::incinerator::check_id(address)
            && !self.transition_allowed(pre_rent_state, post_rent_state)
        {
            debug!(
                "Account {} not rent exempt, state {:?}",
                address, account_state,
            );
            let account_index = account_index as u8;
            Err(TransactionError::InsufficientFundsForRent { account_index })
        } else {
            Ok(())
        }
    }

    /// Collect rent from an account.
    fn collect_from_existing_account(
        &self,
        address: &Pubkey,
        account: &mut AccountSharedData,
    ) -> CollectedInfo;

    /// Determine the rent state of an account.
    ///
    /// This method has a default implementation that uses the `get_rent`
    /// method to determine rent exemption.
    fn get_account_rent_state(&self, account: &AccountSharedData) -> RentState {
        RentState::from_account(account, self.get_rent())
    }

    /// Get the rent manager's rent instance.
    fn get_rent(&self) -> &Rent;

    /// Get the rent due for an account.
    fn get_rent_due(&self, lamports: u64, data_len: usize, account_rent_epoch: Epoch) -> RentDue;

    /// Determine whether a rent state transition is allowed between two
    /// states.
    ///
    /// This method has a default implementation that allows all transitions
    /// to `RentState::Uninitialized` and `RentState::RentExempt`, and
    /// disallows transitions to `RentState::RentPaying` if the account is
    /// resized or credited.
    fn transition_allowed(&self, pre_rent_state: &RentState, post_rent_state: &RentState) -> bool {
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
