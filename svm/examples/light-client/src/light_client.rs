//! Blitz light client implementation.
//!
//! Many fields on the `Blitz` struct are private, so imagine this light client
//! actually exists on a completely different machine than some arbitrary full
//! node running Blitz.
//!
//! The `BlitzLightClient` client API takes a reference to a `&Blitz`, however,
//! in practice, this would be a network connection wherein the light client
//! makes a request to one or more full nodes (like a sampled subset) for a
//! proof.
//!
//! In many scenarios - such as when a light client asks an RPC for a
//! transaction's receipt or pre/post state - the light client can immediately
//! validate the returned data against the roots stored in the block header.

use {
    crate::blitz::Blitz,
    solana_sdk::{account::AccountSharedData, clock::Slot, keccak::Hasher, pubkey::Pubkey},
    solana_svm_trace::{
        receipt::SVMTransactionReceipt,
        stf::{STFDirective, STFEnvironment, STFState, STFTrace},
    },
    solana_svm_transaction::svm_transaction::SVMTransaction,
};

pub struct BlitzLightClient<'a> {
    blitz: &'a Blitz,
    hasher: &'a mut Hasher,
}

impl<'a> BlitzLightClient<'a> {
    pub fn new(blitz: &'a Blitz, hasher: &'a mut Hasher) -> Self {
        Self { blitz, hasher }
    }

    /// Prove a transaction's inclusion in a block.
    ///
    /// Fetches a transaction inclusion proof from a full node and evaluates it
    /// against the provided transaction data.
    pub fn prove_transaction_inclusion(
        &mut self,
        slot: &Slot,
        transaction: &impl SVMTransaction,
    ) -> bool {
        let candidate = {
            crate::blitz::hash_functions::hash_transaction(self.hasher, transaction);
            let raw_hash = self.hasher.result_reset();
            self.hasher.hashv(&[&[0], raw_hash.as_ref()]);
            self.hasher.result_reset()
        };
        self.blitz
            .get_transaction_inclusion_proof(slot, &candidate)
            .map(|proof| proof.verify(candidate))
            .unwrap_or(false)
    }

    /// Prove a transaction's receipt.
    ///
    /// Fetches a transaction receipt proof from a full node and evaluates it
    /// against the provided transaction data.
    pub fn prove_transaction_receipt(
        &mut self,
        slot: &Slot,
        transaction: &impl SVMTransaction,
        receipt: &SVMTransactionReceipt,
    ) -> bool {
        let candidate = {
            crate::blitz::hash_functions::hash_receipt(self.hasher, transaction, receipt);
            let raw_hash = self.hasher.result_reset();
            self.hasher.hashv(&[&[0], raw_hash.as_ref()]);
            self.hasher.result_reset()
        };
        self.blitz
            .get_transaction_receipt_proof(slot, &candidate)
            .map(|proof| proof.verify(candidate))
            .unwrap_or(false)
    }

    /// Prove a transaction's state transition function.
    ///
    /// Fetches a transaction STF proof from a full node and evaluates it
    /// against the provided transaction data.
    pub fn prove_transaction_stf<T: SVMTransaction>(
        &mut self,
        slot: &Slot,
        transaction: &T,
        environment: &STFEnvironment,
        pre_account_state: &[(Pubkey, AccountSharedData)],
        post_account_state: &[(Pubkey, AccountSharedData)],
    ) -> bool {
        let candidate = {
            crate::blitz::hash_functions::hash_trace(
                self.hasher,
                &STFTrace::State::<T>(&STFState {
                    accounts: pre_account_state,
                }),
            );
            crate::blitz::hash_functions::hash_trace(
                self.hasher,
                &STFTrace::Directive::<T>(&STFDirective {
                    environment,
                    transaction,
                }),
            );
            crate::blitz::hash_functions::hash_trace(
                self.hasher,
                &STFTrace::NewState::<T>(&STFState {
                    accounts: post_account_state,
                }),
            );
            let raw_hash = self.hasher.result_reset();
            self.hasher.hashv(&[&[0], raw_hash.as_ref()]);
            self.hasher.result_reset()
        };
        self.blitz
            .get_transaction_stf_trace_proof(slot, &candidate)
            .map(|proof| proof.verify(candidate))
            .unwrap_or(false)
    }
}
