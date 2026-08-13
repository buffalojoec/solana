//! Expected effects on the global program cache.

use {crate::entry::Entry, solana_pubkey::Pubkey};

/// What the global program cache should hold at a point in the timeline.
#[derive(Clone, Debug)]
pub enum Expect {
    /// The cache holds exactly this entry.
    Present(Entry),
    /// The cache holds nothing for this program.
    Absent(Pubkey),
}

impl Expect {
    /// Panics if `entries` does not satisfy this expectation.
    pub(crate) fn assert(&self, entries: &[Entry]) {
        match self {
            Self::Present(expected) => {
                let found = entries.iter().find(|entry| entry.id == expected.id);
                assert_eq!(found, Some(expected), "cache entry mismatch");
            }
            Self::Absent(id) => {
                let found = entries.iter().find(|entry| entry.id == *id);
                assert_eq!(found, None, "expected no cache entry for {id}");
            }
        }
    }
}
