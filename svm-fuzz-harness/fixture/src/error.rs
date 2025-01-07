//! Proto <--> Fixture errors.

#[cfg(feature = "serde")]
use solana_svm_fuzz_harness_fixture_fs::error::FsError;
use {
    prost::DecodeError,
    solana_sdk::{message::SanitizeMessageError, sanitize::SanitizeError},
    thiserror::Error,
};

#[derive(Debug, Error)]
pub enum FixtureError {
    #[cfg(feature = "serde")]
    #[error("FS error")]
    FsError(#[from] FsError),
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
