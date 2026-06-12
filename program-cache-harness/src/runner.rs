use {
    crate::{
        corpus, invariants,
        scenario::{SLOTS_PER_EPOCH, Scenario},
        shims::{sync::RwLock, thread},
    },
    solana_account::{Account, AccountSharedData},
    solana_feature_gate_interface::{self as feature, Feature},
    solana_instruction::Instruction,
    solana_keypair::Keypair,
    solana_leader_schedule::SlotLeader,
    solana_message::Message,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_program_runtime::loaded_programs::ProgramRuntimeEnvironment,
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_runtime::{
        bank::Bank,
        bank_forks::BankForks,
        genesis_utils::{GenesisConfigInfo, create_genesis_config},
    },
    solana_sdk_ids::{bpf_loader, system_program},
    solana_signer::Signer,
    solana_transaction::Transaction,
    std::sync::Arc,
};

struct Setup {
    root_bank: Arc<Bank>,
    bank_forks: Arc<RwLock<BankForks>>,
    genesis_environment: ProgramRuntimeEnvironment,
    rent: Rent,
    program_ids: Arc<Vec<Pubkey>>,
    payers: Vec<Arc<Keypair>>,
}

fn setup(num_programs: usize, num_payers: usize) -> Setup {
    let GenesisConfigInfo {
        mut genesis_config, ..
    } = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
    genesis_config.accounts.remove(&corpus::TRIGGER_FEATURE_ID);
    let (root_bank, bank_forks) =
        Bank::new_for_tests(&genesis_config).wrap_with_bank_forks_for_tests();
    let genesis_environment = root_bank
        .get_transaction_processor()
        .program_runtime_environment_for_epoch(0);

    let program_account = AccountSharedData::from(Account {
        lamports: Rent::default()
            .minimum_balance(corpus::NOOP_SBPF_V0_ELF.len())
            .max(1),
        data: corpus::NOOP_SBPF_V0_ELF.to_vec(),
        owner: bpf_loader::id(),
        executable: true,
        rent_epoch: 0,
    });
    let program_ids = Arc::new(
        (0..num_programs)
            .map(|_| {
                let program_id = Pubkey::new_unique();
                root_bank.store_account(&program_id, &program_account);
                program_id
            })
            .collect(),
    );

    let payer_account = AccountSharedData::new(LAMPORTS_PER_SOL, 0, &system_program::id());
    let payers = (0..num_payers)
        .map(|_| {
            let payer = Keypair::new();
            root_bank.store_account(&payer.pubkey(), &payer_account);
            Arc::new(payer)
        })
        .collect();

    Setup {
        root_bank,
        bank_forks,
        genesis_environment,
        rent: genesis_config.rent,
        program_ids,
        payers,
    }
}

#[derive(Debug, Default)]
pub struct EpochBoundaryPreparationReport {
    /// Total transactions invoking programs in the entire run.
    pub num_transactions: usize,
    /// An upcoming-environment version of a program was observed in the cache,
    /// thus drain output reached the index.
    pub saw_recompiled_entry: bool,
}

/// Exercises the epoch boundary preparation phase end to end against a real
/// `BankForks`.
///
/// The run stages `TRIGGER_FEATURE_ID` mid-epoch so the upcoming epoch's
/// program runtime environment differs from the current one, then walks the
/// scenario's forks round-robin until each has crossed the epoch boundary.
///
/// Each iteration creates a child bank on one fork. This fork runs the
/// preparation phase inside `Bank::new_from_parent`, which snapshots the
/// recompilation queue when the window opens and thereafter recompiles one
/// queued program per new bank. Meanwhile, submitter threads invoke every
/// scenario program on all other forks' tips.
///
/// The drain's insertions race the submitters' extracts and cooperative
/// loads on the shared global cache.
///
/// Once every fork has crossed the epoch boundary, the run concludes by
/// rooting via `Bank::prune_program_cache`.
///
/// Panics if any expectation is violated:
/// - Every invocation succeeds under both environments.
/// - While the phase is active, the queue drains exactly one program per
///   new bank, across all forks.
/// - Cached entries only ever belong to a known environment, with at most
///   one version per (deployment slot, environment) pair.
/// - At reroot, the preparation state is cleared and only
///   current-environment entries survive.
pub fn run_epoch_boundary_preparation(scenario: &Scenario) -> EpochBoundaryPreparationReport {
    let Setup {
        root_bank,
        bank_forks,
        genesis_environment,
        rent,
        program_ids,
        payers,
    } = setup(scenario.num_programs, scenario.num_submitter_threads);

    // Create the forks, each seeded on its own slot. Fork `i` owns the
    // slots where `(slot - 1) % num_forks == i`.
    let num_forks = scenario.num_forks.max(2) as u64;
    let mut tips: Vec<Arc<Bank>> = (0..num_forks)
        .map(|fork| {
            Bank::new_from_parent_with_bank_forks(
                &bank_forks,
                root_bank.clone(),
                SlotLeader::default(),
                fork.saturating_add(1),
            )
        })
        .collect();

    // Stage the feature on every fork; it activates at the epoch boundary.
    let feature_account = feature::create_account(
        &Feature { activated_at: None },
        rent.minimum_balance(Feature::size_of()).max(1),
    );
    for tip in &tips {
        tip.store_account(&corpus::TRIGGER_FEATURE_ID, &feature_account);
    }

    // Warm the cache so the preparation phase has something to snapshot.
    let mut report = EpochBoundaryPreparationReport::default();
    invoke_all_programs(
        &tips[0],
        &payers[0],
        &program_ids,
        0,
        u64::MAX,
        &mut report.num_transactions,
    );

    // Simulate the epoch by iterating through the slots, walking far enough
    // past the epoch boundary for every fork to cross it.
    let last_slot =
        SLOTS_PER_EPOCH.saturating_add(scenario.num_post_boundary_slots.max(1).max(num_forks - 1));
    let mut previous_observation = invariants::observe_preparation(&tips[0]);
    for slot in (num_forks + 1)..=last_slot {
        // The slot's owning fork advances while all other tips are invoked.
        let advancing_fork = ((slot - 1) % num_forks) as usize;
        let parent = tips[advancing_fork].clone();
        let invocation_tips: Vec<Arc<Bank>> = tips
            .iter()
            .enumerate()
            .filter(|(fork, _)| *fork != advancing_fork)
            .map(|(_, tip)| tip.clone())
            .collect();

        // Spawn worker threads to blast all of the programs in the cache with
        // invocations.
        let handles: Vec<_> = payers
            .iter()
            .map(|payer| {
                let invocation_tips = invocation_tips.clone();
                let payer = payer.clone();
                let program_ids = program_ids.clone();
                let num_invocations = scenario.num_invocations_per_slot;
                thread::spawn(move || {
                    let mut num_transactions = 0;
                    for invocation in 0..num_invocations {
                        for tip in &invocation_tips {
                            invoke_all_programs(
                                tip,
                                &payer,
                                &program_ids,
                                slot,
                                invocation as u64,
                                &mut num_transactions,
                            );
                        }
                    }
                    num_transactions
                })
            })
            .collect();
        let new_tip =
            Bank::new_from_parent_with_bank_forks(&bank_forks, parent, SlotLeader::default(), slot);
        for handle in handles {
            report.num_transactions += handle.join().unwrap();
        }

        // Check for invariants.
        let observation = invariants::observe_preparation(&new_tip);
        invariants::assert_queue_drains(&previous_observation, &observation);
        report.saw_recompiled_entry |=
            invariants::assert_slot_versions(&new_tip, &program_ids, &genesis_environment);
        previous_observation = observation;

        // Point the advanced fork's tip at the new bank.
        tips[advancing_fork] = new_tip;
    }

    // Conclude the phase via rooting past the boundary.
    assert!(
        tips.iter().all(|tip| tip.epoch() > 0),
        "every fork must cross the epoch boundary"
    );
    let newest_tip = tips
        .into_iter()
        .max_by_key(|tip| tip.slot())
        .expect("at least two forks");
    newest_tip.prune_program_cache(&bank_forks.read().unwrap());
    invariants::assert_phase_concluded(&newest_tip, &program_ids);
    report
}

fn invoke_all_programs(
    bank: &Bank,
    payer: &Keypair,
    program_ids: &[Pubkey],
    iteration_slot: u64,
    invocation: u64,
    num_transactions: &mut usize,
) {
    for program_id in program_ids {
        let mut data = bank.slot().to_le_bytes().to_vec();
        data.extend_from_slice(&iteration_slot.to_le_bytes());
        data.extend_from_slice(&invocation.to_le_bytes());
        let instruction = Instruction::new_with_bytes(*program_id, &data, Vec::new());
        let message = Message::new(&[instruction], Some(&payer.pubkey()));
        let transaction = Transaction::new(&[payer], message, bank.last_blockhash());
        let result = bank.process_transaction(&transaction);
        *num_transactions += 1;
        assert_eq!(
            result,
            Ok(()),
            "{program_id} must execute at slot {}",
            bank.slot(),
        );
    }
}
