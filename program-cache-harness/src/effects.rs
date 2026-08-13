//! Expected effects on the global program cache.

use {crate::entry::Entry, solana_pubkey::Pubkey, std::collections::HashMap};

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
