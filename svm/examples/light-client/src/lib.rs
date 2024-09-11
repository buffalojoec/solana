//! SVM Light Client example.
//!
//! This example demonstrates a layer 2 blockchain - Blitz - with an SVM
//! execution layer. A full node executes transactions using the SVM API, packs
//! them into blocks, and tracks transactions, transaction receipts, and
//! transaction STF traces in Merkle trees.
//!
//! Everything in the `blitz` module is suggested to be what a full node would
//! run. The full node stores ledger and tree data internally, and exposes
//! access to proofs created from its tree store through a public API. This can
//! be considered analogous to the full node's RPC API.
//!
//! The `light_client` module is suggested to be a completely separate client,
//! which can run on very minimal hardware. The light client tracks only block
//! headers, and will query full nodes for proofs, which can be used to
//! validate transactions against the roots stored in each block header.
//!
//! Note: This example assumes the light client has some sort of sampling
//! method for verifying the proper fork slection and each block header's
//! confirmation.
//!
//! To see the full example, check out the tests.

pub mod blitz;
pub mod light_client;
