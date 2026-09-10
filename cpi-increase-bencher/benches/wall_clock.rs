//! Nested CPI wall clock time bench.

use {
    cpi_increase_bencher::{
        accounts::{DEFAULT_ACCOUNT_DATA_LEN, program_elf},
        harness::Harness,
        scenario::Scenario,
    },
    criterion::{Criterion, criterion_group, criterion_main},
    std::time::Duration,
};

fn scenarios() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        // 64 instructions, no CPI
        ("64x0", vec![0; 64]),
        // 32 instructions, 1 CPI each
        ("32x1", vec![1; 32]),
        // 12 instructions, 4 CPI, then 1 instruction, 3 CPI
        ("12x4_1x3", [vec![4; 12], vec![3]].concat()),
        // 7 instructions, 8 CPI, then 1 instruction, 0 CPI
        ("7x8_1x0", [vec![8; 7], vec![0]].concat()),
    ]
}

fn bench_frames(c: &mut Criterion) {
    let elf = program_elf();
    let mut group = c.benchmark_group("frames");
    for account_data_len in [
        DEFAULT_ACCOUNT_DATA_LEN,
        DEFAULT_ACCOUNT_DATA_LEN * 10,
        DEFAULT_ACCOUNT_DATA_LEN * 100,
    ] {
        for (name, nesting_levels) in scenarios() {
            for direct_mapping in [true, false] {
                let scenario = Scenario {
                    nesting_levels: nesting_levels.clone(),
                    direct_mapping,
                    track_memory: false, // Just wall clock time for `cargo bench`
                    account_data_len,
                };
                assert_eq!(
                    scenario.frames(),
                    64, // MAX_CALL_DEPTH
                    "{name} does not fill the frame limit"
                );
                let id = format!(
                    "{name}/{}KiB/dm_{}",
                    account_data_len / DEFAULT_ACCOUNT_DATA_LEN,
                    if direct_mapping { "on" } else { "off" }
                );
                let mut harness = Harness::new(&elf, &scenario.feature_set());
                group.bench_function(&id, |b| {
                    b.iter_custom(|iters| {
                        (0..iters)
                            .map(|_| harness.measure(&scenario).elapsed)
                            .sum::<Duration>()
                    })
                });
            }
        }
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(30).measurement_time(Duration::from_secs(3));
    targets = bench_frames
}
criterion_main!(benches);
