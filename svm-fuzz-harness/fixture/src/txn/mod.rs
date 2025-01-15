//! Solana SVM transaction fixture.

pub mod context;
pub mod message;
pub mod result;
pub mod transaction;

use {
    crate::{
        error::FixtureError, invoke::metadata::FixtureMetadata,
        proto::TxnFixture as ProtoTxnFixture,
    },
    context::TxnContext,
    result::TxnResult,
};

/// A fixture for invoking a transaction against a simulated SVM environment.
#[derive(Clone, Debug, PartialEq)]
pub struct TxnFixture {
    /// The fixture metadata.
    pub metadata: Option<FixtureMetadata>,
    /// The fixture inputs.
    pub input: TxnContext,
    /// The fixture outputs.
    pub output: TxnResult,
}

impl TryFrom<ProtoTxnFixture> for TxnFixture {
    type Error = FixtureError;

    fn try_from(value: ProtoTxnFixture) -> Result<Self, Self::Error> {
        // All blobs should have an input and output.
        Ok(Self {
            metadata: value.metadata.map(Into::into),
            input: value.input.unwrap().try_into()?,
            output: value.output.unwrap().try_into()?,
        })
    }
}

impl From<TxnFixture> for ProtoTxnFixture {
    fn from(value: TxnFixture) -> Self {
        Self {
            metadata: value.metadata.map(Into::into),
            input: Some(value.input.into()),
            output: Some(value.output.into()),
        }
    }
}
