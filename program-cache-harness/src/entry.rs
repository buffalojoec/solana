//! What kind of entry the cache holds.

use {
    solana_clock::Slot,
    solana_program_runtime::program_cache_entry::{ProgramCacheEntryOwner, ProgramCacheEntryType},
    std::fmt,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub program: u8,
    pub deployment_slot: Slot,
    pub owner: ProgramCacheEntryOwner,
    pub kind: EntryKind,
    pub env: Option<u8>,
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "program={} slot={} owner={:?} kind={} env={:?}",
            self.program,
            self.deployment_slot,
            self.owner,
            self.kind.name(),
            self.env,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntryKind {
    Loaded,
    Unloaded,
    Closed,
    FailedVerification,
    DelayVisibility,
}

impl EntryKind {
    pub fn of(program: &ProgramCacheEntryType) -> Self {
        match program {
            ProgramCacheEntryType::Loaded(_) => Self::Loaded,
            ProgramCacheEntryType::Unloaded(_) => Self::Unloaded,
            ProgramCacheEntryType::Closed => Self::Closed,
            ProgramCacheEntryType::FailedVerification(_) => Self::FailedVerification,
            ProgramCacheEntryType::DelayVisibility => Self::DelayVisibility,
            ProgramCacheEntryType::Builtin(_) => unreachable!("the cache holds no builtins"),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Unloaded => "unloaded",
            Self::Closed => "closed",
            Self::FailedVerification => "failed-verification",
            Self::DelayVisibility => "delay-visibility",
        }
    }
}
