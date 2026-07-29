#![allow(clippy::arithmetic_side_effects)]
//! Benchmarks for the CPI pipeline.

use {
    criterion::{
        BenchmarkGroup, BenchmarkId, Criterion, criterion_group, criterion_main,
        measurement::WallTime,
    },
    solana_account::{Account, AccountSharedData, WritableAccount},
    solana_instruction::Instruction,
    solana_program_runtime::{
        cpi::{
            SolAccountInfo, SolAccountMeta, SolInstruction, SyscallInvokeSigned, TranslatedAccount,
            cpi_common, translate_accounts_c, translate_instruction_c,
        },
        declare_process_instruction,
        invoke_context::{BpfAllocator, InvokeContext},
        memory_context::{MemoryContext, SerializedAccountMetadata},
        program_cache_entry::ProgramCacheEntry,
        serialization::serialize_parameters,
        solana_sbpf::program::BuiltinFunctionDefinition,
        with_mock_invoke_context_with_feature_set,
    },
    solana_pubkey::Pubkey,
    solana_sbpf::{
        ebpf::MM_STACK_START,
        memory_region::{MemoryMapping, MemoryRegion},
        program::SBPFVersion,
        vm::Config,
    },
    solana_sdk_ids::{bpf_loader, native_loader},
    solana_svm_feature_set::SVMFeatureSet,
    solana_transaction_context::{
        IndexOfAccount, MAX_ACCOUNT_DATA_LEN, MAX_ACCOUNTS_PER_TRANSACTION,
        MAX_INSTRUCTION_DATA_LEN, instruction_accounts::InstructionAccount,
    },
    std::{
        mem, ptr,
        sync::Arc,
        time::{Duration, Instant},
    },
};

#[cfg(not(any(target_env = "msvc", target_os = "freebsd")))]
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

struct InvokeSignedC;

impl SyscallInvokeSigned for InvokeSignedC {
    fn translate_instruction(
        addr: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Instruction, Box<dyn std::error::Error>> {
        translate_instruction_c(addr, invoke_context)
    }

    fn translate_accounts<'a>(
        account_infos_addr: u64,
        account_infos_len: u64,
        invoke_context: &InvokeContext,
    ) -> Result<Vec<TranslatedAccount<'a>>, Box<dyn std::error::Error>> {
        translate_accounts_c(account_infos_addr, account_infos_len, invoke_context)
    }
}

/// Most data accounts a CPI can be handed here.
///
/// The CPI limit is `MAX_ACCOUNTS_PER_INSTRUCTION` (255), but the transaction
/// also holds the caller and callee program accounts, so the reachable maximum
/// is two below `MAX_ACCOUNTS_PER_TRANSACTION`.
const MAX_DATA_ACCOUNTS: usize = MAX_ACCOUNTS_PER_TRANSACTION - 2;

declare_process_instruction!(NoopCallee, 1, |_invoke_context| { Ok(()) });

fn reserve(cursor: &mut usize, len: usize, align: usize) -> usize {
    let offset = cursor.next_multiple_of(align);
    *cursor = offset + len;
    offset
}

/// The arguments a program builds on its own stack before invoking the CPI
/// syscall.
struct CallerStack {
    buffer: Vec<u8>,
    instruction_addr: u64,
    account_infos_addr: u64,
    account_infos_len: u64,
}

impl CallerStack {
    fn new(
        callee_program_id: &Pubkey,
        accounts: &[SerializedAccountMetadata],
        instruction_data: &[u8],
    ) -> Self {
        let num_accounts = accounts.len();
        let mut cursor = 0usize;

        let o_instruction = reserve(&mut cursor, mem::size_of::<SolInstruction>(), 8);
        let o_metas = reserve(
            &mut cursor,
            num_accounts * mem::size_of::<SolAccountMeta>(),
            8,
        );
        let o_infos = reserve(
            &mut cursor,
            num_accounts * mem::size_of::<SolAccountInfo>(),
            8,
        );
        let o_program = reserve(&mut cursor, mem::size_of::<Pubkey>(), 1);
        let o_data = reserve(&mut cursor, instruction_data.len(), 1);

        let mut buffer = vec![0u8; cursor];
        let addr = |offset: usize| MM_STACK_START + offset as u64;

        unsafe {
            let base = buffer.as_mut_ptr();
            ptr::write_unaligned(
                base.add(o_instruction).cast(),
                SolInstruction {
                    program_id_addr: addr(o_program),
                    accounts_addr: addr(o_metas),
                    accounts_len: num_accounts as u64,
                    data_addr: addr(o_data),
                    data_len: instruction_data.len() as u64,
                },
            );
            ptr::write_unaligned(base.add(o_program).cast(), *callee_program_id);
            ptr::copy_nonoverlapping(
                instruction_data.as_ptr(),
                base.add(o_data),
                instruction_data.len(),
            );

            for (index, metadata) in accounts.iter().enumerate() {
                ptr::write_unaligned(
                    base.add(o_metas + index * mem::size_of::<SolAccountMeta>())
                        .cast(),
                    SolAccountMeta {
                        pubkey_addr: metadata.vm_key_addr,
                        is_writable: true,
                        is_signer: false,
                    },
                );
                ptr::write_unaligned(
                    base.add(o_infos + index * mem::size_of::<SolAccountInfo>())
                        .cast(),
                    SolAccountInfo {
                        key_addr: metadata.vm_key_addr,
                        lamports_addr: metadata.vm_lamports_addr,
                        data_len: metadata.original_data_len as u64,
                        data_addr: metadata.vm_data_addr,
                        owner_addr: metadata.vm_owner_addr,
                        rent_epoch: 0,
                        is_signer: false,
                        is_writable: true,
                        executable: false,
                    },
                );
            }
        }

        Self {
            buffer,
            instruction_addr: addr(o_instruction),
            account_infos_addr: addr(o_infos),
            account_infos_len: num_accounts as u64,
        }
    }

    fn region(&mut self) -> MemoryRegion {
        MemoryRegion::new(&raw mut self.buffer[..], MM_STACK_START)
    }
}

/// The transaction accounts backing a CPI: the caller program, the callee
/// program, then `num_accounts` writable data accounts.
fn setup_accounts(
    num_accounts: usize,
    account_data_len: usize,
) -> Vec<(Pubkey, AccountSharedData)> {
    let mut caller_program = AccountSharedData::new(0, 0, &bpf_loader::id());
    caller_program.set_executable(true);
    let mut callee_program = AccountSharedData::new(0, 0, &native_loader::id());
    callee_program.set_executable(true);

    let mut transaction_accounts = vec![
        (Pubkey::new_unique(), caller_program),
        (Pubkey::new_unique(), callee_program),
    ];
    for _ in 0..num_accounts {
        transaction_accounts.push((
            Pubkey::new_unique(),
            AccountSharedData::from(Account {
                lamports: 1,
                data: vec![0u8; account_data_len],
                owner: Pubkey::new_unique(),
                executable: false,
                rent_epoch: 0,
            }),
        ));
    }
    transaction_accounts
}

/// Build up the memory regions for the caller, then time one CPI.
fn time_one_cpi(
    transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
    num_accounts: usize,
    instruction_data: &[u8],
    feature_set: &SVMFeatureSet,
) -> Duration {
    let callee_program_id = transaction_accounts[1].0;

    let instruction_accounts = (1..2 + num_accounts as IndexOfAccount)
        .map(|index| InstructionAccount::new(index, false, index > 1))
        .collect::<Vec<_>>();

    with_mock_invoke_context_with_feature_set!(
        invoke_context,
        transaction_context,
        feature_set,
        transaction_accounts,
    );

    let mut program_cache_for_tx_batch = ProgramCacheForTxBatch::default();
    program_cache_for_tx_batch.replenish(
        callee_program_id,
        Arc::new(ProgramCacheEntry::new_builtin(0, NoopCallee::register)),
    );
    invoke_context.program_cache_for_tx_batch = &mut program_cache_for_tx_batch;

    invoke_context
        .transaction_context
        .configure_top_level_instruction_for_tests(0, instruction_accounts, vec![])
        .unwrap();
    invoke_context.push().unwrap();

    let (_input_memory, input_regions, account_metadata, _data_offset) = {
        let instruction_context = invoke_context
            .transaction_context
            .get_current_instruction_context()
            .unwrap();
        serialize_parameters(
            &instruction_context,
            feature_set.virtual_address_space_adjustments,
            feature_set.account_data_direct_mapping,
            feature_set.direct_account_pointers_in_program_input,
        )
        .unwrap()
    };

    let mut stack = CallerStack::new(&callee_program_id, &account_metadata[1..], instruction_data);

    let config = Config {
        aligned_memory_mapping: false,
        ..Config::default()
    };
    let regions = std::iter::once(stack.region())
        .chain(input_regions)
        .collect::<Vec<_>>();
    let memory_mapping = unsafe { MemoryMapping::new(regions, &config, SBPFVersion::V3).unwrap() };
    invoke_context
        .memory_contexts
        .set_memory_context_abi_v1(MemoryContext::new(
            BpfAllocator::new(solana_program_entrypoint::HEAP_LENGTH as u64),
            account_metadata,
            memory_mapping,
        ))
        .unwrap();

    let start = Instant::now();
    let result = cpi_common::<InvokeSignedC>(
        &mut invoke_context,
        stack.instruction_addr,
        stack.account_infos_addr,
        stack.account_infos_len,
        0,
        0,
    );
    let elapsed = start.elapsed();

    if let Err(error) = result {
        if let Some(log_collector) = invoke_context.get_log_collector() {
            for message in log_collector.borrow().get_recorded_content() {
                eprintln!("log: {message}");
            }
        }
        panic!("cpi failed: {error:?}");
    }
    elapsed
}

fn bench_point(
    group: &mut BenchmarkGroup<WallTime>,
    num_accounts: usize,
    account_data_len: usize,
    instruction_data_len: usize,
    parameter: usize,
) {
    let feature_set = SVMFeatureSet::all_enabled();
    let transaction_accounts = setup_accounts(num_accounts, account_data_len);
    let instruction_data = vec![0u8; instruction_data_len];

    group.bench_function(BenchmarkId::from_parameter(parameter), |b| {
        b.iter_custom(|iters| {
            (0..iters)
                .map(|_| {
                    time_one_cpi(
                        transaction_accounts.clone(),
                        num_accounts,
                        &instruction_data,
                        &feature_set,
                    )
                })
                .sum()
        })
    });
}

/// Accounts passed to the CPI, at a fixed small account data size.
fn bench_account_count(c: &mut Criterion) {
    const ACCOUNT_DATA_LEN: usize = 32;
    let mut group = c.benchmark_group("cpi_account_count");
    for num_accounts in [1, 8, 32, 128, MAX_DATA_ACCOUNTS] {
        bench_point(&mut group, num_accounts, ACCOUNT_DATA_LEN, 0, num_accounts);
    }
}

/// Account data size, at a fixed account count. Drives the caller/callee sync.
fn bench_account_data_len(c: &mut Criterion) {
    const NUM_ACCOUNTS: usize = 1;
    let mut group = c.benchmark_group("cpi_account_data_len");
    for account_data_len in [0, 1024, 65536, 1024 * 1024, MAX_ACCOUNT_DATA_LEN as usize] {
        bench_point(
            &mut group,
            NUM_ACCOUNTS,
            account_data_len,
            0,
            account_data_len,
        );
    }
}

/// Instruction data length, which only drives `translate_instruction`.
fn bench_instruction_data_len(c: &mut Criterion) {
    const NUM_ACCOUNTS: usize = 1;
    const ACCOUNT_DATA_LEN: usize = 32;
    let mut group = c.benchmark_group("cpi_instruction_data_len");
    for instruction_data_len in [0, 128, 1024, MAX_INSTRUCTION_DATA_LEN] {
        bench_point(
            &mut group,
            NUM_ACCOUNTS,
            ACCOUNT_DATA_LEN,
            instruction_data_len,
            instruction_data_len,
        );
    }
}

criterion_group!(
    benches,
    bench_account_count,
    bench_account_data_len,
    bench_instruction_data_len
);
criterion_main!(benches);
