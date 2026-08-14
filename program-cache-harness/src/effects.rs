//! Expected effects on the global program cache.

use {
    crate::entry::Entry, solana_program_runtime::loaded_programs::ProgramCacheForTxBatch,
    solana_pubkey::Pubkey, std::collections::HashMap,
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

/// Panics unless the batch was served `expected` for `targets`, in order.
pub(crate) fn assert_served(
    batch: &ProgramCacheForTxBatch,
    targets: &[Pubkey],
    expected: &[Entry],
) {
    assert_entries(batch, targets, expected, "served entry mismatch");
}

/// Panics unless each target was deployed at the batch's slot. We know it was
/// deployed if it's an `Unloaded` entry at that slot in this forks' batch cache.
pub(crate) fn assert_deployed(batch: &ProgramCacheForTxBatch, targets: &[Pubkey]) {
    let expected: Vec<Entry> = targets
        .iter()
        .map(|target| Entry::new_unloaded(*target, batch.slot()))
        .collect();
    assert_entries(batch, targets, &expected, "deployed entry mismatch");
}

fn assert_entries(
    batch: &ProgramCacheForTxBatch,
    targets: &[Pubkey],
    expected: &[Entry],
    message: &str,
) {
    let found: Vec<Option<Entry>> = targets
        .iter()
        .map(|target| {
            batch
                .find_entry(target)
                .and_then(|entry| Entry::from_program_cache_entry(*target, &entry))
        })
        .collect();
    let expected: Vec<Option<Entry>> = expected.iter().copied().map(Some).collect();
    assert_eq!(found, expected, "{message}");
}
