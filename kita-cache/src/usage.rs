//! Program usage tracking for root-cache retention.
//!
//! The root cache can only hold so many compiled programs, so when the runtime
//! roots a bank we keep the most popular programs and evict the rest. This
//! module measures "popular" by counting how often each program is found for
//! execution, then ranks programs by a decayed score at pruning time.
//!
//! # This is a heuristic, not consensus
//!
//! Usage statistics never affect execution results. A program that is mis-ranked
//! and evicted is simply recompiled on the fly the next time it is needed. So
//! the tracker is deliberately approximate and lock-light: it counts every use
//! on every fork - including forks that are later abandoned - without any
//! fork-local bookkeeping. The score is a global, network-wide demand signal.
//!
//! Counting across forks is not a flaw but a reasonable model of demand: a
//! program hammered on a fork that later dies still reflects real transactions
//! that real submitters sent to it. The score decays geometrically each rooting
//! (see [`UsageTracker::decay_and_retain`]), so the telemetry is self-pruning -
//! stale popularity ages out on its own, and the score map cannot grow without
//! bound.
//!
//! # Threat model
//!
//! Because usage is counted across all forks, a malicious leader can spin up its
//! own fork and spam transactions against programs it wants kept warm,
//! inflating their scores so they out-rank programs that genuinely deserve a
//! compiled slot. This is a real but strictly bounded attack:
//!
//! - It cannot affect execution results or consensus - usage only decides which
//!   programs stay JIT-compiled, and a wrong choice merely forces a
//!   recompile-on-demand. It cannot fork or halt the network.
//! - Its only effect is a transient performance skew: the attacker's favored
//!   programs stay warm at the expense of others that should be warm, possibly
//!   slowing execution of those others for a while.
//! - It is self-correcting: once the spam stops, the inflated scores decay out
//!   and retention returns to honest demand.
//!
//! In short, the worst case is a temporary, self-healing performance
//! degradation - undesirable, but not network-threatening.

use {
    solana_pubkey::Pubkey,
    std::{
        collections::HashMap,
        sync::{
            RwLock,
            atomic::{AtomicU64, Ordering},
        },
    },
};

/// Number of programs the root cache retains after each rooting event.
///
// TODO: This is a placeholder depth. It should be tuned against real workloads
// (and possibly made a function of available memory) rather than a fixed
// constant.
pub(crate) const RETENTION_DEPTH: usize = 256;

/// Tracks per-program usage to drive root-cache retention.
///
/// Shared by every bank on the fork graph (held behind an [`Arc`](std::sync::Arc)
/// alongside the root cache), so all forks contribute to one global score.
#[derive(Default)]
pub struct UsageTracker {
    /// Per-program decayed usage score. A shared read lock guards the map so
    /// that concurrent uses of *distinct* programs never contend; only the first
    /// sighting of a program (and pruning) takes the write lock.
    scores: RwLock<HashMap<Pubkey, AtomicU64>>,
}

impl UsageTracker {
    /// Record one use of a program.
    pub fn record(&self, program_id: &Pubkey) {
        // Fast path: an already-tracked program needs only a shared lock and a
        // relaxed atomic bump.
        if let Some(score) = self.scores.read().unwrap().get(program_id) {
            score.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // First sighting: take the write lock to insert the counter.
        // `or_insert_with` tolerates a racing insert from another thread.
        self.scores
            .write()
            .unwrap()
            .entry(*program_id)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Retain the most-used [`RETENTION_DEPTH`] programs in `root_cache` and
    /// decay all scores for the next window.
    ///
    /// Called when a bank is rooted. Programs are ranked by their current score
    /// (ties broken by address for determinism) and the cache is trimmed to the
    /// retention depth. Every score is then halved, and any that reaches zero is
    /// forgotten, keeping the score map bounded and recency-weighted.
    pub fn decay_and_retain<V>(&self, root_cache: &mut HashMap<Pubkey, V>) {
        let mut scores = self.scores.write().unwrap();
        if root_cache.len() > RETENTION_DEPTH {
            let mut ranked: Vec<(Pubkey, u64)> = root_cache
                .keys()
                .map(|program_id| {
                    let score = scores
                        .get(program_id)
                        .map(|score| score.load(Ordering::Relaxed))
                        .unwrap_or(0);
                    (*program_id, score)
                })
                .collect();
            // Most-used first; ties broken by address so eviction is
            // deterministic.
            ranked.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            for (program_id, _) in ranked.into_iter().skip(RETENTION_DEPTH) {
                root_cache.remove(&program_id);
            }
        }
        // Halve every score for the next window and forget the ones that reach
        // zero, so the map stays bounded and weighted toward recent demand.
        scores.retain(|_, score| {
            let value = score.get_mut();
            *value >>= 1;
            *value > 0
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A pubkey whose byte ordering matches the numeric ordering of `n`.
    fn key(n: u16) -> Pubkey {
        let mut bytes = [0u8; 32];
        bytes[0..2].copy_from_slice(&n.to_be_bytes());
        Pubkey::new_from_array(bytes)
    }

    fn score_of(tracker: &UsageTracker, program_id: &Pubkey) -> u64 {
        tracker
            .scores
            .read()
            .unwrap()
            .get(program_id)
            .map(|score| score.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    #[test]
    fn record_accumulates() {
        let tracker = UsageTracker::default();
        assert_eq!(score_of(&tracker, &key(1)), 0);
        tracker.record(&key(1));
        tracker.record(&key(1));
        tracker.record(&key(2));
        assert_eq!(score_of(&tracker, &key(1)), 2);
        assert_eq!(score_of(&tracker, &key(2)), 1);
    }

    #[test]
    fn decay_halves_scores_and_forgets_cold_programs() {
        let tracker = UsageTracker::default();
        for _ in 0..4 {
            tracker.record(&key(1)); // score 4
        }
        tracker.record(&key(2)); // score 1
        let mut under_depth = HashMap::<Pubkey, ()>::new(); // no eviction

        tracker.decay_and_retain(&mut under_depth);
        assert_eq!(score_of(&tracker, &key(1)), 2); // 4 >> 1
        assert_eq!(score_of(&tracker, &key(2)), 0); // 1 >> 1, forgotten
        assert!(!tracker.scores.read().unwrap().contains_key(&key(2)));

        tracker.decay_and_retain(&mut under_depth);
        assert_eq!(score_of(&tracker, &key(1)), 1); // 2 >> 1
    }

    #[test]
    fn retain_keeps_most_used_when_over_depth() {
        let tracker = UsageTracker::default();
        let total = RETENTION_DEPTH.saturating_add(4);
        let mut root_cache: HashMap<Pubkey, ()> = (0..total).map(|n| (key(n as u16), ())).collect();
        // Use the 4 highest-address programs; leave the rest at zero.
        for n in total.saturating_sub(4)..total {
            tracker.record(&key(n as u16));
        }

        tracker.decay_and_retain(&mut root_cache);

        assert_eq!(root_cache.len(), RETENTION_DEPTH);
        // The used programs survive despite having the largest addresses...
        for n in total.saturating_sub(4)..total {
            assert!(root_cache.contains_key(&key(n as u16)));
        }
        // ...while unused programs just outside the depth are evicted.
        assert!(!root_cache.contains_key(&key(total.saturating_sub(5) as u16)));
    }
}
