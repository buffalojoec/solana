pub(crate) use solana_legacy_jit_cache_stats::EMA_SCALE;
pub use solana_legacy_jit_cache_stats::ProgramStatistics;
#[cfg(feature = "metrics")]
use solana_svm_timings::ExecuteDetailsTimings;
use {
    crate::loaded_programs::ForkGraph,
    log::{debug, log_enabled, trace},
    solana_pubkey::Pubkey,
    std::{
        collections::HashMap,
        sync::atomic::{AtomicU64, Ordering},
    },
};

/// Global cache statistics for [ProgramCache].
#[derive(Debug, Default)]
pub struct ProgramCacheStats {
    /// a program was already in the cache
    pub hits: AtomicU64,
    /// a program was not found and loaded instead
    pub misses: AtomicU64,
    /// a compiled executable was unloaded
    pub evictions: HashMap<Pubkey, u64>,
    /// an unloaded program was loaded again (opposite of eviction)
    pub reloads: AtomicU64,
    /// a program was loaded or un/re/deployed
    pub insertions: AtomicU64,
    /// a program was loaded but can not be extracted on its own fork anymore
    pub lost_insertions: AtomicU64,
    /// a program which was already in the cache was reloaded by mistake
    pub replacements: AtomicU64,
    /// a program was only used once before being unloaded
    pub one_hit_wonders: AtomicU64,
    /// a program became unreachable in the fork graph because of rerooting
    pub prunes_orphan: AtomicU64,
    /// a program got pruned because it was not recompiled for the next epoch
    pub prunes_environment: AtomicU64,
    /// a program had no entries because all slot versions got pruned
    pub empty_entries: AtomicU64,
    /// water level of loaded entries currently cached
    pub water_level: AtomicU64,
}

impl ProgramCacheStats {
    pub fn reset(&mut self) {
        *self = ProgramCacheStats::default();
    }
    pub fn log(&self) {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let evictions: u64 = self.evictions.values().sum();
        let reloads = self.reloads.load(Ordering::Relaxed);
        let insertions = self.insertions.load(Ordering::Relaxed);
        let lost_insertions = self.lost_insertions.load(Ordering::Relaxed);
        let replacements = self.replacements.load(Ordering::Relaxed);
        let one_hit_wonders = self.one_hit_wonders.load(Ordering::Relaxed);
        let prunes_orphan = self.prunes_orphan.load(Ordering::Relaxed);
        let prunes_environment = self.prunes_environment.load(Ordering::Relaxed);
        let empty_entries = self.empty_entries.load(Ordering::Relaxed);
        let water_level = self.water_level.load(Ordering::Relaxed);
        debug!(
            "Loaded Programs Cache Stats -- Hits: {hits}, Misses: {misses}, Evictions: \
             {evictions}, Reloads: {reloads}, Insertions: {insertions}, Lost-Insertions: \
             {lost_insertions}, Replacements: {replacements}, One-Hit-Wonders: {one_hit_wonders}, \
             Prunes-Orphan: {prunes_orphan}, Prunes-Environment: {prunes_environment}, Empty: \
             {empty_entries}, Water-Level: {water_level}"
        );

        if log_enabled!(log::Level::Trace) && !self.evictions.is_empty() {
            let mut evictions = self.evictions.iter().collect::<Vec<_>>();
            evictions.sort_by_key(|e| e.1);
            let evictions = evictions
                .into_iter()
                .rev()
                .map(|(program_id, evictions)| {
                    format!("  {:<44}  {}", program_id.to_string(), evictions)
                })
                .collect::<Vec<_>>();
            let evictions = evictions.join("\n");
            trace!(
                "Eviction Details:\n  {:<44}  {}\n{}",
                "Program", "Count", evictions
            );
        }
    }
}

#[cfg(feature = "metrics")]
/// Time measurements for loading a single [ProgramCacheEntry].
#[derive(Debug, Default)]
pub struct LoadProgramMetrics {
    /// Program address, but as text
    pub program_id: String,
    /// Microseconds it took to `create_program_runtime_environment`
    pub register_syscalls_us: u64,
    /// Microseconds it took to `Executable::<InvokeContext>::load`
    pub load_elf_us: u64,
    /// Microseconds it took to `executable.verify::<RequisiteVerifier>`
    pub verify_code_us: u64,
    /// Microseconds it took to `executable.jit_compile`
    pub jit_compile_us: u64,
}

#[cfg(feature = "metrics")]
impl LoadProgramMetrics {
    pub fn submit_datapoint(&self, timings: &mut ExecuteDetailsTimings) {
        timings.create_executor_register_syscalls_us += self.register_syscalls_us;
        timings.create_executor_load_elf_us += self.load_elf_us;
        timings.create_executor_verify_code_us += self.verify_code_us;
        timings.create_executor_jit_compile_us += self.jit_compile_us;
    }
}

impl<FG: ForkGraph> crate::loaded_programs::ProgramCache<FG> {
    /// Log per-entry statistics for each entry in the global cache.
    #[cfg(feature = "dev-context-only-utils")]
    pub fn output_entry_stats(&self) {
        use {crate::program_cache_entry::ProgramCacheEntryType, std::fmt::Write};
        // The entry stats can become very verbose after some runtime. Rather than dumping them
        // to the log, we'd rather maintain a continuously updated file instead...
        static ENTRY_STAT_PATH: std::sync::LazyLock<Option<std::ffi::OsString>> =
            std::sync::LazyLock::new(|| std::env::var_os("AGAVE_PROGRAM_CACHE_ENTRY_STATS_PATH"));
        let Some(stat_path) = &*ENTRY_STAT_PATH else {
            log::trace!("Set AGAVE_PROGRAM_CACHE_ENTRY_STATS_PATH to write per-entry stats");
            return;
        };
        let mut output = String::new();
        let entries = self.get_flattened_entries_for_tests();
        for (addr, entry) in entries {
            let entry_ty = match &entry.program {
                ProgramCacheEntryType::FailedVerification(_) => "FailedVerification",
                ProgramCacheEntryType::Closed => "Closed",
                ProgramCacheEntryType::DelayVisibility => "DelayVisibility",
                ProgramCacheEntryType::Unloaded(_) => "Unloaded",
                ProgramCacheEntryType::Builtin(_) => "Builtin",
                #[cfg(not(all(not(target_os = "windows"), target_arch = "x86_64")))]
                ProgramCacheEntryType::Loaded(_) => "Loaded",
                #[cfg(all(not(target_os = "windows"), target_arch = "x86_64"))]
                ProgramCacheEntryType::Loaded(executable) => {
                    if executable.get_compiled_program().is_some() {
                        "JitCompiled"
                    } else {
                        "Loaded"
                    }
                }
            };
            let stats = &entry.stats;
            let uses = stats.uses.load(Ordering::Relaxed);
            let compiles = stats.compilations.load(Ordering::Relaxed);
            let comptime = stats.total_compilation_time_us.load(Ordering::Relaxed);
            let comptime_ema = stats.compilation_time_ema.load(Ordering::Relaxed) / EMA_SCALE;
            let invokes = stats.jit_invocations.load(Ordering::Relaxed);
            let jittime = stats.total_jit_execution_time_us.load(Ordering::Relaxed);
            let jittime_ema = stats.jit_execution_time_ema.load(Ordering::Relaxed) / EMA_SCALE;
            let interps = stats.interpreted_invocations.load(Ordering::Relaxed);
            let interptime = stats.total_interpretation_time_us.load(Ordering::Relaxed);
            let interpema = stats.interpretation_time_ema.load(Ordering::Relaxed) / EMA_SCALE;
            let _ = writeln!(
                &mut output,
                "{addr},{entry_ty},{uses},{compiles},{comptime},{comptime_ema},{invokes},\
                 {jittime},{jittime_ema},{interps},{interptime},{interpema}"
            );
        }
        if let Err(e) = std::fs::write(stat_path, output) {
            log::info!("Writing entry stats to {stat_path:?} failed: {e:?}");
        } else {
            log::debug!("Entry stats written to {stat_path:?}");
        }
    }
}
