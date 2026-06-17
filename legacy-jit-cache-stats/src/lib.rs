//! Per-program usage statistics for the legacy program JIT cache.
//!
//! Extracted into a leaf crate so low-level crates (e.g. `solana-svm-callback`)
//! can reference [`ProgramStatistics`] without depending on `program-runtime`.

use std::sync::atomic::{AtomicU64, Ordering};

/// Number of compilation observations contributing to the
/// [`ProgramStatistics::compilation_time_ema`].
const COMPILATION_EMA_WINDOW_SIZE: u64 = 10;
/// Number of execution observations contributing to the execution EMA stats.
const EXECUTION_EMA_WINDOW_SIZE: u64 = 500;
/// Track exponential moving average in scaled-up units.
///
/// Doing so allows to mitigate error from rounding-towards-zero we get when using integer math.
pub const EMA_SCALE: u64 = 1_000;

#[derive(Debug, Default)]
pub struct ProgramStatistics {
    pub uses: AtomicU64,

    pub compilations: AtomicU64,
    pub total_compilation_time_us: AtomicU64,
    /// Exponential moving average of the compilation time.
    pub compilation_time_ema: AtomicU64,

    pub jit_invocations: AtomicU64,
    pub total_jit_execution_time_us: AtomicU64,
    /// Exponential moving average of the JIT execution time.
    pub jit_execution_time_ema: AtomicU64,

    pub interpreted_invocations: AtomicU64,
    pub total_interpretation_time_us: AtomicU64,
    /// Exponential moving average of the interpreted execution time.
    pub interpretation_time_ema: AtomicU64,
}

impl ProgramStatistics {
    fn observe_ema<const WINDOW_SIZE: u64>(counter: &AtomicU64, duration_us: u64) {
        let duration_ema = duration_us.saturating_mul(EMA_SCALE);
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |ema| {
                // Exponential moving average iteratively is computed as $ema' = alpha *
                // observation + (1 - alpha) * ema$. This works great for floating point, but we
                // want integers. For purposes of convenience we also want to really think in terms
                // of simple moving average window sizes as that is easier to reason about.
                //
                // Exponential moving average and simple moving average of window N has a rough
                // equivalence of `alpha ≈ 2 / (N + 1)`. Slotting this into our original iterative
                // formula:
                //
                // $$ ema' = 2 / (N+1) * observation + (1 - 2/(N+1)) * ema $$
                //
                // we get
                //
                // $$ ema' = (2*observation)/(N+1) + (N+1-2)*ema/(N+1) $$
                let (numer, denom) = const { (2, 1 + WINDOW_SIZE) };
                Some(if ema == 0 {
                    duration_ema
                } else {
                    let weighted_observation = duration_ema.saturating_mul(numer);
                    let previous_observations = ema.saturating_mul(denom.saturating_sub(numer));
                    weighted_observation
                        .saturating_add(previous_observations)
                        .checked_div(denom)
                        .expect("unreachable: denom is >= 1")
                })
            })
            .expect("unreachable: closure always returns a Some");
    }

    /// Record information about JIT compilation.
    pub fn jit_compiled(&self, duration_us: u64) {
        let ord = Ordering::Relaxed;
        self.compilations.fetch_add(1, ord);
        self.total_compilation_time_us.fetch_add(duration_us, ord);
        Self::observe_ema::<COMPILATION_EMA_WINDOW_SIZE>(&self.compilation_time_ema, duration_us);
    }

    /// Record information about JIT-compiled program having been executed.
    pub fn jit_executed(&self, duration_us: u64) {
        let ord = Ordering::Relaxed;
        self.jit_invocations.fetch_add(1, ord);
        self.total_jit_execution_time_us.fetch_add(duration_us, ord);
        Self::observe_ema::<EXECUTION_EMA_WINDOW_SIZE>(&self.jit_execution_time_ema, duration_us);
    }

    /// Record information about program executed with the interpreter.
    pub fn interpreter_executed(&self, duration_us: u64) {
        let ord = Ordering::Relaxed;
        self.interpreted_invocations.fetch_add(1, ord);
        self.total_interpretation_time_us
            .fetch_add(duration_us, ord);
        Self::observe_ema::<EXECUTION_EMA_WINDOW_SIZE>(&self.interpretation_time_ema, duration_us);
    }

    pub fn merge_from(&self, other: &ProgramStatistics) {
        let ord = Ordering::Relaxed;
        self.uses.fetch_add(other.uses.load(ord), ord);
        let other_compilations = other.compilations.load(ord);
        let this_compilations = self.compilations.fetch_add(other_compilations, ord);
        self.total_compilation_time_us
            .fetch_add(other.total_compilation_time_us.load(ord), ord);
        let other_jit_invocations = other.jit_invocations.load(ord);
        let this_jit_invocations = self.jit_invocations.fetch_add(other_jit_invocations, ord);
        self.total_jit_execution_time_us
            .fetch_add(other.total_jit_execution_time_us.load(ord), ord);
        let other_interpretations = other.interpreted_invocations.load(ord);
        let this_interpretations = self
            .interpreted_invocations
            .fetch_add(other_interpretations, ord);
        self.total_interpretation_time_us
            .fetch_add(other.total_interpretation_time_us.load(ord), ord);
        if let Some(comp_ema) =
            combined_ema::<COMPILATION_EMA_WINDOW_SIZE, COMPILATION_EMA_WINDOW_SIZE>(
                &self.compilation_time_ema,
                &other.compilation_time_ema,
                this_compilations,
                other_compilations,
            )
        {
            self.compilation_time_ema.store(comp_ema, ord);
        }
        if let Some(exec_ema) = combined_ema::<EXECUTION_EMA_WINDOW_SIZE, EXECUTION_EMA_WINDOW_SIZE>(
            &self.jit_execution_time_ema,
            &other.jit_execution_time_ema,
            this_jit_invocations,
            other_jit_invocations,
        ) {
            self.jit_execution_time_ema.store(exec_ema, ord);
        }
        if let Some(interp_ema) = combined_ema::<EXECUTION_EMA_WINDOW_SIZE, EXECUTION_EMA_WINDOW_SIZE>(
            &self.interpretation_time_ema,
            &other.interpretation_time_ema,
            this_interpretations,
            other_interpretations,
        ) {
            self.interpretation_time_ema.store(interp_ema, ord);
        }
    }
}

/// Merge two independent EMA trackers, weighting each by its observation count.
fn combined_ema<const WINDOW1: u64, const WINDOW2: u64>(
    into_ema: &AtomicU64,
    from_ema: &AtomicU64,
    into_observations: u64,
    from_observations: u64,
) -> Option<u64> {
    // This is a mild non-sense, but there is no good mathematically rigorous way to merge
    // two independent EMA trackers AFAICT and this is the best I (nagisa) could come up
    // with…
    let other_ema_val = from_ema.load(Ordering::Relaxed);
    let other_ema_weight = std::cmp::max(WINDOW1, from_observations);
    let this_ema_val = into_ema.load(Ordering::Relaxed);
    let this_ema_weight = std::cmp::max(WINDOW2, into_observations);
    other_ema_val
        .wrapping_mul(other_ema_weight)
        .wrapping_add(this_ema_val.wrapping_mul(this_ema_weight))
        .checked_div(other_ema_weight.wrapping_add(this_ema_weight))
}
