//! Rent manager for the Agave runtime. Manages account rent state according to
//! the Solana protocol.
//!
//! Implements the AVM API's `SVMRentManager` trait.

use {
    log::*,
    solana_sdk::{
        account::AccountSharedData,
        clock::Epoch,
        pubkey::Pubkey,
        rent::{Rent, RentDue},
        rent_collector::{CollectedInfo, RentCollector},
        transaction::{Result, TransactionError},
        transaction_context::IndexOfAccount,
    },
    solana_svm::rent_manager::{RentState, SVMRentManager},
};

#[cfg_attr(feature = "frozen-abi", derive(AbiExample))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RentManager {
    pub rent_collector: RentCollector,
}

impl RentManager {
    pub fn new(rent_collector: RentCollector) -> Self {
        Self { rent_collector }
    }
}

impl SVMRentManager for RentManager {
    // Override to submit rent state metrics.
    fn check_rent_state_with_account(
        &self,
        pre_rent_state: &RentState,
        post_rent_state: &RentState,
        address: &Pubkey,
        account_state: &AccountSharedData,
        account_index: IndexOfAccount,
    ) -> Result<()> {
        submit_rent_state_metrics(pre_rent_state, post_rent_state);
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

    fn collect_from_existing_account(
        &self,
        address: &Pubkey,
        account: &mut AccountSharedData,
    ) -> CollectedInfo {
        self.rent_collector
            .collect_from_existing_account(address, account)
    }

    fn get_rent(&self) -> &Rent {
        &self.rent_collector.rent
    }

    fn get_rent_due(&self, lamports: u64, data_len: usize, account_rent_epoch: Epoch) -> RentDue {
        self.rent_collector
            .get_rent_due(lamports, data_len, account_rent_epoch)
    }
}

fn submit_rent_state_metrics(pre_rent_state: &RentState, post_rent_state: &RentState) {
    match (pre_rent_state, post_rent_state) {
        (&RentState::Uninitialized, &RentState::RentPaying { .. }) => {
            inc_new_counter_info!("rent_paying_err-new_account", 1);
        }
        (&RentState::RentPaying { .. }, &RentState::RentPaying { .. }) => {
            inc_new_counter_info!("rent_paying_ok-legacy", 1);
        }
        (_, &RentState::RentPaying { .. }) => {
            inc_new_counter_info!("rent_paying_err-other", 1);
        }
        _ => {}
    }
}
