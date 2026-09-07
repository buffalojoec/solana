//! Conformance testing between runners.

use solana_program_cache_harness::{LoadResult, Op, Scenario, run_both, tree};

#[test]
fn the_runners_agree_on_a_deployment_and_two_batches() {
    let extract = || Op::Extract {
        programs: vec![0],
        fork_tip: 2,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: Vec::new(),
        ops: vec![
            Op::Deploy { program: 0, at: 1 },
            extract(),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            extract(),
        ],
    };

    run_both(&scenario).assert_clean();
}
