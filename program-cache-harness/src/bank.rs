//! Bank helpers.

use {
    crate::consts::{MINT_LAMPORTS, SLOTS_PER_EPOCH},
    agave_feature_set::FeatureSet,
    solana_epoch_schedule::EpochSchedule,
    solana_keypair::Keypair,
    solana_runtime::{
        bank::Bank,
        genesis_utils::{GenesisConfigInfo, create_genesis_config, deactivate_features},
    },
    solana_transaction::versioned::VersionedTransaction,
};

pub(crate) fn create_genesis_bank(feature_set: &FeatureSet) -> (Bank, Keypair) {
    let GenesisConfigInfo {
        mut genesis_config,
        mint_keypair,
        ..
    } = create_genesis_config(MINT_LAMPORTS);

    deactivate_features(
        &mut genesis_config,
        &feature_set.inactive().iter().copied().collect(),
    );
    genesis_config.epoch_schedule = EpochSchedule::custom(SLOTS_PER_EPOCH, SLOTS_PER_EPOCH, false);

    (Bank::new_for_tests(&genesis_config), mint_keypair)
}

pub(crate) fn process_transactions_and_assert_success(
    bank: &Bank,
    transactions: Vec<VersionedTransaction>,
) {
    for (index, result) in bank
        .process_entry_transactions(transactions)
        .into_iter()
        .enumerate()
    {
        assert!(result.is_ok(), "transaction {index} failed: {result:?}");
    }
}
