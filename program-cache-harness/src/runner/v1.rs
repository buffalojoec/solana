//! # V1: Synthetic Runner
//!
//! This runner's implementation is what we call "synthetic", meaning it models
//! the production configuration of the program cache, but doesn't actually
//! invoke or operate on prod specifically.
//!
//! In this context, *configuration* simply means "how the validator uses the
//! program cache's API". This includes:
//!
//! - Selecting program accounts to extract in the transaction processing
//!   pipeline.
//! - Reloading non-cached or unloaded programs via cooperative loading.
//! - The actual parse, load, and verify process of a reload.
//! - Pruning the cache's contents as slots root.
//! - Reloading programs ahead of an epoch boundary where a feature is known
//!   to be activating.
//!
//! All of these elements are essentially mocked out or simulated by the
//! harness.
//!
//! The primary reason for such an approach is throughput: we can generate
//! significantly more executions per second for a lightweight harness like
//! this one, as opposed to one which uses a real `BankForks` and actual
//! `Bank` nodes in its fork graphs.
//!
//! As a result, we can achieve saturated fuzz coverage much quicker, but we
//! are effectively clamping that coverage solely to `solana-program-runtime`
//! modules pertaining to the program cache.
//!
//! The V2 version of this harness runs against actual production code.

pub mod forks;

use {
    self::forks::{Forks, Graph},
    crate::{
        elf::NOOP_OK,
        entry::{Entry, EntryKind},
        extraction::{Extraction, ExtractionRecord},
        invariants::{self, Violation},
        ledger::{AccountState, Ledger},
        report::Report,
        rules,
        scenario::{ForkTree, LoadResult, NUM_ENVIRONMENTS, NUM_PROGRAMS, Op, Scenario},
    },
    solana_clock::Slot,
    solana_program_runtime::{
        loaded_programs::{
            ProgramCache, ProgramCacheForTxBatch, ProgramRuntimeEnvironment, ProgramToLoad,
        },
        program_cache_entry::{ProgramCacheEntry, ProgramCacheEntryOwner, ProgramCacheEntryType},
        solana_sbpf::{elf::Executable, program::BuiltinProgram},
    },
    solana_pubkey::Pubkey,
    std::{
        collections::{BTreeSet, HashMap},
        sync::{Arc, LazyLock},
    },
};

pub struct Harness {
    /// The fork tree from the provided scenario. Read-only.
    tree: ForkTree,
    /// Account state independent ledger, tracked as we step through ops.
    ledger: Ledger,

    /// Simulated fork graph for synthetic testing.
    /// Controls the `Graph` fork graph that gets passed to the global program
    /// cache in this harness.
    forks: Forks,
    cache: ProgramCache<Graph>,

    /// Current batch processor env.
    current_env: u8,
    /// Upcoming env for new epoch.
    upcoming_env: u8,

    /// Every entry the scenario put in, keyed by the slot it landed at. A
    /// slot can hold several (one per environment).
    deployed: HashMap<(u8, Slot), Vec<Arc<ProgramCacheEntry>>>,

    /// Cooperative loading tasks handed out and not yet finished, with the
    /// batch which asked for each and the account state it asked against.
    pending: HashMap<u8, PendingLoad>,

    /// A load has already completed for this `(program, deployment slot,
    /// environment, owner)`.
    loaded_here: BTreeSet<(u8, Slot, u8, ProgramCacheEntryOwner)>,

    extractions: Vec<ExtractionRecord>,
    violations: Vec<Violation>,
}

impl Harness {
    fn deploy(&mut self, program: u8, at: Slot, owner: ProgramCacheEntryOwner, env: u8) {
        self.place(program, at, owner, env, EntryKind::Unloaded);
    }

    fn close(&mut self, program: u8, at: Slot) {
        let owner = self
            .ledger
            .account_state(&self.tree, program, at)
            .map(|state| state.owner)
            .unwrap_or(ProgramCacheEntryOwner::LoaderV3);
        let env = self.current_env;
        self.place(program, at, owner, env, EntryKind::Closed);
    }

    fn extract(&mut self, programs: &[u8], fork_tip: Slot, env: u8) {
        if !rules::can_extract(self.forks.contains(fork_tip)) {
            return;
        }

        let requested: Vec<Requested> = (0..NUM_PROGRAMS)
            .filter(|program| programs.contains(program))
            .filter_map(|program| {
                let state = self.ledger.account_state(&self.tree, program, fork_tip)?;
                // In production, closed programs can't land in the search list.
                // `filter_executable_program_accounts` skips them.
                (state.kind != EntryKind::Closed).then_some(Requested {
                    program,
                    key: program_id(program),
                    state,
                })
            })
            .collect();
        if requested.is_empty() {
            return;
        }

        let mut search_for: Vec<ProgramToLoad> = requested
            .iter()
            .map(|asked| ProgramToLoad {
                program_id: &asked.key,
                loader: asked.state.owner,
                deployment_slot: asked.state.deployment_slot,
            })
            .collect();
        let batch_env = environment(env);
        let mut extracted = ProgramCacheForTxBatch::new(fork_tip);
        let task = self
            .cache
            .extract(&mut search_for, &mut extracted, &batch_env, true, true);

        // At most one program per call is handed a loading task.
        if let Some(asked) = task.and_then(|task| requested.iter().find(|asked| asked.key == task))
        {
            // Record the pending cooperative load.
            self.pending.insert(
                asked.program,
                PendingLoad {
                    batch_slot: fork_tip,
                    env,
                    requested: asked.state,
                },
            );
        }

        // Record extraction observations and check for invariant violations.
        let ancestry = self.tree.ancestry(fork_tip);
        for asked in &requested {
            let Requested {
                program,
                state,
                key,
            } = *asked;
            let extraction = Extraction {
                program,
                batch_slot: fork_tip,
                batch_env: batch_env.clone(),
                ancestry: ancestry.clone(),
                account_state: state,
                candidates: self
                    .deployed
                    .get(&(program, state.deployment_slot))
                    .cloned()
                    .unwrap_or_default(),
                returned: extracted.find(&key),
                started_load: task == Some(key),
                already_loaded_here: self.loaded_here.contains(&(
                    program,
                    state.deployment_slot,
                    env,
                    state.owner,
                )),
            };
            self.extractions.push(extraction.record());
            self.check_invariants(&extraction);
        }
    }

    fn finish_load(&mut self, program: u8, result: LoadResult) {
        let Some(PendingLoad {
            batch_slot,
            env,
            requested: state,
        }) = self.pending.remove(&program)
        else {
            return;
        };
        match state.kind {
            EntryKind::Closed | EntryKind::DelayVisibility => return,
            EntryKind::FailedVerification | EntryKind::Loaded | EntryKind::Unloaded => (),
        }
        let kind = match result {
            LoadResult::Loaded => EntryKind::Loaded,
            LoadResult::FailedVerification => EntryKind::FailedVerification,
        };
        let deployment_slot = state.deployment_slot;
        let state = AccountState {
            env,
            kind,
            deployment_slot,
            ..state
        };
        let key = program_id(program);
        let entry = build_entry(state);
        self.deployed
            .entry((program, state.deployment_slot))
            .or_default()
            .push(Arc::clone(&entry));
        self.loaded_here
            .insert((program, deployment_slot, env, state.owner));
        self.cache
            .finish_cooperative_loading_task(&environment(env), batch_slot, key, entry);
    }

    fn prune(&mut self, root: Slot, new_env: Option<u8>) -> bool {
        if !rules::can_root(self.forks.contains(root), root, self.cache.latest_root_slot) {
            return false;
        }
        // Prune the cache *before* moving the root, as is done in production
        // via `votor::root_utils`.
        let new_env = new_env.map(environment);
        {
            let fork_graph = self.forks.read();
            self.cache.prune(root, new_env, &fork_graph);
        }
        self.forks.set_root(root);
        self.forget_dropped_loads();
        true
    }

    fn recompile_for_epoch(&mut self, program: u8, fork_tip: Slot) {
        let upcoming = self.upcoming_env;
        self.extract(&[program], fork_tip, upcoming);
    }

    fn cross_epoch_boundary(&mut self, root: Slot) {
        let upcoming = self.upcoming_env;
        if !self.prune(root, Some(upcoming)) {
            return;
        }
        self.current_env = upcoming;
        self.upcoming_env = upcoming.wrapping_add(1) % NUM_ENVIRONMENTS;
    }

    fn purge_slot(&mut self, slot: Slot) {
        if !rules::can_dump(self.forks.contains(slot), slot, self.cache.latest_root_slot) {
            return;
        }
        self.ledger.purge(slot);
        self.deployed.retain(|(_, at_slot), _| *at_slot != slot);
        self.pending
            .retain(|_, load| load.requested.deployment_slot != slot);
        self.cache.prune_by_deployment_slot(slot);
        self.forget_dropped_loads();
    }

    fn place(
        &mut self,
        program: u8,
        at: Slot,
        owner: ProgramCacheEntryOwner,
        env: u8,
        kind: EntryKind,
    ) {
        if !rules::can_deploy(self.forks.contains(at), at, self.cache.latest_root_slot) {
            return;
        }
        let state = AccountState {
            deployment_slot: at,
            owner,
            env,
            kind,
        };
        if !self.ledger.deploy(program, at, state) {
            return;
        }
        let key = program_id(program);
        let entry = build_entry(state);
        self.deployed
            .entry((program, at))
            .or_default()
            .push(Arc::clone(&entry));

        // Models the round-trip for a "modified entry" in production.
        let mut batch = ProgramCacheForTxBatch::new(at);
        batch.store_modified_entry(key, entry);
        let modified = batch.drain_modified_entries();
        batch.merge(&modified);
        self.cache.merge(&environment(env), at, &modified);
    }

    fn check_invariants(&mut self, extraction: &Extraction) {
        for invariant in invariants::all() {
            if let Some(violation) = invariant.check(extraction) {
                self.violations.push(violation);
            }
        }
    }

    fn forget_dropped_loads(&mut self) {
        let held: BTreeSet<(u8, Slot, u8, ProgramCacheEntryOwner)> = self
            .cache
            .get_flattened_entries_for_tests()
            .iter()
            .filter(|(_, entry)| {
                // Skip over unloaded entries.
                matches!(
                    EntryKind::of(&entry.program),
                    EntryKind::Loaded | EntryKind::FailedVerification
                )
            })
            .filter_map(|(key, entry)| {
                Some((
                    program_index(key)?,
                    entry.deployment_slot,
                    environment_index(entry.program.get_environment()?)?,
                    entry.account_owner,
                ))
            })
            .collect();
        self.loaded_here.retain(|mark| held.contains(mark));
    }

    fn fingerprint(&self) -> Vec<Entry> {
        let mut held: Vec<Entry> = self
            .cache
            .get_flattened_entries_for_tests()
            .iter()
            .map(|(key, entry)| Entry {
                program: program_index(key).unwrap_or(u8::MAX),
                deployment_slot: entry.deployment_slot,
                owner: entry.account_owner,
                kind: EntryKind::of(&entry.program),
                env: entry
                    .program
                    .get_environment()
                    .map(|env| environment_index(env).unwrap_or(u8::MAX)),
            })
            .collect();
        held.sort();
        held
    }
}

impl super::Runner for Harness {
    fn new(scenario: &Scenario) -> Self {
        let forks = Forks::new(&scenario.tree);
        let mut cache = ProgramCache::<Graph>::new(forks.root());
        cache.set_fork_graph(forks.weak());
        Self {
            cache,
            forks,
            tree: scenario.tree.clone(),
            ledger: Ledger::default(),
            current_env: 0,
            upcoming_env: 1,
            deployed: HashMap::new(),
            pending: HashMap::new(),
            loaded_here: BTreeSet::new(),
            extractions: Vec::new(),
            violations: Vec::new(),
        }
    }

    fn step(&mut self, op: &Op) {
        match op {
            Op::Deploy {
                program,
                at,
                owner,
                env,
            } => self.deploy(*program, *at, *owner, *env),
            Op::Close { program, at } => self.close(*program, *at),
            Op::Extract { programs, fork_tip } => {
                self.extract(programs, *fork_tip, self.current_env)
            }
            Op::FinishLoad { program, result } => self.finish_load(*program, *result),
            Op::Prune { root } => {
                self.prune(*root, None);
            }
            Op::RecompileForEpoch { program, fork_tip } => {
                self.recompile_for_epoch(*program, *fork_tip)
            }
            Op::CrossEpochBoundary { root } => self.cross_epoch_boundary(*root),
            Op::PurgeSlot { slot } => self.purge_slot(*slot),
        }
    }

    fn finish(self) -> Report {
        Report {
            fingerprint: self.fingerprint(),
            extractions: self.extractions,
            violations: self.violations,
        }
    }
}

struct Requested {
    program: u8,
    key: Pubkey,
    state: AccountState,
}

struct PendingLoad {
    batch_slot: Slot,
    env: u8,
    requested: AccountState,
}

static ENVIRONMENTS: LazyLock<[ProgramRuntimeEnvironment; NUM_ENVIRONMENTS as usize]> =
    LazyLock::new(|| {
        std::array::from_fn(|_| ProgramRuntimeEnvironment::from(BuiltinProgram::new_mock()))
    });

fn environment(index: u8) -> ProgramRuntimeEnvironment {
    ENVIRONMENTS[usize::from(index % NUM_ENVIRONMENTS)].clone()
}

fn environment_index(env: &ProgramRuntimeEnvironment) -> Option<u8> {
    ENVIRONMENTS
        .iter()
        .position(|at| at == env)
        .and_then(|index| u8::try_from(index).ok())
}

static PROGRAM_IDS: LazyLock<[Pubkey; NUM_PROGRAMS as usize]> = LazyLock::new(|| {
    std::array::from_fn(|program| {
        let mut address = [0u8; 32];
        address[..8].copy_from_slice(b"harness!");
        address[8] = program as u8;
        Pubkey::new_from_array(address)
    })
});

fn program_id(program: u8) -> Pubkey {
    PROGRAM_IDS[usize::from(program % NUM_PROGRAMS)]
}

fn program_index(key: &Pubkey) -> Option<u8> {
    PROGRAM_IDS
        .iter()
        .position(|id| id == key)
        .and_then(|index| u8::try_from(index).ok())
}

fn build_entry(state: AccountState) -> Arc<ProgramCacheEntry> {
    let env = environment(state.env);
    let program = match state.kind {
        EntryKind::Loaded => ProgramCacheEntryType::Loaded(
            Executable::load(NOOP_OK, Arc::clone(&*env)).expect("load noop_ok"),
        ),
        EntryKind::Unloaded => ProgramCacheEntryType::Unloaded(env),
        EntryKind::Closed => ProgramCacheEntryType::Closed,
        EntryKind::DelayVisibility => ProgramCacheEntryType::DelayVisibility,
        EntryKind::FailedVerification => ProgramCacheEntryType::FailedVerification(env),
    };
    Arc::new(ProgramCacheEntry {
        program,
        account_owner: state.owner,
        deployment_slot: state.deployment_slot,
        ..ProgramCacheEntry::default()
    })
}
