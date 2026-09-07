//! Fork-based test and fuzz harness for the global program cache.

mod elf;
mod entry;
mod extraction;
mod invariants;
mod ledger;
mod report;
mod rules;
mod runner;
mod scenario;

pub use {
    entry::{Entry, EntryKind},
    extraction::{EbppRecord, ExtractionRecord},
    report::Report,
    runner::{
        Runner, run, run_twice,
        v1::{Harness as V1, forks::Forks},
        v2::Harness as V2,
    },
    scenario::{ForkTree, LoadResult, Op, Scenario, Seed, tree},
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner as Owner,
};
