//! Proto <--> Fixture errors.

use {
    prost::DecodeError,
    solana_sdk::{message::SanitizeMessageError, sanitize::SanitizeError},
    thiserror::Error,
};

#[derive(Debug, Error, PartialEq)]
pub enum FixtureError {
    #[error("Decode error")]
    DecodeError(#[from] DecodeError),
    #[error("Transaction sanitization error")]
    SanitizeError(#[from] SanitizeError),
    #[error("Transaction message sanitization error")]
    SanitizeMessageError(#[from] SanitizeMessageError),
    #[error("Invalid public key bytes")]
    InvalidPubkeyBytes(Vec<u8>),
    #[error("Invalid hash bytes")]
    InvalidHashBytes(Vec<u8>),
    #[error("Invalid signature bytes")]
    InvalidSignatureBytes(Vec<u8>),
    #[error("An account is missing for instruction account index {0}")]
    AccountMissingForInstrAccount(usize),
    #[error("Transaction message missing")]
    TransactionMessageMissing,
    #[error("Transaction missing")]
    TransactionMissing,
}
