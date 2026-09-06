//! The properties a run must not violate.

use {
    crate::extraction::Extraction,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryType, std::sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Fails the run.
    Critical,
    /// Collected and reported, doesn't fail.
    Warning,
}

#[derive(Clone, Debug)]
pub struct Violation {
    pub invariant: &'static str,
    pub severity: Severity,
    pub detail: String,
}

pub trait Invariant {
    fn name(&self) -> &'static str;
    fn severity(&self) -> Severity;
    fn check(&self, extraction: &Extraction) -> Option<Violation>;

    fn violation(&self, detail: String) -> Violation {
        Violation {
            invariant: self.name(),
            severity: self.severity(),
            detail,
        }
    }
}

/// The entry extracted must be an absolute match.
///
/// Asserts extraction did not hand back an entry the caller did not ask for.
/// In other words, the deployment slot, owner and environment must all match.
///
/// An entry which carries no environment at all - a tombstone, a builtin - is
/// held to the first two only.
pub struct WrongEntry;

impl Invariant for WrongEntry {
    fn name(&self) -> &'static str {
        "wrong-entry"
    }

    fn severity(&self) -> Severity {
        Severity::Critical
    }

    fn check(&self, extraction: &Extraction) -> Option<Violation> {
        let returned = extraction.returned.as_ref()?;
        let asked = extraction.account_state;

        if returned.deployment_slot != asked.deployment_slot {
            return Some(self.violation(format!(
                "asked for slot {} and got slot {}",
                asked.deployment_slot, returned.deployment_slot
            )));
        }
        if returned.account_owner != asked.owner {
            return Some(self.violation(format!(
                "asked for owner {:?} and got {:?}",
                asked.owner, returned.account_owner
            )));
        }
        match returned.program.get_environment() {
            Some(env) if *env != extraction.batch_env => Some(self.violation(format!(
                "got an entry built for another environment at slot {}",
                returned.deployment_slot
            ))),
            _ => None,
        }
    }
}

/// The entry extracted must actually be from the fork's lineage.
///
/// While `WrongEntry` above asserts the entry is a direct match, this
/// invariant asserts any member from another fork didn't get pulled by
/// mistake, even if it is in fact a match by fields alone.
pub struct EntryIsOnTheCallersFork;

impl Invariant for EntryIsOnTheCallersFork {
    fn name(&self) -> &'static str {
        "entry-is-on-the-callers-fork"
    }

    fn severity(&self) -> Severity {
        Severity::Critical
    }

    fn check(&self, extraction: &Extraction) -> Option<Violation> {
        let returned = extraction.returned.as_ref()?;
        if extraction.ancestry.contains(&returned.deployment_slot) {
            return None;
        }
        Some(self.violation(format!(
            "served an entry deployed at slot {}, which is not on the fork of the batch at slot \
             {} (ancestry {:?})",
            returned.deployment_slot, extraction.batch_slot, extraction.ancestry
        )))
    }
}

/// The entry extracted must be one that we placed there.
///
/// The harness keeps track of every entry it put in at each slot in a ledger.
/// This invariant allows us to compare by pointer to what's recorded in the
/// ledger, meaning it can tell two entries apart even when every field agrees.
///
/// A scenario like this could arise if an orphan were deployed in the same
/// slot as an entry on the canonical fork, with the same environment and
/// owner, but different bytecode (ie. equivocation). The two invariants above
/// would both pass it.
pub struct EntryMatchesLedger;

impl Invariant for EntryMatchesLedger {
    fn name(&self) -> &'static str {
        "entry-matches-ledger"
    }

    fn severity(&self) -> Severity {
        Severity::Critical
    }

    fn check(&self, extraction: &Extraction) -> Option<Violation> {
        let returned = extraction.returned.as_ref()?;
        if matches!(returned.program, ProgramCacheEntryType::DelayVisibility) {
            return None;
        }
        if extraction.candidates.is_empty() {
            return None;
        }
        if !extraction
            .candidates
            .iter()
            .any(|candidate| Arc::ptr_eq(returned, candidate))
        {
            return Some(self.violation(format!(
                "got an entry which was never put in at slot {}",
                extraction.account_state.deployment_slot
            )));
        }
        None
    }
}

/// A load must not be asked for twice at the same deployment slot.
///
/// This invariant catches the "reload cycle", wherein a program is reloaded
/// and inserted into the cache via cooperative loading task, but then it gets
/// signaled for reload again on the next `extract`.
///
/// This scenario would mean we have a bug - likely in solana-svm's
/// `replenish_program_cache` - which is causing the transaction processor to
/// miss the proper entry and keep reloading it.
///
/// We saw an edge case like this was possible if a *closed* program was not
/// properly excluded from the search list and we kept trying to extract it
/// at a particular *deployment slot* (closed programs should have no
/// deployment slot in their account state anymore).
pub struct NoRepeatedReload;

impl Invariant for NoRepeatedReload {
    fn name(&self) -> &'static str {
        "no-repeated-reload"
    }

    fn severity(&self) -> Severity {
        Severity::Critical
    }

    fn check(&self, extraction: &Extraction) -> Option<Violation> {
        if !(extraction.started_load && extraction.already_loaded_here) {
            return None;
        }
        Some(self.violation(format!(
            "asked to load slot {} again, having already loaded it - the batch at slot {} cannot \
             see what the last load produced",
            extraction.account_state.deployment_slot, extraction.batch_slot
        )))
    }
}

pub const fn all() -> [&'static dyn Invariant; 4] {
    [
        &WrongEntry,
        &EntryIsOnTheCallersFork,
        &EntryMatchesLedger,
        &NoRepeatedReload,
    ]
}
