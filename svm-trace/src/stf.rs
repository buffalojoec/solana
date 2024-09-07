//! SVM STF trace.

use {
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        feature_set::FeatureSet,
        fee::FeeStructure,
        keccak::Hasher,
        pubkey::Pubkey,
        rent::Rent,
    },
    solana_svm_rent_collector::svm_rent_collector::SVMRentCollector,
    solana_svm_transaction::svm_transaction::SVMTransaction,
};

pub struct STFState<'a> {
    pub accounts: &'a [(Pubkey, AccountSharedData)],
}

pub struct STFEnvironment<'a> {
    pub feature_set: &'a FeatureSet,
    pub fee_structure: Option<&'a FeeStructure>,
    pub lamports_per_signature: &'a u64,
    pub rent_collector: Option<&'a dyn SVMRentCollector>,
}

pub struct STFDirective<'a, T: SVMTransaction> {
    pub environment: &'a STFEnvironment<'a>,
    pub transaction: &'a T,
}

pub enum STFTrace<'a, T: SVMTransaction> {
    State(&'a STFState<'a>),
    Directive(&'a STFDirective<'a, T>),
    NewState(&'a STFState<'a>),
}

pub fn hash_account(hasher: &mut Hasher, pubkey: &Pubkey, account: &AccountSharedData) {
    hasher.hashv(&[
        pubkey.as_ref(),
        account.data(),
        &account.lamports().to_le_bytes(),
        account.owner().as_ref(),
        &[if account.executable() { 1 } else { 0 }],
        &account.rent_epoch().to_le_bytes(),
    ]);
}

fn hash_feature_set(hasher: &mut Hasher, feature_set: &FeatureSet) {
    // TODO: This is slow...
    let mut active = feature_set.active.iter().collect::<Vec<_>>();
    active.sort_by_key(|(k, _)| *k);

    let mut inactive = feature_set.inactive.iter().collect::<Vec<_>>();
    inactive.sort();

    active
        .iter()
        .map(|(k, _)| k)
        .chain(inactive.iter())
        .for_each(|feature| {
            hasher.hash(feature.as_ref());
        });
}

fn hash_fee_structure(hasher: &mut Hasher, fee_structure: &FeeStructure) {
    hasher.hash(&fee_structure.lamports_per_signature.to_le_bytes());
    hasher.hash(&fee_structure.lamports_per_write_lock.to_le_bytes());
    // `compute_fee_bins` skipped for now.
}

fn hash_rent(hasher: &mut Hasher, rent: &Rent) {
    hasher.hash(&rent.lamports_per_byte_year.to_le_bytes());
    hasher.hash(&rent.exemption_threshold.to_le_bytes());
    hasher.hash(&rent.burn_percent.to_le_bytes());
}

fn hash_rent_collector(hasher: &mut Hasher, rent_collector: &dyn SVMRentCollector) {
    hash_rent(hasher, rent_collector.get_rent());
}

pub fn hash_environment(hasher: &mut Hasher, environment: &STFEnvironment) {
    hash_feature_set(hasher, environment.feature_set);
    if let Some(fee_structure) = environment.fee_structure {
        hash_fee_structure(hasher, fee_structure);
    }
    hasher.hash(&environment.lamports_per_signature.to_le_bytes());
    if let Some(rent_collector) = environment.rent_collector {
        hash_rent_collector(hasher, rent_collector);
    }
}

pub fn hash_transaction(hasher: &mut Hasher, transaction: &impl SVMTransaction) {
    hasher.hash(transaction.signature().as_ref());
}

pub fn hash_trace<T: SVMTransaction>(hasher: &mut Hasher, trace: &STFTrace<'_, T>) {
    match trace {
        STFTrace::State(state) => {
            for (pubkey, account) in state.accounts {
                hash_account(hasher, pubkey, account);
            }
        }
        STFTrace::Directive(directive) => {
            hash_environment(hasher, directive.environment);
            hash_transaction(hasher, directive.transaction);
        }
        STFTrace::NewState(state) => {
            for (pubkey, account) in state.accounts {
                hash_account(hasher, pubkey, account);
            }
        }
    }
}
