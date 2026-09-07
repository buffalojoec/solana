//! Fuzzer throughput (exec/s).

#![allow(clippy::arithmetic_side_effects)]
use {
    criterion::{
        BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime,
    },
    solana_program_cache_harness::{
        ForkTree, LoadResult, Op, Owner, Runner, Scenario, Seed, V1, V2, run,
    },
    std::hint::black_box,
};

const OPS_PER_ROUND: usize = 9;

fn tree_of(slots: u64) -> ForkTree {
    let mut tree = ForkTree::default();
    if slots > 0 {
        tree.insert_fork(&(1..=slots).collect::<Vec<u64>>());
    }
    tree
}

fn scenario_of(slots: u64, rounds: usize) -> Scenario {
    let tip = slots.max(1);
    let mut ops = Vec::new();
    for _ in 0..rounds {
        for program in 0..3 {
            ops.push(Op::Deploy { program, at: 1 });
        }
        ops.push(Op::Extract {
            programs: vec![0, 1, 2],
            fork_tip: tip,
        });
        for program in 0..3 {
            ops.push(Op::FinishLoad {
                program,
                result: LoadResult::Loaded,
            });
        }
        ops.push(Op::Extract {
            programs: vec![0, 1, 2],
            fork_tip: tip,
        });
        ops.push(Op::Prune { root: tip });
    }
    assert_eq!(ops.len(), rounds * OPS_PER_ROUND);
    Scenario {
        tree: tree_of(slots),
        seeds: vec![
            Seed {
                program: 0,
                owner: Owner::LoaderV3,
                verifies: true,
            },
            Seed {
                program: 1,
                owner: Owner::LoaderV3,
                verifies: true,
            },
            Seed {
                program: 2,
                owner: Owner::LoaderV3,
                verifies: true,
            },
        ],
        ops,
    }
}

fn variable_slots<R: Runner>(group: &mut BenchmarkGroup<'_, WallTime>, runner: &str) {
    for slots in [2, 8, 24, 48] {
        let scenario = scenario_of(slots, 1);
        group.bench_function(format!("variable-slots/{runner}/{slots}-slots"), |b| {
            b.iter(|| black_box(run::<R>(black_box(&scenario))));
        });
    }
}

fn variable_ops<R: Runner>(group: &mut BenchmarkGroup<'_, WallTime>, runner: &str) {
    for rounds in [1, 3, 5] {
        let scenario = scenario_of(8, rounds);
        let num_ops = rounds * OPS_PER_ROUND;
        group.bench_function(format!("variable-ops/{runner}/{num_ops}-ops"), |b| {
            b.iter(|| black_box(run::<R>(black_box(&scenario))));
        });
    }
}

fn throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    variable_slots::<V1>(&mut group, "v1");
    variable_slots::<V2>(&mut group, "v2");
    variable_ops::<V1>(&mut group, "v1");
    variable_ops::<V2>(&mut group, "v2");

    group.finish();
}

criterion_group!(benches, throughput);
criterion_main!(benches);
