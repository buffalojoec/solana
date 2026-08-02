use {
    criterion::{Criterion, Throughput, criterion_group, criterion_main},
    solana_program_runtime::{
        loaded_programs::ProgramRuntimeEnvironment, program_cache_entry::ProgramCacheEntry,
        program_metrics::LoadProgramMetrics,
    },
    solana_program_runtime_bench::{
        DEPLOYMENT_SLOT, program::programs, program_runtime_environment,
    },
    solana_sdk_ids::bpf_loader_upgradeable,
};

fn bench_program_load(c: &mut Criterion) {
    let program_runtime_environment = program_runtime_environment();

    for program in programs() {
        let mut group = c.benchmark_group(&program.name);
        group.throughput(Throughput::Bytes(program.elf.len() as u64));
        group.bench_function("new", |b| {
            // Entries are dropped outside of the timed section, since the
            // validator holds on to them rather than tearing them down.
            b.iter_with_large_drop(|| {
                ProgramCacheEntry::new(
                    &bpf_loader_upgradeable::id(),
                    ProgramRuntimeEnvironment::clone(&program_runtime_environment),
                    DEPLOYMENT_SLOT,
                    &program.elf,
                    &mut LoadProgramMetrics::default(),
                )
                .unwrap()
            })
        });
        group.bench_function("reload", |b| {
            // Verifies the ELF against this environment, which is the
            // precondition `reload` assumes has already been met.
            ProgramCacheEntry::new(
                &bpf_loader_upgradeable::id(),
                ProgramRuntimeEnvironment::clone(&program_runtime_environment),
                DEPLOYMENT_SLOT,
                &program.elf,
                &mut LoadProgramMetrics::default(),
            )
            .unwrap();
            b.iter_with_large_drop(|| {
                // SAFETY: The executable has been verified just above, against
                // the same environment.
                unsafe {
                    ProgramCacheEntry::reload(
                        &bpf_loader_upgradeable::id(),
                        ProgramRuntimeEnvironment::clone(&program_runtime_environment),
                        DEPLOYMENT_SLOT,
                        &program.elf,
                        &mut LoadProgramMetrics::default(),
                    )
                    .unwrap()
                }
            })
        });
        group.finish();
    }
}

criterion_group!(benches, bench_program_load);
criterion_main!(benches);
