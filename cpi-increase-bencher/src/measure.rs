use {
    crate::{accounts::accounts, harness::Harness, scenario::Scenario},
    solana_instruction::{AccountMeta, Instruction},
    solana_program_runtime::{
        __private::{Hash, TransactionContext},
        cpi::copied_bytes,
        invoke_context::{EnvironmentConfig, InvokeContext, mock_compile_message},
        memory_context::set_peak_tracking,
        sysvar_cache::SysvarCache,
    },
    solana_rent::Rent,
    solana_svm::conformance::{
        callback::DefaultCallback,
        setup::{compute_budget, program_runtime_environments},
    },
    solana_svm_timings::ExecuteTimings,
    std::time::{Duration, Instant},
};

pub struct Measurement {
    pub elapsed: Duration,
    pub compute_units: u64,
    pub peak_mapped_bytes: u64,
    pub copied_bytes: u64,
}

pub(crate) fn measure(harness: &mut Harness, scenario: &Scenario) -> Measurement {
    let program_id = harness.program_id;
    let accounts = accounts(&program_id, &harness.elf, scenario.account_data_len);
    let feature_set = scenario.feature_set();

    let metas = std::iter::once(AccountMeta::new_readonly(program_id, false))
        .chain(
            accounts[1..]
                .iter()
                .map(|(key, _)| AccountMeta::new(*key, false)),
        )
        .collect::<Vec<_>>();
    let instructions = scenario
        .nesting_levels
        .iter()
        .map(|level| Instruction::new_with_bytes(program_id, &[*level], metas.clone()))
        .collect::<Vec<_>>();

    let (message, transaction_accounts) = mock_compile_message(
        &instructions,
        &accounts,
        &program_id,
        &solana_sdk_ids::bpf_loader::id(),
    );

    let compute_budget = compute_budget(&feature_set);
    let environments = program_runtime_environments(&feature_set, &compute_budget);
    let mut sysvar_cache = SysvarCache::default();
    sysvar_cache.fill_missing_entries(|pubkey, callback| {
        for (key, account) in accounts.iter() {
            if key == pubkey {
                callback(&account.data);
            }
        }
    });

    let mut transaction_context = TransactionContext::new(
        transaction_accounts,
        Rent::default(),
        compute_budget.max_instruction_stack_depth,
        compute_budget.max_instruction_trace_length,
        instructions.len(),
    );

    let callback = DefaultCallback;
    let environment_config = EnvironmentConfig::new(
        Hash::default(),
        0,
        false,
        &callback,
        &feature_set,
        &environments,
        &sysvar_cache,
    );
    let mut invoke_context = InvokeContext::new(
        &mut transaction_context,
        &mut harness.program_cache,
        environment_config,
        None,
        compute_budget.to_budget(),
        compute_budget.to_cost(),
    );

    let mut timings = ExecuteTimings::default();
    let mut compute_units = 0u64;
    copied_bytes::take();
    set_peak_tracking(scenario.track_memory);

    let start = Instant::now();
    let result = invoke_context.process_message(&message, &mut timings, &mut compute_units);
    let elapsed = start.elapsed();

    result.unwrap_or_else(|(index, err)| panic!("instruction {index} failed: {err:?}"));

    Measurement {
        elapsed,
        compute_units,
        peak_mapped_bytes: invoke_context.memory_contexts.peak_mapped_bytes(),
        copied_bytes: copied_bytes::take(),
    }
}
