//! Initial test environment configuration.

use {crate::entry::Entry, agave_feature_set::FeatureSet, solana_pubkey::Pubkey};

/// Configuration for the initial test environment before anything is executed.
///
/// The test environment starts out with one single fork, beginning at the
/// target slot and rooted `FINALITY_SLOTS` slots back, or at slot 0.
#[derive(Clone, Debug)]
pub struct Genesis {
    /// The programs present at genesis, and what the cache holds for each.
    ///
    /// What the program actually does or looks like is not important for
    /// this harness.
    pub cache_contents: Vec<Entry>,
    /// The current feature set.
    ///
    /// Used in combination with the default compute budget to derive the
    /// program runtime environment to use for all entries in the genesis
    /// cache contents.
    pub feature_set: FeatureSet,
    /// The slot the initial test environment should start at.
    pub slot: u64,
}

impl Genesis {
    /// Every feature enabled, except those in `disabled`.
    pub fn new_with_features_disabled(
        cache_contents: Vec<Entry>,
        slot: u64,
        disabled: &[Pubkey],
    ) -> Self {
        let mut feature_set = FeatureSet::all_enabled();
        for id in disabled {
            feature_set.deactivate(id);
        }
        Self {
            cache_contents,
            feature_set,
            slot,
        }
    }

    /// Every feature enabled.
    pub fn new_with_features_all_enabled(cache_contents: Vec<Entry>, slot: u64) -> Self {
        Self::new_with_features_disabled(cache_contents, slot, &[])
    }
}
