//! Fuzzer throughput (exec/s).

#![allow(clippy::arithmetic_side_effects)]
use {
    criterion::{Criterion, criterion_group, criterion_main},
    solana_program_cache_harness::{
        ForkTree, Forks, LoadResult, Op, Owner, Scenario, Seed, V1, run,
    },
    std::hint::black_box,
};

const OPS_PER_ROUND: usize = 9;

// Axis 1: Number of nodes in the tree.
fn tree_of(slots: u64) -> ForkTree {
    let mut tree = ForkTree::default();
    if slots > 0 {
        tree.insert_fork(&(1..=slots).collect::<Vec<u64>>());
    }
    tree
}

// Axis 2: Number of ops in the scenario.
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

fn throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    // Fixed tree, varying ops.
    for rounds in [1, 3, 5] {
        let scenario = scenario_of(8, rounds);
        let num_ops = rounds * OPS_PER_ROUND;
        group.bench_function(format!("variable-ops/8-slots/{num_ops}-ops"), |b| {
            b.iter(|| black_box(run::<V1>(black_box(&scenario))));
        });
    }

    // Fixed ops, varying tree.
    for slots in [2, 8, 24, 64] {
        let scenario = scenario_of(slots, 1);
        group.bench_function(format!("variable-forks/{slots}-slots"), |b| {
            b.iter(|| black_box(Forks::new(black_box(&scenario.tree))));
        });
    }

    group.finish();
}

criterion_group!(benches, throughput);
criterion_main!(benches);
