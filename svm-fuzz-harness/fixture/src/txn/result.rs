//! Transaction result (output).

use {
    crate::{
        error::FixtureError,
        proto::{
            FeeDetails as ProtoFeeDetails, RentDebits as ProtoRentDebits,
            ResultingState as ProtoResultingState, TxnResult as ProtoTxnResult,
        },
    },
    solana_account::Account,
    solana_pubkey::Pubkey,
    solana_sdk::fee::FeeDetails,
};

impl TryFrom<ProtoRentDebits> for (Pubkey, u64) {
    type Error = FixtureError;

    fn try_from(value: ProtoRentDebits) -> Result<Self, Self::Error> {
        let pubkey = Pubkey::try_from(value.pubkey).map_err(FixtureError::InvalidPubkeyBytes)?;
        Ok((pubkey, value.rent_collected as u64))
    }
}

impl From<(Pubkey, u64)> for ProtoRentDebits {
    fn from(value: (Pubkey, u64)) -> Self {
        ProtoRentDebits {
            pubkey: value.0.to_bytes().to_vec(),
            rent_collected: value.1 as i64,
        }
    }
}

impl From<ProtoFeeDetails> for FeeDetails {
    fn from(value: ProtoFeeDetails) -> Self {
        FeeDetails::new(
            value.transaction_fee,
            value.prioritization_fee,
            /* remove_rounding_in_fee_calculation */ false,
        )
    }
}

impl From<FeeDetails> for ProtoFeeDetails {
    fn from(value: FeeDetails) -> Self {
        ProtoFeeDetails {
            transaction_fee: value.transaction_fee(),
            prioritization_fee: value.prioritization_fee(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResultingState {
    pub account_states: Vec<(Pubkey, Account)>,
    pub rent_debits: Vec<(Pubkey, u64)>,
    pub transaction_rent: u64,
}

impl TryFrom<ProtoResultingState> for ResultingState {
    type Error = FixtureError;

    fn try_from(value: ProtoResultingState) -> Result<Self, Self::Error> {
        let account_states = value
            .acct_states
            .into_iter()
            .map(|acct| acct.try_into())
            .collect::<Result<Vec<_>, _>>()?;
        let rent_debits = value
            .rent_debits
            .into_iter()
            .map(|r| r.try_into())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ResultingState {
            account_states,
            rent_debits,
            transaction_rent: value.transaction_rent,
        })
    }
}

impl From<ResultingState> for ProtoResultingState {
    fn from(value: ResultingState) -> Self {
        ProtoResultingState {
            acct_states: value.account_states.into_iter().map(Into::into).collect(),
            rent_debits: value.rent_debits.into_iter().map(Into::into).collect(),
            transaction_rent: value.transaction_rent,
        }
    }
}

/// The execution results for a transaction
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TxnResult {
    /// Whether this transaction was executed.
    pub executed: bool,
    /// Whether there was a sanitization error.
    pub sanitization_error: bool,
    /// The state of each account after the transaction.
    pub resulting_state: Option<ResultingState>,
    /// Rent.
    pub rent: u64,
    /// If an executed transaction has no error.
    pub is_ok: bool,
    /// The transaction status (error code).
    pub status: u32, // TODO ?
    /// The instruction error, if any.
    pub instruction_error: u32, // TODO ?
    /// The instruction error index, if any.
    pub instruction_error_index: u32, // TODO ?
    /// Custom error, if any.
    pub custom_error: u32,
    /// The return data from this transaction, if any.
    pub return_data: Vec<u8>,
    /// Number of executed compute units.
    pub executed_units: u64,
    /// The collected fees in this transaction.
    pub fee_details: Option<FeeDetails>,
}

impl TryFrom<ProtoTxnResult> for TxnResult {
    type Error = FixtureError;

    fn try_from(value: ProtoTxnResult) -> Result<Self, Self::Error> {
        Ok(TxnResult {
            executed: value.executed,
            sanitization_error: value.sanitization_error,
            resulting_state: value.resulting_state.and_then(|res| res.try_into().ok()),
            rent: value.rent,
            is_ok: value.is_ok,
            status: value.status,
            instruction_error: value.instruction_error,
            instruction_error_index: value.instruction_error_index,
            custom_error: value.custom_error,
            return_data: value.return_data,
            executed_units: value.executed_units,
            fee_details: value.fee_details.map(Into::into),
        })
    }
}

impl From<TxnResult> for ProtoTxnResult {
    fn from(value: TxnResult) -> Self {
        ProtoTxnResult {
            executed: value.executed,
            sanitization_error: value.sanitization_error,
            resulting_state: value.resulting_state.map(Into::into),
            rent: value.rent,
            is_ok: value.is_ok,
            status: value.status,
            instruction_error: value.instruction_error,
            instruction_error_index: value.instruction_error_index,
            custom_error: value.custom_error,
            return_data: value.return_data,
            executed_units: value.executed_units,
            fee_details: value.fee_details.map(Into::into),
        }
    }
}
