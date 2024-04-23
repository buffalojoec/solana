//! The interface of the Solana SVM.
//!
//! This crate outlines the Solana SVM specification in Rust.
//! Developers who wish to build custom SVM implementations in Rust can use
//! this interface to build a specification-compliant SVM, complete with an
//! integration testing harness to ensure maximum compatibility with the
//! network.

pub mod load_results;
pub mod results;

use {
    load_results::TransactionLoadResult, results::TransactionExecutionResult,
    solana_sdk::transaction::SanitizedTransaction,
};

/// The main interface for the Solana SVM.
///
/// At its core, the SVM is a transaction batch processor.
/// Given a batch of Solana transactions, any SVM implementation must be able
/// to process the batch and return a result.
/// This functionality is fully extentable, allowing developers to add custom
/// functionality, as well as configure their own monitoring tooling, such as
/// timings, metrics, and logging.
pub trait TransactionBatchProcessorInterface<C, T>
where
    C: TransactionBatchProcessorContext,
    T: TransactionBatchProcessorOutput,
{
    /// The entrypoint to the SVM.
    /// Load and execute a batch of sanitized transactions.
    fn load_and_execute_sanitized_transactions(
        &self,
        sanitized_txs: &[SanitizedTransaction],
        context: C,
    ) -> T;
}

/// An extendable context argument to the SVM's
/// `load_and_execute_sanitized_transactions`, for custom functionality.
pub trait TransactionBatchProcessorContext {}

/// The main return type of the SVM interface.
///
/// The output of the `load_and_execute_sanitized_transactions` method.
pub trait TransactionBatchProcessorOutput {
    /// The output of any SVM implementation should retain a list of loaded
    /// transactions.
    fn loaded_transactions(&self) -> Vec<TransactionLoadResult>;
    /// The output of any SVM implementation should retain a list of execution
    /// results.
    fn execution_results(&self) -> Vec<TransactionExecutionResult>;
}
