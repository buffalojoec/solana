//! Main harness for executing an Agave syscall using a Protobuf syscall
//! context.

use {
    crate::utils::{
        err_map::unpack_stable_result,
        vm::{mem_regions, HEAP_MAX, STACK_SIZE},
    },
    prost::Message,
    solana_account::AccountSharedData,
    solana_bpf_loader_program::{
        serialization::serialize_parameters, syscalls::create_program_runtime_environment_v1,
    },
    solana_compute_budget::compute_budget::ComputeBudget,
    solana_feature_set::bpf_account_data_direct_mapping,
    solana_log_collector::LogCollector,
    solana_program_runtime::{
        invoke_context::{EnvironmentConfig, InvokeContext},
        mem_pool::VmMemoryPool,
        solana_rbpf::{
            aligned_memory::AlignedMemory,
            ebpf::{self, HOST_ALIGN},
            memory_region::{MemoryMapping, MemoryRegion},
            program::{BuiltinProgram, SBPFVersion},
            vm::{ContextObject, EbpfVm},
        },
    },
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_sdk::transaction_context::{IndexOfAccount, TransactionAccount, TransactionContext},
    solana_svm_fuzz_harness_fixture::{
        invoke::context::InstrContext,
        proto::{SyscallContext as ProtoSyscallContext, SyscallEffects as ProtoSyscallEffects},
    },
    std::{cell::RefCell, ffi::c_int, sync::Arc},
};

/// Main harness for executing an Agave syscall using a Protobuf syscall
/// context.
///
/// Returns the VM's effects as a Protobuf instruction "effects".
pub fn execute_vm_syscall_proto(input: ProtoSyscallContext) -> Option<ProtoSyscallEffects> {
    let mut instr_ctx: InstrContext = input.instr_ctx?.try_into().ok()?;

    let existing_pubkeys: Vec<_> = instr_ctx
        .accounts
        .iter()
        .map(|(pubkey, _)| pubkey)
        .collect();

    if !existing_pubkeys.contains(&&instr_ctx.program_id) {
        instr_ctx
            .accounts
            .push((instr_ctx.program_id, AccountSharedData::default().into()));
    }

    let feature_set = instr_ctx.epoch_context.feature_set;

    let program_runtime_environment_v1 =
        create_program_runtime_environment_v1(&feature_set, &ComputeBudget::default(), true, false)
            .unwrap();
    let config = program_runtime_environment_v1.get_config();

    // Create invoke context
    // TODO: factor this into common code with lib.rs
    let mut transaction_accounts =
        Vec::<TransactionAccount>::with_capacity(instr_ctx.accounts.len().saturating_add(1));
    #[allow(deprecated)]
    instr_ctx
        .accounts
        .clone()
        .into_iter()
        .map(|(pubkey, account)| (pubkey, AccountSharedData::from(account)))
        .for_each(|x| transaction_accounts.push(x));

    let compute_budget = ComputeBudget {
        compute_unit_limit: instr_ctx.compute_units_available,
        ..ComputeBudget::default()
    };
    let mut transaction_context = TransactionContext::new(
        transaction_accounts.clone(),
        Rent::default(),
        compute_budget.max_instruction_stack_depth,
        compute_budget.max_instruction_trace_length,
    );

    if let Some(vm_ctx) = &input.vm_ctx {
        if let Some(return_data) = vm_ctx.return_data.clone() {
            let program_id = Pubkey::try_from(return_data.program_id).unwrap();
            transaction_context
                .set_return_data(program_id, return_data.data)
                .unwrap();
        }
    }

    let mut program_cache = solana_svm_fuzz_harness_instr::program_cache::setup_program_cache(
        &feature_set,
        &compute_budget,
        instr_ctx.slot_context.slot,
    );

    let sysvar_cache =
        solana_svm_fuzz_harness_instr::sysvar_cache::setup_sysvar_cache(&instr_ctx.accounts);

    #[allow(deprecated)]
    let (blockhash, lamports_per_signature) = sysvar_cache
        .get_recent_blockhashes()
        .ok()
        .and_then(|x| (*x).last().cloned())
        .map(|x| (x.blockhash, x.fee_calculator.lamports_per_signature))
        .unwrap_or_default();

    let environment_config = EnvironmentConfig::new(
        blockhash,
        None,
        None,
        Arc::new(feature_set.clone()),
        lamports_per_signature,
        &sysvar_cache,
    );
    let log_collector = LogCollector::new_ref();

    let invoke_context = RefCell::new(InvokeContext::new(
        &mut transaction_context,
        &mut program_cache,
        environment_config,
        Some(log_collector.clone()),
        compute_budget,
    ));

    let instr_accounts =
        solana_svm_fuzz_harness_instr::build_instruction_accounts(&instr_ctx.instruction_accounts);

    let program_idx_in_txn = transaction_accounts
        .iter()
        .position(|(pubkey, _)| *pubkey == instr_ctx.program_id)?
        as IndexOfAccount;

    let mut invoke_ctx: std::cell::RefMut<'_, InvokeContext<'_>> = invoke_context.borrow_mut();
    let direct_mapping = invoke_ctx
        .get_feature_set()
        .is_active(&bpf_account_data_direct_mapping::id());

    invoke_ctx
        .transaction_context
        .get_next_instruction_context()
        .unwrap()
        .configure(
            &[program_idx_in_txn],
            instr_accounts.as_slice(),
            &instr_ctx.instruction_data,
        );
    drop(invoke_ctx);

    let mut invoke_ctx = invoke_context.borrow_mut();

    match invoke_ctx.push() {
        Ok(_) => (),
        Err(_) => eprintln!("Failed to push invoke context"),
    }
    drop(invoke_ctx);

    let invoke_ctx = invoke_context.borrow_mut();
    let caller_instr_ctx = invoke_ctx
        .transaction_context
        .get_current_instruction_context()
        .unwrap();
    let (_aligned_memory, input_memory_regions, acc_metadatas) = serialize_parameters(
        invoke_ctx.transaction_context,
        caller_instr_ctx,
        !direct_mapping,
    )
    .unwrap();

    drop(invoke_ctx);

    let sbpf_version = SBPFVersion::V0;

    // Set up memory mapping
    let vm_ctx = input.vm_ctx.unwrap();
    // Follow FD harness behavior
    if vm_ctx.heap_max as usize > HEAP_MAX {
        return None;
    }

    // Memory regions.
    // In Agave all memory regions are AlignedMemory::<HOST_ALIGN> == AlignedMemory::<16>,
    // i.e. they're all 16-byte aligned in the host.
    // The memory regions are:
    //   1. program rodata
    //   2. stack
    //   3. heap
    //   4. input data aka accounts
    // The stack gap is size is 0 iff direct mapping is enabled.
    // There's some extra quirks:
    //   - heap size is MIN_HEAP_FRAME_BYTES..=MAX_HEAP_FRAME_BYTES
    //   - input data (at least when direct mapping is off) is 1 single map of all
    //     serialized accounts (and each account is serialized to a multiple of 16 bytes)
    // In this implementation, however:
    //   - heap can be smaller than MIN_HEAP_FRAME_BYTES
    //   - input data is made of multiple regions, and regions don't necessarily have
    //     length multiple of 16, i.e. virtual addresses may be unaligned
    // These differences allow us to test more edge cases.
    let mut mempool = VmMemoryPool::new();
    let rodata = AlignedMemory::<HOST_ALIGN>::from(&vm_ctx.rodata);
    let mut stack = mempool.get_stack(STACK_SIZE);
    // let mut heap = mempool.get_heap(heap_max); // this would force MIN_HEAP_FRAME_BYTES
    let mut heap = AlignedMemory::<HOST_ALIGN>::from(&vec![0; vm_ctx.heap_max as usize]);
    let rodata_stack_heap = vec![
        MemoryRegion::new_readonly(rodata.as_slice(), ebpf::MM_RODATA_START),
        MemoryRegion::new_writable_gapped(
            stack.as_slice_mut(),
            ebpf::MM_STACK_START,
            if !sbpf_version.dynamic_stack_frames() && config.enable_stack_frame_gaps {
                config.stack_frame_size as u64
            } else {
                0
            },
        ),
        MemoryRegion::new_writable(heap.as_slice_mut(), ebpf::MM_HEAP_START),
    ];
    let regions = rodata_stack_heap
        .into_iter()
        .chain(input_memory_regions)
        .collect();

    let Ok(memory_mapping) = MemoryMapping::new(regions, config, sbpf_version) else {
        return None;
    };

    let mut invoke_ctx = invoke_context.borrow_mut();

    invoke_ctx
        .set_syscall_context(solana_program_runtime::invoke_context::SyscallContext {
            allocator: solana_program_runtime::invoke_context::BpfAllocator::new(vm_ctx.heap_max),
            accounts_metadata: acc_metadatas, // TODO: accounts metadata for direct mapping support
            trace_log: Vec::new(),
        })
        .unwrap();

    // Set up the vm instance
    let loader = std::sync::Arc::new(BuiltinProgram::new_mock());
    let mut vm = EbpfVm::new(
        loader,
        sbpf_version,
        &mut *invoke_ctx,
        memory_mapping,
        STACK_SIZE,
    );
    vm.registers[0] = vm_ctx.r0;
    vm.registers[1] = vm_ctx.r1;
    vm.registers[2] = vm_ctx.r2;
    vm.registers[3] = vm_ctx.r3;
    vm.registers[4] = vm_ctx.r4;
    vm.registers[5] = vm_ctx.r5;
    vm.registers[6] = vm_ctx.r6;
    vm.registers[7] = vm_ctx.r7;
    vm.registers[8] = vm_ctx.r8;
    vm.registers[9] = vm_ctx.r9;
    vm.registers[10] = vm_ctx.r10;
    vm.registers[11] = vm_ctx.r11;

    if let Some(syscall_invocation) = input.syscall_invocation.clone() {
        mem_regions::copy_memory_prefix(heap.as_slice_mut(), &syscall_invocation.heap_prefix);
        mem_regions::copy_memory_prefix(stack.as_slice_mut(), &syscall_invocation.stack_prefix);
    }

    // Actually invoke the syscall

    // Invoke the syscall
    let (_, syscall_func) = program_runtime_environment_v1
        .get_function_registry(sbpf_version)
        .lookup_by_name(&input.syscall_invocation?.function_name)?;
    vm.invoke_function(syscall_func);

    // Unwrap and return the effects of the syscall
    let program_id = instr_ctx.program_id;
    let program_result = vm.program_result;
    let (error, error_kind, r0) =
        unpack_stable_result(program_result, vm.context_object_pointer, &program_id);
    Some(ProtoSyscallEffects {
        // Register 0 doesn't seem to contain the result, maybe we're missing some code from agave.
        // Regardless, the result is available in vm.program_result, so we can return it from there.
        r0,
        // Registers are only for vm_interp
        r1: 0,
        r2: 0,
        r3: 0,
        r4: 0,
        r5: 0,
        r6: 0,
        r7: 0,
        r8: 0,
        r9: 0,
        r10: 0,
        cu_avail: vm.context_object_pointer.get_remaining(),
        heap: heap.as_slice().into(),
        stack: stack.as_slice().into(),
        input_data_regions: mem_regions::extract_input_data_regions(&vm.memory_mapping),
        inputdata: vec![], // deprecated
        rodata: rodata.as_slice().into(),
        frame_count: vm.call_depth,
        error,
        error_kind: error_kind as i32,
        log: invoke_ctx
            .get_log_collector()?
            .borrow()
            .get_recorded_content()
            .join("\n")
            .into_bytes(),
        pc: 0,
    })
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn sol_compat_vm_syscall_execute_v1(
    out_ptr: *mut u8,
    out_psz: *mut u64,
    in_ptr: *mut u8,
    in_sz: u64,
) -> c_int {
    let in_slice = std::slice::from_raw_parts(in_ptr, in_sz as usize);
    let Ok(syscall_context) = ProtoSyscallContext::decode(in_slice) else {
        return 0;
    };

    let Some(syscall_effects) = execute_vm_syscall_proto(syscall_context) else {
        return 0;
    };
    let out_slice = std::slice::from_raw_parts_mut(out_ptr, (*out_psz) as usize);
    let out_vec = syscall_effects.encode_to_vec();
    if out_vec.len() > out_slice.len() {
        return 0;
    }
    out_slice[..out_vec.len()].copy_from_slice(&out_vec);
    *out_psz = out_vec.len() as u64;

    1
}
