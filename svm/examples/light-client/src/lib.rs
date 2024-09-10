//! SVM Light Client.
//!
//! This example demonstrates a light client built specifically for a
//! particular SVM L2. It showcases how an SVM L2 can merely enable SVM traces
//! for its execution layer, and as a result quite simply bootstrap a light
//! client for its ledger.
//!
//! The L2 itself is dubbed "Blitz", to avoid using the moniker "L2"
//! everywhere.
//!
//! To see the full example, check out the tests.

pub mod blitz;
pub mod light_client;
