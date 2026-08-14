//! Expected effects on the global program cache.

use {
    crate::{
        consts::NATIVE_BUILTINS,
        entry::{Entry, Environments},
    },
    solana_program_runtime::loaded_programs::ProgramCacheForTxBatch,
    solana_pubkey::Pubkey,
    std::collections::HashMap,
};

/// Panics unless the cache holds exactly `expected` — no more, no less.
///
/// Programs may appear in any order, but the ordering of their second_level
/// (slot versions) is enforced.
pub(crate) fn assert_cache_contents(entries: &[Entry], expected: &[Entry]) {
    assert_eq!(
        slot_versions(entries),
        slot_versions(expected),
        "cache contents mismatch"
    );
}

fn slot_versions(entries: &[Entry]) -> HashMap<Pubkey, Vec<Entry>> {
    let mut versions: HashMap<Pubkey, Vec<Entry>> = HashMap::new();
    for entry in entries {
        versions.entry(entry.id).or_default().push(*entry);
    }
    versions
}

/// Panics unless the batch was served exactly `expected` — no more, no less,
/// except for native builtins.
pub(crate) fn assert_served(
    batch: &ProgramCacheForTxBatch,
    expected: &[Entry],
    environments: &Environments,
) {
    let served: Vec<Entry> = batch
        .get_entries_for_tests()
        .into_iter()
        .filter(|(id, _)| !NATIVE_BUILTINS.contains(id))
        .filter_map(|(id, entry)| Entry::from_program_cache_entry(id, &entry, environments))
        .collect();
    assert_eq!(
        slot_versions(&served),
        slot_versions(expected),
        "served entry mismatch"
    );
}

/// Panics unless each target was deployed at the batch's slot. We know it was
/// deployed if it's an `Unloaded` entry at that slot in this forks' batch cache.
pub(crate) fn assert_deployed(
    batch: &ProgramCacheForTxBatch,
    targets: &[Pubkey],
    environments: &Environments,
) {
    assert_targets(
        batch,
        targets,
        Entry::new_unloaded,
        "deployed entry mismatch",
        environments,
    );
}

/// Panics unless each target was closed at the batch's slot. We know it was
/// closed if it's a `Closed` entry at that slot in this forks' batch cache.
pub(crate) fn assert_closed(
    batch: &ProgramCacheForTxBatch,
    targets: &[Pubkey],
    environments: &Environments,
) {
    assert_targets(
        batch,
        targets,
        Entry::new_closed,
        "closed entry mismatch",
        environments,
    );
}

fn assert_targets(
    batch: &ProgramCacheForTxBatch,
    targets: &[Pubkey],
    entry: fn(Pubkey, u64) -> Entry,
    message: &str,
    environments: &Environments,
) {
    let found: Vec<Option<Entry>> = targets
        .iter()
        .map(|target| {
            batch
                .find_entry(target)
                .and_then(|found| Entry::from_program_cache_entry(*target, &found, environments))
        })
        .collect();
    let expected: Vec<Option<Entry>> = targets
        .iter()
        .map(|target| Some(entry(*target, batch.slot())))
        .collect();
    assert_eq!(found, expected, "{message}");
}
