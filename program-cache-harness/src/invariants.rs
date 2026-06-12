use {
    solana_program_runtime::loaded_programs::ProgramRuntimeEnvironment, solana_pubkey::Pubkey,
    solana_runtime::bank::Bank,
};

/// Snapshot of the global `EpochBoundaryPreparation` state.
#[derive(Clone, Copy, Debug)]
pub struct PreparationObservation {
    pub has_upcoming_environment: bool,
    pub num_programs_to_recompile: usize,
}

pub fn observe_preparation(bank: &Bank) -> PreparationObservation {
    let preparation = bank
        .get_transaction_processor()
        .epoch_boundary_preparation
        .read()
        .unwrap();
    PreparationObservation {
        has_upcoming_environment: preparation.upcoming_environment.is_some(),
        num_programs_to_recompile: preparation.programs_to_recompile.len(),
    }
}

/// While the phase is active, the recompilation queue drains exactly one
/// entry per bank created.
pub fn assert_queue_drains(previous: &PreparationObservation, current: &PreparationObservation) {
    if previous.has_upcoming_environment && current.has_upcoming_environment {
        assert_eq!(
            current.num_programs_to_recompile,
            previous.num_programs_to_recompile.saturating_sub(1),
            "recompilation queue must drain exactly one program per new bank: {previous:?} -> \
             {current:?}",
        );
    }
}

/// Cached entries must belong to a known environment, with at most one version
/// per (deployment slot, environment) pair. Returns whether an
/// upcoming-environment version of any program was observed.
pub fn assert_slot_versions(
    bank: &Bank,
    program_ids: &[Pubkey],
    genesis_environment: &ProgramRuntimeEnvironment,
) -> bool {
    let processor = bank.get_transaction_processor();
    let current = processor.program_runtime_environment_for_epoch(bank.epoch());
    let upcoming = processor.program_runtime_environment_for_epoch(bank.epoch().saturating_add(1));
    let phase_active = *current != *upcoming;
    let cache = processor.global_program_cache.read().unwrap();
    let mut saw_recompiled = false;
    for program_id in program_ids {
        let slot_versions = cache.get_slot_versions_for_tests(program_id);
        assert!(
            slot_versions.len() <= 2,
            "at most one version per environment expected for {program_id}: {slot_versions:?}",
        );
        for entry in slot_versions {
            let environment = entry
                .program
                .get_environment()
                .expect("scenario programs are never builtins");
            assert!(
                **environment == **genesis_environment
                    || **environment == *current
                    || **environment == *upcoming,
                "entry for {program_id} belongs to no known environment: {entry:?}",
            );
            if phase_active && **environment == *upcoming {
                saw_recompiled = true;
            }
        }
    }
    saw_recompiled
}

/// After pruning, the preparation state must be cleared and only
/// current-environment entries may survive.
pub fn assert_phase_concluded(bank: &Bank, program_ids: &[Pubkey]) {
    let observation = observe_preparation(bank);
    assert!(
        !observation.has_upcoming_environment && observation.num_programs_to_recompile == 0,
        "preparation phase must conclude at reroot: {observation:?}",
    );
    let processor = bank.get_transaction_processor();
    let current = processor.program_runtime_environment_for_epoch(bank.epoch());
    let cache = processor.global_program_cache.read().unwrap();
    for program_id in program_ids {
        for entry in cache.get_slot_versions_for_tests(program_id) {
            let environment = entry
                .program
                .get_environment()
                .expect("scenario programs are never builtins");
            assert!(
                **environment == *current,
                "outdated environment survived pruning for {program_id}: {entry:?}",
            );
        }
    }
}
