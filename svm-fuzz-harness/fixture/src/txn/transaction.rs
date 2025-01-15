//! Transaction.

use {
    crate::{error::FixtureError, proto::SanitizedTransaction as ProtoSanitizedTransaction},
    solana_sdk::{message::VersionedMessage, transaction::VersionedTransaction},
    solana_signature::Signature,
};

impl TryFrom<ProtoSanitizedTransaction> for VersionedTransaction {
    type Error = FixtureError;

    fn try_from(value: ProtoSanitizedTransaction) -> Result<Self, Self::Error> {
        let message: VersionedMessage = value
            .message
            .ok_or(FixtureError::TransactionMessageMissing)?
            .try_into()?;

        let mut signatures = value
            .signatures
            .into_iter()
            .map(|bytes| Signature::try_from(bytes).map_err(FixtureError::InvalidSignatureBytes))
            .collect::<Result<Vec<Signature>, _>>()?;

        if signatures.is_empty() {
            signatures.push(Signature::default());
        }

        Ok(VersionedTransaction {
            message,
            signatures,
        })
    }
}

impl From<VersionedTransaction> for ProtoSanitizedTransaction {
    fn from(value: VersionedTransaction) -> Self {
        let message_hash = value.message.hash().to_bytes().to_vec();
        let signatures = value
            .signatures
            .iter()
            .map(|sig| sig.as_ref().to_vec())
            .collect();

        ProtoSanitizedTransaction {
            message: Some(value.message.into()),
            message_hash,
            signatures,
        }
    }
}
