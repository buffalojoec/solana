// Mock syscalls for SVM tests to avoid dependency on agave-syscalls
#![allow(dead_code)]

use {
    solana_program_runtime::{
        execution_budget::SVMTransactionExecutionBudget,
        invoke_context::InvokeContext,
        solana_sbpf::{
            declare_builtin_function,
            memory_region::MemoryMapping,
            program::{BuiltinProgram, SBPFVersion},
            vm::Config,
        },
    },
    solana_svm_feature_set::SVMFeatureSet,
    std::result::Result,
};

// Minimal syscall implementations for tests
declare_builtin_function!(
    MockAbort,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple abort for tests
        Err("Program aborted".into())
    }
);

declare_builtin_function!(
    MockLog,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // No-op log for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockMemcpy,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _dst: u64,
        _src: u64,
        _n: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple memcpy stub for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockMemset,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _dst: u64,
        _val: u64,
        _n: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple memset stub for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockMemcmp,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple memcmp stub for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockMemmove,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _dst: u64,
        _src: u64,
        _n: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple memmove stub for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockInvokeSigned,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple invoke stub for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockSetReturnData,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple return data stub for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockGetClockSysvar,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple clock sysvar stub for tests
        Ok(0)
    }
);

declare_builtin_function!(
    MockGetRentSysvar,
    fn rust(
        _invoke_context: &mut InvokeContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Simple rent sysvar stub for tests
        Ok(0)
    }
);

/// Create a custom loader with mock syscalls for tests
pub fn create_custom_loader<'a>() -> BuiltinProgram<InvokeContext<'a>> {
    let compute_budget = SVMTransactionExecutionBudget::default();
    let vm_config = Config {
        max_call_depth: compute_budget.max_call_depth,
        stack_frame_size: compute_budget.stack_frame_size,
        enable_address_translation: true,
        enable_stack_frame_gaps: true,
        instruction_meter_checkpoint_distance: 10000,
        enable_instruction_meter: true,
        enable_instruction_tracing: true,
        enable_symbol_and_section_labels: true,
        reject_broken_elfs: true,
        noop_instruction_rate: 256,
        sanitize_user_provided_values: true,
        enabled_sbpf_versions: SBPFVersion::V0..=SBPFVersion::V3,
        optimize_rodata: false,
        aligned_memory_mapping: true,
    };

    let mut loader = BuiltinProgram::new_loader(vm_config);
    
    // Register minimal mock syscalls
    loader
        .register_function("abort", MockAbort::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_log_", MockLog::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_memcpy_", MockMemcpy::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_memset_", MockMemset::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_memcmp_", MockMemcmp::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_memmove_", MockMemmove::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_invoke_signed_rust", MockInvokeSigned::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_set_return_data", MockSetReturnData::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_get_clock_sysvar", MockGetClockSysvar::vm)
        .expect("Registration failed");
    loader
        .register_function("sol_get_rent_sysvar", MockGetRentSysvar::vm)
        .expect("Registration failed");
        
    loader
}

/// Create a minimal program runtime environment for tests
pub fn create_program_runtime_environment_v1<'a>(
    _feature_set: &SVMFeatureSet,
    compute_budget: &SVMTransactionExecutionBudget,
    _reject_deployment_of_broken_elfs: bool,
    _debugging_features: bool,
) -> Result<BuiltinProgram<InvokeContext<'a>>, Box<dyn std::error::Error>> {
    let vm_config = Config {
        max_call_depth: compute_budget.max_call_depth,
        stack_frame_size: compute_budget.stack_frame_size,
        enable_address_translation: true,
        enable_stack_frame_gaps: true,
        instruction_meter_checkpoint_distance: 10000,
        enable_instruction_meter: true,
        enable_instruction_tracing: true,
        enable_symbol_and_section_labels: true,
        reject_broken_elfs: true,
        noop_instruction_rate: 256,
        sanitize_user_provided_values: true,
        enabled_sbpf_versions: SBPFVersion::V0..=SBPFVersion::V3,
        optimize_rodata: false,
        aligned_memory_mapping: true,
    };

    // Create a basic loader without any syscalls for now
    // The conformance tests might add their own syscalls as needed
    Ok(BuiltinProgram::new_loader(vm_config))
}