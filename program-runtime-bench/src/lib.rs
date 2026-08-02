//! Setup shared by the program runtime benchmarks in `benches/`.

pub mod program;

use {
    solana_clock::Slot, solana_compute_budget::compute_budget::ComputeBudget,
    solana_program_runtime::loaded_programs::ProgramRuntimeEnvironment,
    solana_svm_feature_set::SVMFeatureSet, solana_syscalls::create_program_runtime_environment,
};

pub const DEPLOYMENT_SLOT: Slot = 0;

pub fn program_runtime_environment() -> ProgramRuntimeEnvironment {
    let feature_set = SVMFeatureSet::all_enabled();
    let compute_budget = ComputeBudget::new_with_defaults(feature_set.raise_cpi_nesting_limit_to_8);
    create_program_runtime_environment(
        &feature_set,
        &compute_budget.to_budget(),
        /* reject_deployment_of_broken_elfs */ false,
        /* debugging_features */ false,
    )
    .expect("cannot create the program runtime environment")
}
