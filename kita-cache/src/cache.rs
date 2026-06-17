//! Kita-Cache: Program JIT Cache v2
//!
//! The Kita Cache is a two-tiered caching system for JIT-compiled executable
//! programs. It's designed to be fork-local and to gather most of its
//! information from its parent bank.
//!
//! The first tier is a pointer to the global, intra-fork "root cache", which
//! houses all JIT-compiled executables for the most popular programs according
//! to its tracked usage statistics. Each time the runtime's fork graph is
//! pruned, this global cache is updated. It cannot be written to except for
//! during pruning.
//!
//! The second tier is a fork-local "event cache", which houses JIT-compiled
//! executables that materialize as a result of compilation events on the
//! designated bank's fork. These are most commonly deployments or loads.
//!
//! A [`KitaCache`] is stored on each bank. A child bank inherits its parent's
//! fork-local state via [`KitaCache::new_from_parent`], so the bank lineage
//! itself models the fork graph - dead forks reclaim their caches when their
//! banks are dropped.

use {
    crate::{entry::Entry, usage::UsageTracker},
    solana_clock::Slot,
    solana_pubkey::Pubkey,
    solana_sbpf::{program::BuiltinProgram, vm::ContextObject},
    std::{
        collections::HashMap,
        sync::{Arc, RwLock},
    },
};

/// A fork-local cache entry: a compiled program tagged with the slot of the
/// most recent compilation event that produced it.
struct EventCacheEntry<C: ContextObject> {
    /// The slot that this entry was last updated with a new compilation event.
    slot: Slot,
    /// The JIT-compiled program.
    entry: Entry<C>,
}

impl<C: ContextObject> Clone for EventCacheEntry<C> {
    fn clone(&self) -> Self {
        Self {
            slot: self.slot,
            entry: self.entry.clone(),
        }
    }
}

type RootCache<C> = Arc<RwLock<HashMap<Pubkey, Entry<C>>>>;
type ParentEventCache<C> = Arc<HashMap<Pubkey, EventCacheEntry<C>>>;
type CurrentEventCache<C> = RwLock<HashMap<Pubkey, EventCacheEntry<C>>>;
type Builtins<C> = RwLock<Arc<HashMap<Pubkey, Arc<BuiltinProgram<C>>>>>;

pub struct KitaCache<C: ContextObject> {
    /// A pointer to the global cache of JIT-compiled programs. Shared by every
    /// bank; only written during pruning.
    root_cache: RootCache<C>,
    /// Read-only snapshot of the fork-local compilation events inherited from
    /// the parent bank. Shared by reference: inheriting is a refcount bump, and
    /// the parent's writable cache is folded into a fresh snapshot only when it
    /// actually recorded a compilation event (see [`Self::new_from_parent`]).
    parent_event_cache: ParentEventCache<C>,
    /// Writable cache of compilation events recorded by *this* bank. A write
    /// only ever touches this small map, never the larger inherited snapshot, so
    /// inserts are scoped to the entry being written rather than cloning the
    /// whole event cache. Folded into the child's `parent_event_cache` when a
    /// child bank is derived.
    current_event_cache: CurrentEventCache<C>,
    /// The built-in programs active on this fork. Deterministic and free of any
    /// ELF to compile, so they are simply registered rather than loaded. Shared
    /// copy-on-write with the parent, since built-ins only change on feature
    /// transitions.
    builtins: Builtins<C>,
    /// Per-program usage statistics, shared by every bank like the root cache.
    /// Records demand on `find` and drives root-cache retention at pruning.
    usage_tracker: Arc<UsageTracker>,
}

impl<C: ContextObject> Default for KitaCache<C> {
    fn default() -> Self {
        Self {
            root_cache: Arc::default(),
            parent_event_cache: Arc::default(),
            current_event_cache: RwLock::default(),
            builtins: RwLock::default(),
            usage_tracker: Arc::default(),
        }
    }
}

impl<C: ContextObject> KitaCache<C> {
    /// Derive a child cache from its parent bank's cache. The global root cache
    /// is shared by reference. The parent's writable cache is sealed into the
    /// snapshot the child inherits, so the child starts with a fresh read-only
    /// `parent_event_cache` and an empty writable cache.
    ///
    /// The inherited snapshot is cloned only when the parent actually recorded a
    /// compilation event; otherwise the child shares the parent's snapshot by
    /// reference.
    pub fn new_from_parent(parent: &Self) -> Self {
        let parent_event_cache = {
            let parent_current = parent.current_event_cache.read().unwrap();
            if parent_current.is_empty() {
                Arc::clone(&parent.parent_event_cache)
            } else {
                let mut sealed = (*parent.parent_event_cache).clone();
                // The parent's events are newer than anything in its inherited
                // snapshot, so they always win.
                for (program_id, event) in parent_current.iter() {
                    sealed.insert(*program_id, event.clone());
                }
                Arc::new(sealed)
            }
        };
        Self {
            root_cache: Arc::clone(&parent.root_cache),
            parent_event_cache,
            current_event_cache: RwLock::default(),
            builtins: RwLock::new(Arc::clone(&parent.builtins.read().unwrap())),
            usage_tracker: Arc::clone(&parent.usage_tracker),
        }
    }

    /// Register a built-in program. There is no ELF to compile, so the prepared
    /// [`BuiltinProgram`] is stored directly.
    pub fn add_builtin(&self, program_id: &Pubkey, builtin: BuiltinProgram<C>) {
        let mut guard = self.builtins.write().unwrap();
        Arc::make_mut(&mut guard).insert(*program_id, Arc::new(builtin));
    }

    /// Find a program in the cache. Built-ins resolve directly; otherwise check
    /// this bank's writable cache, then the inherited snapshot, then fall back
    /// to the root cache.
    ///
    /// Returns an owned [`Entry`] - cheap, since the program is shared behind an
    /// [`Arc`] - because the tiers are guarded by locks.
    pub fn find(&self, program_id: &Pubkey) -> Option<Entry<C>> {
        // Built-ins are never compiled or evicted, so they are exempt from usage
        // tracking and resolve before it.
        if let Some(builtin) = self.builtins.read().unwrap().get(program_id) {
            return Some(Entry::Builtin(Arc::clone(builtin)));
        }

        let mut entry = None;
        if let Some(event) = self.current_event_cache.read().unwrap().get(program_id) {
            entry = Some(event.entry.clone());
        } else if let Some(event) = self.parent_event_cache.get(program_id) {
            entry = Some(event.entry.clone());
        } else if let Some(program) = self.root_cache.read().unwrap().get(program_id) {
            entry = Some(program.clone());
        }

        if entry.is_some() {
            // Count demand for this program to inform root-cache retention.
            self.usage_tracker.record(program_id);
        }
        entry
    }

    /// Insert a new entry into this bank's writable event cache.
    ///
    /// Compilation events arrive in increasing slot order along a fork, so
    /// an entry is only overwritten by one from an equal or newer slot; a stale
    /// event can never clobber a fresher compilation.
    pub fn insert(&self, program_id: &Pubkey, entry: Entry<C>, slot: Slot) {
        let mut current = self.current_event_cache.write().unwrap();
        match current.get_mut(program_id) {
            Some(event) if slot >= event.slot => {
                event.entry = entry;
                event.slot = slot;
            }
            Some(_) => {}
            None => {
                current.insert(*program_id, EventCacheEntry { slot, entry });
            }
        }
    }

    /// Prune the cache in response to this cache's bank being rooted.
    ///
    /// The inherited snapshot holds the programs that became rooted along the
    /// fork, so they graduate into the shared root cache, shedding their slot
    /// metadata. This bank's own writable cache is intentionally ignored: those
    /// events have already been promoted into descendant banks' inherited
    /// snapshots by [`Self::new_from_parent`], and will graduate when the root
    /// advances past them.
    ///
    /// After folding, the root cache is trimmed to retain only the most-used
    /// programs (see [`UsageTracker::decay_and_retain`]) so it stays bounded as
    /// the chain advances.
    pub fn prune(&self) {
        let mut root_cache = self.root_cache.write().unwrap();
        for (program_id, event) in self.parent_event_cache.iter() {
            root_cache.insert(*program_id, event.entry.clone());
        }
        self.usage_tracker.decay_and_retain(&mut root_cache);
    }
}

#[cfg(feature = "dev-context-only-utils")]
impl<C: ContextObject> KitaCache<C> {
    /// The program ids held in the global root cache.
    pub fn root_cache_program_ids(&self) -> Vec<Pubkey> {
        self.root_cache.read().unwrap().keys().copied().collect()
    }

    /// Whether the global root cache holds `program_id`.
    pub fn root_cache_contains(&self, program_id: &Pubkey) -> bool {
        self.root_cache.read().unwrap().contains_key(program_id)
    }

    /// The program ids in the snapshot inherited from ancestor banks.
    pub fn parent_event_cache_program_ids(&self) -> Vec<Pubkey> {
        self.parent_event_cache.keys().copied().collect()
    }

    /// Whether the inherited snapshot holds `program_id`.
    pub fn parent_event_cache_contains(&self, program_id: &Pubkey) -> bool {
        self.parent_event_cache.contains_key(program_id)
    }

    /// The program ids recorded by this bank's own writable event cache.
    pub fn current_event_cache_program_ids(&self) -> Vec<Pubkey> {
        self.current_event_cache
            .read()
            .unwrap()
            .keys()
            .copied()
            .collect()
    }

    /// Whether this bank's writable event cache holds `program_id`.
    pub fn current_event_cache_contains(&self, program_id: &Pubkey) -> bool {
        self.current_event_cache
            .read()
            .unwrap()
            .contains_key(program_id)
    }

    /// The program's current usage score (drives root-cache retention).
    pub fn usage_score(&self, program_id: &Pubkey) -> u64 {
        self.usage_tracker.score(program_id)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{entry::Reason, usage::RETENTION_DEPTH},
    };

    // A do-nothing context object; the cache never executes the programs it
    // holds, so the methods are unreachable in these tests.
    struct TestCtx;
    impl ContextObject for TestCtx {
        fn consume(&mut self, _amount: u64) {}
        fn get_remaining(&self) -> u64 {
            0
        }
        fn active_mapping_ptr(
            &mut self,
        ) -> std::ptr::NonNull<solana_sbpf::memory_region::MemoryMapping> {
            unimplemented!("test context object is never executed")
        }
    }

    type Cache = KitaCache<TestCtx>;

    fn tomb(reason: Reason) -> Entry<TestCtx> {
        Entry::Tombstone(reason)
    }

    // Distinguish entries by their tombstone reason, which has no `PartialEq`.
    fn reason_of(entry: &Entry<TestCtx>) -> Reason {
        match entry {
            Entry::Tombstone(reason) => reason.clone(),
            _ => panic!("expected a tombstone entry"),
        }
    }

    // A pubkey whose byte ordering matches the numeric ordering of `n`, so tests
    // can reason about address-ordered retention.
    fn key(n: u16) -> Pubkey {
        let mut bytes = [0u8; 32];
        bytes[0..2].copy_from_slice(&n.to_be_bytes());
        Pubkey::new_from_array(bytes)
    }

    #[test]
    fn write_is_scoped_and_shadows_inherited_snapshot() {
        let parent = Cache::default();
        parent.insert(&key(1), tomb(Reason::UnknownSyscall), 1);
        parent.insert(&key(2), tomb(Reason::UnknownConfig), 1);

        // The child inherits the parent's events via the sealed snapshot.
        let child = Cache::new_from_parent(&parent);
        assert!(matches!(
            reason_of(&child.find(&key(1)).unwrap()),
            Reason::UnknownSyscall
        ));
        assert!(matches!(
            reason_of(&child.find(&key(2)).unwrap()),
            Reason::UnknownConfig
        ));

        // A write on the child shadows the inherited entry...
        child.insert(&key(2), tomb(Reason::UnsupportedSyscall), 2);
        assert!(matches!(
            reason_of(&child.find(&key(2)).unwrap()),
            Reason::UnsupportedSyscall
        ));
        // ...without disturbing untouched inherited entries...
        assert!(matches!(
            reason_of(&child.find(&key(1)).unwrap()),
            Reason::UnknownSyscall
        ));
        // ...and without mutating the parent (the write was scoped to the
        // child's own cache, not the shared snapshot).
        assert!(matches!(
            reason_of(&parent.find(&key(2)).unwrap()),
            Reason::UnknownConfig
        ));
        assert!(
            child
                .current_event_cache
                .read()
                .unwrap()
                .contains_key(&key(2))
        );
        assert!(
            !child
                .current_event_cache
                .read()
                .unwrap()
                .contains_key(&key(1))
        );
    }

    #[test]
    fn child_shares_snapshot_by_reference_when_parent_made_no_writes() {
        let parent = Cache::default();
        parent.insert(&key(1), tomb(Reason::Other), 1);

        // The parent wrote, so its child seals a fresh snapshot...
        let child = Cache::new_from_parent(&parent);
        // ...but a child that made no writes is inherited by reference.
        let grandchild = Cache::new_from_parent(&child);
        assert!(Arc::ptr_eq(
            &child.parent_event_cache,
            &grandchild.parent_event_cache
        ));
    }

    #[test]
    fn prune_folds_inherited_snapshot_and_ignores_current() {
        let parent = Cache::default();
        parent.insert(&key(1), tomb(Reason::Other), 1); // an ancestor's deploy
        let child = Cache::new_from_parent(&parent); // key 1 is now in the snapshot
        child.insert(&key(2), tomb(Reason::Other), 2); // the child's own deploy

        child.prune();

        let root = child.root_cache.read().unwrap();
        assert!(root.contains_key(&key(1))); // inherited snapshot graduated
        assert!(!root.contains_key(&key(2))); // current cache ignored
    }

    #[test]
    fn prune_retains_most_used_programs_over_depth() {
        let total = RETENTION_DEPTH.saturating_add(4);
        let parent = Cache::default();
        for n in 0..total {
            parent.insert(&key(n as u16), tomb(Reason::Other), 1);
        }
        let child = Cache::new_from_parent(&parent);
        // Heavily use the 4 highest-address programs - the ones a naive
        // address-order eviction would drop first.
        for n in total.saturating_sub(4)..total {
            for _ in 0..5 {
                let _ = child.find(&key(n as u16));
            }
        }

        child.prune();

        let root = child.root_cache.read().unwrap();
        assert_eq!(root.len(), RETENTION_DEPTH);
        // The heavily-used programs survive despite their large addresses...
        for n in total.saturating_sub(4)..total {
            assert!(root.contains_key(&key(n as u16)));
        }
        // ...while unused programs just inside the address range are evicted.
        assert!(!root.contains_key(&key(total.saturating_sub(5) as u16)));
    }
}
