use solana_compute_budget::compute_budget::ComputeBudget;

/// Encapsulates flags that can be used to tweak the runtime behavior.
#[derive(Debug, Default, Clone)]
pub struct RuntimeConfig {
    pub compute_budget: Option<ComputeBudget>,
    pub log_messages_bytes_limit: Option<usize>,
    pub transaction_account_lock_limit: Option<usize>,
    /// Disable the legacy global program JIT cache in favor of the KitaCache.
    pub use_kita_cache: bool,
}
