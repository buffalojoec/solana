use solana_sdk::{
    account::AccountSharedData,
    pubkey::Pubkey,
    rent::Rent,
    transaction::Result,
    transaction_context::{IndexOfAccount, TransactionContext},
};

mod rent_state;

/// Rent state trait.
///
/// Implementors are responsible for evaluating rent state of accounts.
pub trait SVMRentState {
    /// Type representing the rent state of an account.
    type RentState;

    /// Check rent state transition for an account in a transaction.
    ///
    /// This method has a default implementation that calls into
    /// `check_rent_state_with_account`.
    fn check_rent_state(
        pre_rent_state: Option<&Self::RentState>,
        post_rent_state: Option<&Self::RentState>,
        transaction_context: &TransactionContext,
        index: IndexOfAccount,
    ) -> Result<()> {
        if let Some((pre_rent_state, post_rent_state)) = pre_rent_state.zip(post_rent_state) {
            let expect_msg =
                "account must exist at TransactionContext index if rent-states are Some";
            Self::check_rent_state_with_account(
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
    fn check_rent_state_with_account(
        pre_rent_state: &Self::RentState,
        post_rent_state: &Self::RentState,
        address: &Pubkey,
        account_state: &AccountSharedData,
        account_index: IndexOfAccount,
    ) -> Result<()>;

    /// Determine the rent state of an account.
    fn get_account_rent_state(account: &AccountSharedData, rent: &Rent) -> Self::RentState;

    /// Check whether a transition from the pre_rent_state to the
    /// post_rent_state is valid.
    fn transition_allowed(
        pre_rent_state: &Self::RentState,
        post_rent_state: &Self::RentState,
    ) -> bool;
}
