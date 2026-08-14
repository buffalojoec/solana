//! Bank helpers.

use {
    crate::consts::{MINT_LAMPORTS, SLOTS_PER_EPOCH},
    agave_feature_set::FeatureSet,
    solana_epoch_schedule::EpochSchedule,
    solana_keypair::Keypair,
    solana_program_runtime::loaded_programs::ProgramCacheForTxBatch,
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
) -> ProgramCacheForTxBatch {
    let mut batch = None;
    let results = bank
        .process_entry_transactions_and_inspect(transactions, |output| {
            batch = Some(output.program_cache_for_tx_batch.clone());
        })
        .expect("failed to sanitize transactions");
    for (index, result) in results.into_iter().enumerate() {
        assert!(result.is_ok(), "transaction {index} failed: {result:?}");
    }
    let batch = batch.expect("inspect callback never ran");
    assert_eq!(
        batch.slot(),
        bank.slot(),
        "batch cache belongs to another bank"
    );
    batch
}
