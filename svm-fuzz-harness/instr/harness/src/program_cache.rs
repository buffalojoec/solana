use {
    solana_bpf_loader_program::syscalls::create_program_runtime_environment_v1,
    solana_compute_budget::compute_budget::ComputeBudget,
    solana_feature_set::{
        enable_program_runtime_v2_and_loader_v4, zk_elgamal_proof_program_enabled,
        zk_token_sdk_enabled, FeatureSet,
    },
    solana_program_runtime::loaded_programs::{
        ProgramCacheEntry, ProgramCacheForTxBatch, ProgramRuntimeEnvironments,
    },
    solana_pubkey::Pubkey,
    solana_runtime::bank::builtins::BUILTINS,
    std::sync::Arc,
};

// These programs have been migrated to Core BPF, and therefore should not be
// included in the fuzzing harness.
const MIGRATED_BUILTINS: &[Pubkey] = &[
    solana_sdk::address_lookup_table::program::id(),
    solana_sdk::config::program::id(),
];

pub fn setup_program_cache(
    feature_set: &FeatureSet,
    compute_budget: &ComputeBudget,
    slot: u64,
) -> ProgramCacheForTxBatch {
    let mut cache = ProgramCacheForTxBatch::default();

    let environments = ProgramRuntimeEnvironments {
        program_runtime_v1: Arc::new(
            create_program_runtime_environment_v1(
                feature_set,
                compute_budget,
                false, /* deployment */
                false, /* debugging_features */
            )
            .unwrap(),
        ),
        ..ProgramRuntimeEnvironments::default()
    };

    cache.set_slot_for_tests(slot);
    cache.environments = environments.clone();
    cache.upcoming_environments = Some(environments);

    for builtin in BUILTINS {
        // Skip migrated builtins.
        if MIGRATED_BUILTINS.contains(&builtin.program_id) {
            continue;
        }

        // Only activate feature-gated builtins if the feature is active.
        if builtin.program_id == solana_sdk::loader_v4::id()
            && !feature_set.is_active(&enable_program_runtime_v2_and_loader_v4::id())
        {
            continue;
        }
        if builtin.program_id == solana_zk_sdk::zk_elgamal_proof_program::id()
            && !feature_set.is_active(&zk_elgamal_proof_program_enabled::id())
        {
            continue;
        }
        if builtin.program_id == solana_zk_token_sdk::zk_token_proof_program::id()
            && !feature_set.is_active(&zk_token_sdk_enabled::id())
        {
            continue;
        }

        cache.replenish(
            builtin.program_id,
            Arc::new(ProgramCacheEntry::new_builtin(
                0u64,
                builtin.name.len(),
                builtin.entrypoint,
            )),
        );
    }

    cache
}
