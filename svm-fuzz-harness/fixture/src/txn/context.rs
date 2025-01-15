//! Transaction context (inputs).

use {
    crate::{
        context::{epoch_context::EpochContext, slot_context::SlotContext},
        error::FixtureError,
        proto::TxnContext as ProtoTxnContext,
    },
    solana_account::Account,
    solana_hash::Hash,
    solana_pubkey::Pubkey,
    solana_sdk::transaction::VersionedTransaction,
};

#[derive(Clone, Debug, PartialEq)]
pub struct TxnContext {
    /// Input accounts with state.
    pub accounts: Vec<(Pubkey, Account)>,
    /// Blockhash queue.
    pub blockhash_queue: Vec<Hash>,
    /// Transaction.
    pub transaction: VersionedTransaction,
    /// Slot context.
    pub slot_context: SlotContext,
    /// Epoch context.
    pub epoch_context: EpochContext,
}

impl TryFrom<ProtoTxnContext> for TxnContext {
    type Error = FixtureError;

    fn try_from(value: ProtoTxnContext) -> Result<Self, Self::Error> {
        let accounts: Vec<(Pubkey, Account)> = value
            .account_shared_data
            .into_iter()
            .map(|a| a.try_into())
            .collect::<Result<_, _>>()?;

        let blockhash_queue: Vec<Hash> = if value.blockhash_queue.is_empty() {
            vec![Hash::from([0u8; 32])]
        } else {
            value
                .blockhash_queue
                .into_iter()
                .map(|bytes| {
                    TryInto::<[u8; 32]>::try_into(bytes.as_ref())
                        .map(Hash::from)
                        .map_err(|_| FixtureError::InvalidHashBytes(bytes))
                })
                .collect::<Result<Vec<Hash>, _>>()?
        };

        let transaction: VersionedTransaction = value
            .tx
            .ok_or(FixtureError::TransactionMissing)?
            .try_into()?;

        Ok(Self {
            accounts,
            blockhash_queue,
            transaction,
            slot_context: value
                .slot_ctx
                .map(Into::into)
                .unwrap_or(SlotContext { slot: 10 }),
            epoch_context: value.epoch_ctx.map(Into::into).unwrap_or_default(),
        })
    }
}

impl From<TxnContext> for ProtoTxnContext {
    fn from(value: TxnContext) -> Self {
        let account_shared_data = value.accounts.into_iter().map(Into::into).collect();

        let blockhash_queue = value
            .blockhash_queue
            .iter()
            .map(|h| h.to_bytes().to_vec())
            .collect();

        Self {
            tx: Some(value.transaction.into()),
            account_shared_data,
            blockhash_queue,
            slot_ctx: Some(value.slot_context.into()),
            epoch_ctx: Some(value.epoch_context.into()),
        }
    }
}
