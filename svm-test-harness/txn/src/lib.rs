//! Solana SVM test harness for transaction execution.
//!
//! This crate provides an API for Agave's SVM transaction pipeline in order to
//! execute transactions directly without the runtime.

mod harness;

pub use {
    harness::execute_txn,
    solana_compute_budget::compute_budget::ComputeBudget,
    solana_svm::transaction_processor::ExecutionRecordingConfig,
    solana_svm_test_harness_fixture as fixture,
    solana_svm_test_harness_instr::{file, keyed_account},
};

#[cfg(feature = "fuzz")]
pub mod fuzz;
