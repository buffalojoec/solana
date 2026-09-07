//! # V2: Production Runner
//!
//! Unlike the V1 runner, this runner actually executes across production code.
//!
//! This runner drives real `Bank` nodes in a real `BankForks` fork tree,
//! enacts deployments and closures through real transactions, and produces
//! program cache entry extractions through the real transaction processing
//! pipeline by invoking the program with a real transaction.
//!
//! It also uses a real ProgramRuntimeEnvironment, seeded by a select handful
//! of SVMFeatureSet candidates (syscall feature activations).
//!
//! This version is obviously much slower and therefore takes much longer to
//! saturate, but coverage is much richer.

pub mod account;
pub mod forks;
pub mod keypair;
pub mod transaction;

use {
    self::forks::Forks,
    crate::{
        elf::{NOOP_OK, VERIFIER_ERR},
        entry::{Entry, EntryKind},
        extraction::{EbppRecord, Extraction, ExtractionRecord},
        invariants::{self, Violation},
        ledger::{AccountState, Ledger},
        report::Report,
        rules,
        scenario::{ForkTree, LoadResult, NUM_PROGRAMS, Op, Scenario, Seed},
    },
    solana_clock::Slot,
    solana_keypair::Keypair,
    solana_program_runtime::{
        loaded_programs::{ProgramCacheForTxBatch, ProgramRuntimeEnvironment, ProgramToLoad},
        program_cache_entry::{ProgramCacheEntry, ProgramCacheEntryOwner, ProgramCacheEntryType},
    },
    solana_pubkey::Pubkey,
    solana_runtime::bank::Bank,
    solana_signer::Signer as _,
    solana_svm::{
        transaction_error_metrics::TransactionErrorMetrics,
        transaction_processor::TransactionProcessingConfig,
    },
    solana_svm_timings::ExecuteTimings,
    std::{
        collections::{BTreeSet, HashMap},
        sync::Arc,
    },
};

pub struct Harness {
    /// The fork tree from the provided scenario. Read-only.
    tree: ForkTree,
    /// Account state independent ledger, tracked as we step through ops.
    ledger: Ledger,

    /// Real banks, in the shape of the tree.
    forks: Forks,

    /// Every entry the cache built at each deployment slot. v2 makes none of
    /// its own, so this records what came back the first time.
    deployed: HashMap<(u8, Slot), Vec<Arc<ProgramCacheEntry>>>,

    /// What each program's most recent batch loaded, and what it loaded it
    /// with.
    pending: HashMap<u8, PendingLoad>,

    /// A load has already completed for this `(program, deployment slot,
    /// environment, owner)`.
    loaded_here: BTreeSet<(u8, Slot, u8, ProgramCacheEntryOwner)>,

    /// The keypairs each program is deployed with, made on first use.
    keys: HashMap<u8, Keys>,

    /// The environments the run has executed on, in the order it first saw
    /// them, so a report names them the way v1 does.
    environments: Vec<ProgramRuntimeEnvironment>,

    extractions: Vec<ExtractionRecord>,
    ebpp_records: Vec<EbppRecord>,
    violations: Vec<Violation>,
}

impl Harness {
    fn deploy(&mut self, program: u8, at: Slot) {
        let existing = self.ledger.account_state(&self.tree, program, at);
        if !rules::can_deploy_over(existing.map(|state| state.owner)) {
            return;
        }
        if !self.can_place(program, at) {
            return;
        }
        let Some(deployment_slot) = self.deploy_program(program, at, existing.is_some()) else {
            return;
        };
        let env = self.environment_at(at);
        self.ledger.deploy(
            program,
            at,
            AccountState {
                deployment_slot,
                owner: ProgramCacheEntryOwner::LoaderV3,
                env,
                kind: EntryKind::Unloaded,
            },
        );
    }

    fn close(&mut self, program: u8, at: Slot) {
        let Some(state) = self.ledger.account_state(&self.tree, program, at) else {
            return;
        };
        if state.kind == EntryKind::Closed || !rules::can_close(Some(state.owner)) {
            return;
        }
        if !self.can_place(program, at) {
            return;
        }
        let Some((bank, payer)) = self.ready(at) else {
            return;
        };
        let key = self.program_id(program);
        let transaction = transaction::close(&payer, &key, &payer, bank.last_blockhash());
        if bank.process_transaction(&transaction).is_err() {
            return;
        }
        self.ledger.deploy(
            program,
            at,
            AccountState {
                kind: EntryKind::Closed,
                ..state
            },
        );
    }

    fn extract(&mut self, programs: &[u8], fork_tip: Slot) {
        if !rules::can_extract(self.forks.contains(fork_tip)) {
            return;
        }
        let Some(bank) = self.forks.writable_bank(fork_tip) else {
            return;
        };
        let requested: Vec<Requested> = (0..NUM_PROGRAMS)
            .filter(|program| programs.contains(program))
            .filter_map(|program| {
                let state = self.ledger.account_state(&self.tree, program, fork_tip)?;
                // In production, closed programs can't land in the search list.
                // `filter_executable_program_accounts` skips them.
                (state.kind != EntryKind::Closed).then_some(Requested {
                    program,
                    key: self.program_id(program),
                    state,
                })
            })
            .collect();
        if requested.is_empty() {
            return;
        }

        let payer = self.forks.payer().insecure_clone();
        let keys: Vec<Pubkey> = requested.iter().map(|asked| asked.key).collect();
        bank.register_unique_recent_blockhash_for_test();
        let transaction = transaction::invoke(&payer, &keys, bank.last_blockhash());
        let batch = bank.prepare_batch_for_tests(vec![transaction]);
        let output = bank.load_and_execute_transactions(
            &batch,
            usize::MAX,
            &mut ExecuteTimings::default(),
            &mut TransactionErrorMetrics::default(),
            TransactionProcessingConfig::default(),
        );
        let extracted = output.program_cache_for_tx_batch;
        let batch_env = bank
            .transaction_processor()
            .program_runtime_environment
            .clone();
        let env = self.environment_index(&batch_env);
        let ancestry = self.tree.ancestry(fork_tip);

        for asked in &requested {
            let Requested {
                program,
                key,
                state,
            } = *asked;
            let returned = extracted.find(&key);
            let started_load = extracted.loaded_keys.contains(&key);
            // v2 builds no entries of its own, so the first one the cache hands
            // back at a deployment slot is what it put there.
            if let Some(entry) = returned.clone() {
                let candidates = self
                    .deployed
                    .entry((program, state.deployment_slot))
                    .or_default();
                if !candidates.iter().any(|at| Arc::ptr_eq(at, &entry)) {
                    candidates.push(entry);
                }
            }
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
                returned,
                started_load,
                already_loaded_here: self.loaded_here.contains(&(
                    program,
                    state.deployment_slot,
                    env,
                    state.owner,
                )),
            };
            if started_load {
                self.loaded_here
                    .insert((program, state.deployment_slot, env, state.owner));
                // A batch in the deployment slot itself is served a delay
                // window tombstone, which the cache has no transition for
                // landing again.
                let landable = extraction.returned.clone().filter(|entry| {
                    !matches!(entry.program, ProgramCacheEntryType::DelayVisibility)
                });
                self.pending.insert(
                    program,
                    PendingLoad {
                        batch_slot: fork_tip,
                        env: batch_env.clone(),
                        requested: state,
                        entry: landable,
                    },
                );
            }
            self.extractions.push(extraction.record(env));
            self.check_invariants(&extraction);
        }
    }

    fn finish_load(&mut self, program: u8, _result: LoadResult) {
        let Some(load) = self.pending.remove(&program) else {
            return;
        };
        let Some(entry) = load.entry else {
            return;
        };
        let key = self.program_id(program);
        let root = self.forks.root_bank();
        let mut cache = root
            .transaction_processor()
            .global_program_cache
            .write()
            .unwrap();
        let still_there = cache
            .get_flattened_entries_for_tests()
            .iter()
            .any(|(at, held)| *at == key && held.deployment_slot == load.requested.deployment_slot);
        if still_there {
            return;
        }
        // Take the task the way a batch would, so the cache knows a load is in
        // flight for this key, then complete it. That pair is the race.
        let mut search_for = vec![ProgramToLoad {
            program_id: &key,
            loader: load.requested.owner,
            deployment_slot: load.requested.deployment_slot,
        }];
        let mut batch = ProgramCacheForTxBatch::new(load.batch_slot);
        let task = cache.extract(&mut search_for, &mut batch, &load.env, false, false);
        if task != Some(key) {
            return;
        }
        cache.finish_cooperative_loading_task(&load.env, load.batch_slot, key, entry);
    }

    fn prune(&mut self, root: Slot) {
        if !rules::can_root(self.forks.contains(root), root, self.forks.root()) {
            return;
        }
        // Prune the cache *before* moving the root, as is done in production
        // via `votor::root_utils`.
        self.forks.prune_program_cache(root);
        self.forks.set_root(root);
        self.forget_dropped_loads();
    }

    fn recompile_for_epoch(&mut self, program: u8, fork_tip: Slot) {
        if !rules::can_extract(self.forks.contains(fork_tip)) {
            return;
        }
        let Some(bank) = self.forks.bank(fork_tip) else {
            return;
        };
        let key = self.program_id(program);
        // The queue is snapshotted from what the cache is holding, so a program
        // the cache has nothing compiled for is not one it could have named.
        let Some(entry) = bank
            .transaction_processor()
            .global_program_cache
            .read()
            .unwrap()
            .get_flattened_entries()
            .into_iter()
            .find_map(|(at, entry)| (at == key).then_some(entry))
        else {
            return;
        };
        self.arm_ebpp();
        let processor = bank.transaction_processor();
        let upcoming = processor
            .epoch_boundary_preparation
            .read()
            .unwrap()
            .upcoming_environment
            .clone();
        let Some(upcoming) = upcoming else {
            return;
        };
        let already_built = processor
            .global_program_cache
            .read()
            .unwrap()
            .get_flattened_entries()
            .into_iter()
            .any(|(at, held)| {
                at == key
                    && held
                        .program
                        .get_environment()
                        .is_some_and(|env| *env == upcoming)
            });
        processor
            .epoch_boundary_preparation
            .write()
            .unwrap()
            .programs_to_recompile
            .push((key, entry));
        bank.prepare_program_cache_for_upcoming_feature_set();
        let env = self.environment_index(&upcoming);
        self.ebpp_records.push(EbppRecord {
            program,
            fork_tip,
            env,
            already_built,
            started_load: !already_built,
        });
    }

    fn purge_slot(&mut self, slot: Slot) {
        if !rules::can_dump(self.forks.contains(slot), slot, self.forks.root()) {
            return;
        }
        self.ledger.purge(slot);
        self.deployed.retain(|(_, at_slot), _| *at_slot != slot);
        self.pending
            .retain(|_, load| load.requested.deployment_slot != slot);
        self.forks.clear(slot);
        self.forget_dropped_loads();
    }

    fn seed(&mut self, seed: Seed) {
        let key = self.program_id(seed.program);
        let authority = self.forks.payer().pubkey();
        let elf = if seed.verifies { NOOP_OK } else { VERIFIER_ERR };
        let accounts = account::deployed_under(seed.owner, &key, &authority, elf);
        if accounts.is_empty() {
            return;
        }
        let bank = self.forks.root_bank();
        for (key, account) in &accounts {
            bank.store_account(key, account);
        }
        let env = self.environment_at(0);
        self.ledger.deploy(
            seed.program,
            0,
            AccountState {
                deployment_slot: 0,
                owner: seed.owner,
                env,
                kind: EntryKind::Unloaded,
            },
        );
    }

    fn arm_ebpp(&mut self) {
        let bank = self.forks.root_bank();
        let processor = bank.transaction_processor();
        self.environment_index(&processor.program_runtime_environment.clone());
        if processor
            .epoch_boundary_preparation
            .read()
            .unwrap()
            .upcoming_environment
            .is_some()
        {
            return;
        }
        let (upcoming, _) = bank.compute_active_feature_set(true);
        let environment = bank.create_program_runtime_environment(&upcoming);
        self.environment_index(&environment);
        let mut preparation = processor.epoch_boundary_preparation.write().unwrap();
        preparation.upcoming_epoch = bank.epoch().saturating_add(1);
        preparation.upcoming_environment = Some(environment);
    }

    fn can_place(&mut self, program: u8, at: Slot) -> bool {
        self.ledger.deployed_at(program, at).is_none()
            && rules::can_deploy(self.forks.contains(at), at, self.forks.root())
    }

    fn deploy_program(&mut self, program: u8, at: Slot, upgrade: bool) -> Option<Slot> {
        let (bank, payer) = self.ready(at)?;
        let (program_key, buffer_key) = {
            let keys = self.keys(program);
            (keys.program.insecure_clone(), keys.buffer.insecure_clone())
        };
        bank.store_account(
            &buffer_key.pubkey(),
            &account::buffer(&payer.pubkey(), NOOP_OK),
        );
        let transaction = if upgrade {
            transaction::upgrade(
                &payer,
                &program_key.pubkey(),
                &buffer_key.pubkey(),
                &payer,
                bank.last_blockhash(),
            )
        } else {
            transaction::deploy(
                &payer,
                &program_key,
                &buffer_key.pubkey(),
                &payer,
                NOOP_OK.len(),
                bank.last_blockhash(),
            )
        };
        bank.process_transaction(&transaction).ok()?;
        Some(at)
    }

    fn ready(&mut self, at: Slot) -> Option<(Arc<Bank>, Keypair)> {
        let bank = self.forks.writable_bank(at)?;
        bank.register_unique_recent_blockhash_for_test();
        Some((bank, self.forks.payer().insecure_clone()))
    }

    fn check_invariants(&mut self, extraction: &Extraction) {
        for invariant in invariants::all() {
            if let Some(violation) = invariant.check(extraction) {
                self.violations.push(violation);
            }
        }
    }

    fn forget_dropped_loads(&mut self) {
        let names = self.program_ids();
        let root = self.forks.root_bank();
        let cache = root
            .transaction_processor()
            .global_program_cache
            .read()
            .unwrap();
        let held: BTreeSet<(u8, Slot, u8, ProgramCacheEntryOwner)> = cache
            .get_flattened_entries_for_tests()
            .iter()
            // The run's own programs first: a bank seeds builtins, which are
            // not entries the harness ever put in.
            .filter_map(|(key, entry)| Some((*names.get(key)?, entry)))
            // Skip over unloaded entries.
            .filter(|(_, entry)| {
                matches!(
                    EntryKind::of(&entry.program),
                    EntryKind::Loaded | EntryKind::FailedVerification
                )
            })
            .filter_map(|(program, entry)| {
                let env = self.environments.iter().position(|at| {
                    entry
                        .program
                        .get_environment()
                        .is_some_and(|held| held == at)
                })?;
                Some((
                    program,
                    entry.deployment_slot,
                    u8::try_from(env).ok()?,
                    entry.account_owner,
                ))
            })
            .collect();
        drop(cache);
        self.loaded_here.retain(|mark| held.contains(mark));
    }

    fn fingerprint(&self) -> Vec<Entry> {
        let names = self.program_ids();
        let root = self.forks.root_bank();
        let cache = root
            .transaction_processor()
            .global_program_cache
            .read()
            .unwrap();
        let mut held: Vec<Entry> = cache
            .get_flattened_entries_for_tests()
            .iter()
            .filter_map(|(key, entry)| Some((*names.get(key)?, entry)))
            .map(|(program, entry)| Entry {
                program,
                deployment_slot: entry.deployment_slot,
                owner: entry.account_owner,
                kind: EntryKind::of(&entry.program),
                env: entry.program.get_environment().map(|env| {
                    self.environments
                        .iter()
                        .position(|at| at == env)
                        .and_then(|index| u8::try_from(index).ok())
                        .unwrap_or(u8::MAX)
                }),
            })
            .collect();
        held.sort();
        held
    }

    fn environment_at(&mut self, slot: Slot) -> u8 {
        let Some(bank) = self.forks.bank(slot) else {
            return 0;
        };
        let env = bank
            .transaction_processor()
            .program_runtime_environment
            .clone();
        self.environment_index(&env)
    }

    fn environment_index(&mut self, env: &ProgramRuntimeEnvironment) -> u8 {
        let index = self
            .environments
            .iter()
            .position(|at| at == env)
            .unwrap_or_else(|| {
                self.environments.push(env.clone());
                self.environments.len().saturating_sub(1)
            });
        u8::try_from(index).unwrap_or(u8::MAX)
    }

    fn program_id(&mut self, program: u8) -> Pubkey {
        self.keys(program).program.pubkey()
    }

    fn keys(&mut self, program: u8) -> &Keys {
        self.keys.entry(program).or_insert_with(|| Keys {
            program: keypair::keypair(program),
            buffer: keypair::keypair(program.saturating_add(64)),
        })
    }

    fn program_ids(&self) -> HashMap<Pubkey, u8> {
        self.keys
            .iter()
            .map(|(program, keys)| (keys.program.pubkey(), *program))
            .collect()
    }
}

impl super::Runner for Harness {
    fn new(scenario: &Scenario) -> Self {
        let mut harness = Self {
            tree: scenario.tree.clone(),
            ledger: Ledger::default(),
            forks: Forks::new(&scenario.tree),
            keys: HashMap::new(),
            deployed: HashMap::new(),
            pending: HashMap::new(),
            loaded_here: BTreeSet::new(),
            environments: Vec::new(),
            extractions: Vec::new(),
            ebpp_records: Vec::new(),
            violations: Vec::new(),
        };
        // Arm the preparation phase before anything is built, and name both
        // environments while doing it.
        //
        // Production arms in the back half of an epoch, before any fork has
        // crossed, and every bank which then crosses consolidates onto the one
        // environment it anticipated. Arming late - or not at all - leaves each
        // fork minting its own, so the run ends up with an environment per fork
        // instead of one per epoch.
        harness.arm_ebpp();
        for seed in &scenario.seeds {
            harness.seed(*seed);
        }
        harness
    }

    fn step(&mut self, op: &Op) {
        match op {
            Op::Deploy { program, at } => self.deploy(*program, *at),
            Op::Close { program, at } => self.close(*program, *at),
            Op::Extract { programs, fork_tip } => self.extract(programs, *fork_tip),
            Op::FinishLoad { program, result } => self.finish_load(*program, *result),
            Op::Prune { root } => self.prune(*root),
            Op::RecompileForEpoch { program, fork_tip } => {
                self.recompile_for_epoch(*program, *fork_tip)
            }
            Op::PurgeSlot { slot } => self.purge_slot(*slot),
        }
    }

    fn finish(self) -> Report {
        Report {
            fingerprint: self.fingerprint(),
            extractions: self.extractions,
            ebpp_records: self.ebpp_records,
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
    env: ProgramRuntimeEnvironment,
    requested: AccountState,
    entry: Option<Arc<ProgramCacheEntry>>,
}

struct Keys {
    program: Keypair,
    buffer: Keypair,
}
