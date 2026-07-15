//! Program deployment functionality.

#[cfg(feature = "metrics")]
use {crate::program_metrics::LoadProgramMetrics, solana_svm_measure::measure::Measure};
use {
    crate::{
        invoke_context::InvokeContext,
        loaded_programs::ProgramRuntimeEnvironment,
        program_cache_entry::{DELAY_VISIBILITY_SLOT_OFFSET, ProgramCacheEntry},
    },
    solana_clock::Slot,
    solana_instruction::error::InstructionError,
    solana_pubkey::Pubkey,
    solana_sbpf::{
        elf::{ElfError, Executable},
        program::{BuiltinProgram, SBPFVersion},
        verifier::RequisiteVerifier,
    },
    solana_svm_log_collector::{LogCollector, ic_logger_msg},
    solana_svm_type_overrides::sync::Arc,
    solana_transaction_context::IndexOfAccount,
    std::{cell::RefCell, rc::Rc},
};

fn morph_into_deployment_environment(
    from: ProgramRuntimeEnvironment,
    disable_sbpf_v0_v1_v2_deployment: bool,
) -> Result<BuiltinProgram<InvokeContext<'static, 'static>>, ElfError> {
    let mut config = (*from).get_config().clone();
    config.reject_broken_elfs = true;
    if disable_sbpf_v0_v1_v2_deployment {
        config.enabled_sbpf_versions = SBPFVersion::V3..=*config.enabled_sbpf_versions.end();
    }

    let mut result = BuiltinProgram::new_loader(config);

    for (_key, (name, value)) in (*from).get_function_registry().iter() {
        // Deployment of programs with sol_alloc_free is disabled. So do not register the syscall.
        if name != *b"sol_alloc_free_" {
            result.register_function(unsafe { std::str::from_utf8_unchecked(name) }, value)?;
        }
    }

    Ok(result)
}

/// The source of the program bits to deploy.
pub enum ProgramData<'a> {
    /// The data of one of the current instruction's accounts, from `offset` on.
    InstructionAccount {
        index: IndexOfAccount,
        offset: usize,
    },
    /// A caller provided buffer. Only for deploys driven by the runtime itself,
    /// which do not have any accounts to load from.
    Bytes(&'a [u8]),
}

/// Load `programdata`, verify it against the stricter deployment environment,
/// and reload it into a cache entry against the regular environment.
#[allow(clippy::too_many_arguments)]
fn load_and_verify_program(
    log_collector: &Option<Rc<RefCell<LogCollector>>>,
    #[cfg(feature = "metrics")] load_program_metrics: &mut LoadProgramMetrics,
    program_runtime_environment: ProgramRuntimeEnvironment,
    disable_sbpf_v0_v1_v2_deployment: bool,
    loader_key: &Pubkey,
    account_size: usize,
    programdata: &[u8],
    deployment_slot: Slot,
) -> Result<ProgramCacheEntry, InstructionError> {
    #[cfg(feature = "metrics")]
    let mut register_syscalls_time = Measure::start("register_syscalls_time");
    let deployment_program_runtime_environment = morph_into_deployment_environment(
        ProgramRuntimeEnvironment::clone(&program_runtime_environment),
        disable_sbpf_v0_v1_v2_deployment,
    )
    .map_err(|e| {
        ic_logger_msg!(log_collector, "Failed to register syscalls: {}", e);
        InstructionError::ProgramEnvironmentSetupFailure
    })?;
    #[cfg(feature = "metrics")]
    {
        register_syscalls_time.stop();
        load_program_metrics.register_syscalls_us = register_syscalls_time.as_us();
    }
    // Verify using stricter deployment_program_runtime_environment
    #[cfg(feature = "metrics")]
    let mut load_elf_time = Measure::start("load_elf_time");
    let executable = Executable::<InvokeContext>::load(
        programdata,
        Arc::new(deployment_program_runtime_environment),
    )
    .map_err(|err| {
        ic_logger_msg!(log_collector, "{}", err);
        InstructionError::InvalidAccountData
    })?;
    #[cfg(feature = "metrics")]
    {
        load_elf_time.stop();
        load_program_metrics.load_elf_us = load_elf_time.as_us();
    }
    #[cfg(feature = "metrics")]
    let mut verify_code_time = Measure::start("verify_code_time");
    executable.verify::<RequisiteVerifier>().map_err(|err| {
        ic_logger_msg!(log_collector, "{}", err);
        InstructionError::InvalidAccountData
    })?;
    #[cfg(feature = "metrics")]
    {
        verify_code_time.stop();
        load_program_metrics.verify_code_us = verify_code_time.as_us();
    }
    // Reload but with program_runtime_environment
    let executor = unsafe {
        // SAFETY: The executable has been verified just above.
        ProgramCacheEntry::reload(
            loader_key,
            program_runtime_environment,
            deployment_slot,
            deployment_slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET),
            programdata,
            account_size,
            #[cfg(feature = "metrics")]
            load_program_metrics,
        )
    }
    .map_err(|err| {
        ic_logger_msg!(log_collector, "{}", err);
        InstructionError::InvalidAccountData
    })?;
    Ok(executor)
}

/// Deploy a program, and store it in the cache for the transaction batch.
#[allow(clippy::too_many_arguments)]
pub fn deploy_program(
    invoke_context: &mut InvokeContext,
    program_id: &Pubkey,
    loader_key: &Pubkey,
    account_size: usize,
    programdata: ProgramData,
    deployment_slot: Slot,
    disable_sbpf_v0_v1_v2_deployment: bool,
) -> Result<(), InstructionError> {
    assert_eq!(
        deployment_slot,
        invoke_context.program_cache_for_tx_batch.slot()
    );
    let log_collector = invoke_context.get_log_collector();
    let program_runtime_environment = invoke_context
        .get_program_runtime_environment_for_deployment()
        .clone();
    #[cfg(feature = "metrics")]
    let mut load_program_metrics = LoadProgramMetrics::default();

    let load = |programdata: &[u8]| {
        load_and_verify_program(
            &log_collector,
            #[cfg(feature = "metrics")]
            &mut load_program_metrics,
            program_runtime_environment,
            disable_sbpf_v0_v1_v2_deployment,
            loader_key,
            account_size,
            programdata,
            deployment_slot,
        )
    };
    // The account borrow has to outlive the load, since the program bits are a
    // slice of the data it guards, and has to end before the cache is updated
    // below, since it borrows the invoke context.
    let executor = match programdata {
        ProgramData::Bytes(programdata) => load(programdata)?,
        ProgramData::InstructionAccount { index, offset } => {
            let instruction_context = invoke_context
                .transaction_context
                .get_current_instruction_context()?;
            let borrowed_account = instruction_context.try_borrow_instruction_account(index)?;
            let programdata = borrowed_account
                .get_data()
                .get(offset..)
                .ok_or(InstructionError::AccountDataTooSmall)?;
            load(programdata)?
        }
    };

    let program_cache_for_tx_batch = &mut invoke_context.program_cache_for_tx_batch;
    if let Some(old_entry) = program_cache_for_tx_batch.find(program_id) {
        executor.stats.merge_from(&old_entry.stats);
    }
    program_cache_for_tx_batch.store_modified_entry(*program_id, Arc::new(executor));
    #[cfg(feature = "metrics")]
    {
        load_program_metrics.program_id = program_id.to_string();
        load_program_metrics.submit_datapoint(&mut invoke_context.timings);
    }
    Ok(())
}
