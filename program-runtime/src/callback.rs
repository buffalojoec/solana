//! Callbacks required for program runtime operations.

use {
    crate::program_metrics::ProgramStatistics, solana_pubkey::Pubkey,
    solana_svm_type_overrides::sync::Arc,
};

/// Callback used by [`InvokeContext`] to reach into the global
/// [`ProgramCache`] for programs the batch-local cache does not hold.
///
/// [`InvokeContext`]: crate::invoke_context::InvokeContext
/// [`ProgramCache`]: crate::loaded_programs::ProgramCache
pub trait ProgramCacheCallback {
    /// Obtain usage statistics recorded for `program_id`, if found, without
    /// loading or compiling its binary.
    fn get_program_stats(&self, _program_id: &Pubkey) -> Option<Arc<ProgramStatistics>> {
        None
    }
}

/// A [`ProgramCacheCallback`] that never reaches the global
/// [`ProgramCache`](crate::loaded_programs::ProgramCache).
///
/// Only appropriate where the batch-local cache is fully populated up front.
#[cfg(feature = "dev-context-only-utils")]
pub struct NoOpProgramCacheCallback;

#[cfg(feature = "dev-context-only-utils")]
impl ProgramCacheCallback for NoOpProgramCacheCallback {}
