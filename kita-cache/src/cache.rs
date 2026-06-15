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
    crate::entry::Entry,
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
type EventCache<C> = RwLock<Arc<HashMap<Pubkey, EventCacheEntry<C>>>>;
type Builtins<C> = RwLock<Arc<HashMap<Pubkey, Arc<BuiltinProgram<C>>>>>;

pub struct KitaCache<C: ContextObject> {
    /// A pointer to the global cache of JIT-compiled programs. Shared by every
    /// bank; only written during pruning.
    root_cache: RootCache<C>,
    /// Fork-local cache of compilation events. If a program was JIT-compiled in
    /// a given bank, its newly compiled executable is stored here until a
    /// rooting event graduates it into the root cache. Shared copy-on-write with
    /// the parent: inheriting is a refcount bump, and the map is only cloned
    /// when a new compilation event is actually inserted.
    event_cache: EventCache<C>,
    /// The built-in programs active on this fork. Deterministic and free of any
    /// ELF to compile, so they are simply registered rather than loaded. Shared
    /// copy-on-write with the parent, like the event cache, since built-ins only
    /// change on feature transitions.
    builtins: Builtins<C>,
}

impl<C: ContextObject> Default for KitaCache<C> {
    fn default() -> Self {
        Self {
            root_cache: Arc::default(),
            event_cache: RwLock::default(),
            builtins: RwLock::default(),
        }
    }
}

impl<C: ContextObject> KitaCache<C> {
    /// Derive a child cache from its parent bank's cache. The global root cache
    /// is shared and the fork-local event cache is shared copy-on-write (cloned
    /// only when the child records a compilation event).
    pub fn new_from_parent(parent: &Self) -> Self {
        Self {
            root_cache: Arc::clone(&parent.root_cache),
            event_cache: RwLock::new(Arc::clone(&parent.event_cache.read().unwrap())),
            builtins: RwLock::new(Arc::clone(&parent.builtins.read().unwrap())),
        }
    }

    /// Register a built-in program. There is no ELF to compile, so the prepared
    /// [`BuiltinProgram`] is stored directly.
    pub fn add_builtin(&self, program_id: &Pubkey, builtin: BuiltinProgram<C>) {
        let mut guard = self.builtins.write().unwrap();
        Arc::make_mut(&mut guard).insert(*program_id, Arc::new(builtin));
    }

    /// Find a program in the cache. Built-ins resolve directly; otherwise check
    /// for a more recent fork-local version in the event cache, then fall back
    /// to the root cache.
    ///
    /// Returns an owned [`Entry`] - cheap, since the program is shared behind an
    /// [`Arc`] - because the tiers are guarded by locks.
    pub fn find(&self, program_id: &Pubkey) -> Option<Entry<C>> {
        if let Some(builtin) = self.builtins.read().unwrap().get(program_id) {
            return Some(Entry::Builtin(Arc::clone(builtin)));
        }
        if let Some(event) = self.event_cache.read().unwrap().get(program_id) {
            return Some(event.entry.clone());
        }
        self.root_cache.read().unwrap().get(program_id).cloned()
    }

    /// Insert a new entry into the event cache.
    ///
    /// Compilation events arrive in increasing slot order along a fork, so
    /// an entry is only overwritten by one from an equal or newer slot; a stale
    /// event can never clobber a fresher compilation.
    pub fn insert(&self, program_id: &Pubkey, entry: Entry<C>, slot: Slot) {
        let mut guard = self.event_cache.write().unwrap();
        // Clone away from the parent only now that we actually have something to
        // record on this fork.
        let event_cache = Arc::make_mut(&mut guard);
        match event_cache.get_mut(program_id) {
            Some(event) if slot >= event.slot => {
                event.entry = entry;
                event.slot = slot;
            }
            Some(_) => {}
            None => {
                event_cache.insert(*program_id, EventCacheEntry { slot, entry });
            }
        }
    }
}
