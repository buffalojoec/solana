//! Known edge cases, written as scenarios.

use {
    solana_program_cache_harness::{
        EntryKind, LoadResult, Op, Owner, Runner, Scenario, V1, run_twice, tree,
    },
    std::marker::PhantomData,
    test_case::test_case,
};

/// Fork graph created for the test
///            1 - 2
///
/// A deployment lands unloaded, so the first batch to name it misses and is
/// handed the load. Once that load finishes the next batch is served it.
#[test_case(PhantomData::<V1>; "v1")]
fn sanity<R: Runner>(_: PhantomData<R>) {
    let extract = || Op::Extract {
        programs: vec![0],
        fork_tip: 2,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: 1,
                owner: Owner::LoaderV3,
                env: 0,
            },
            extract(),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            extract(),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [miss, hit] = report.extractions.as_slice() else {
        panic!("two extractions: {:?}", report.extractions);
    };
    assert_eq!(miss.asked_for, 1);
    assert!(!miss.hit, "the deployment is still unloaded");
    assert!(miss.started_load, "so the batch is handed the load");
    assert_eq!(hit.asked_for, 1);
    assert!(hit.hit, "the finished load is served");
    assert!(!hit.started_load, "and no second load is started");
}

/// Fork graph created for the test
///            1 - 2
///            |   |
///            |   `-- and is served the entry itself here
///            `------ a deployment is not visible in its own slot
///
/// A deployment only becomes effective one slot after it lands. A batch in
/// the deployment slot itself is handed a delay window tombstone instead -
/// which counts as a hit, since the cache did return something, but is not
/// the program and cannot be executed. No load is started for it either.
#[test_case(PhantomData::<V1>; "v1")]
fn a_deployment_is_not_visible_in_its_own_slot<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: 1,
                owner: Owner::LoaderV3,
                env: 0,
            },
            extract(1),
            extract(2),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            extract(2),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [delayed, miss, served] = report.extractions.as_slice() else {
        panic!("three extractions: {:?}", report.extractions);
    };
    assert_eq!(delayed.asked_for, 1);
    assert!(delayed.hit, "the cache does hand something back");
    assert_eq!(
        delayed.kind,
        Some(EntryKind::DelayVisibility),
        "but it is a tombstone, not the program"
    );
    assert!(!delayed.started_load, "and nothing is loaded for it");

    assert!(!miss.hit, "the next slot finds the deployment unloaded");
    assert!(miss.started_load);
    assert_eq!(
        served.kind,
        Some(EntryKind::Loaded),
        "and is served the program once it is loaded"
    );
}

/// Fork graph created for the test
///            1 - 2 - 3 - 4
///            |       |
///            |       `-- redeployment
///            `---------- deployment
///
/// A batch names the newest deployment its own account state holds, and
/// `extract` matches on that slot. So the loaded entry for the first
/// deployment cannot serve the second, and the redeployment costs one reload.
#[test_case(PhantomData::<V1>; "v1")]
fn a_redeployment_costs_one_reload<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy {
        program: 0,
        at,
        owner: Owner::LoaderV3,
        env: 0,
    };
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3, 4]]),
        ops: vec![
            deploy(1),
            extract(2),
            finish_load(),
            deploy(3),
            extract(4),
            finish_load(),
            extract(4),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [first, unloaded, reloaded] = report.extractions.as_slice() else {
        panic!("three extractions: {:?}", report.extractions);
    };
    assert_eq!(first.asked_for, 1);
    assert_eq!(unloaded.asked_for, 3, "the batch names the redeployment");
    assert!(!unloaded.hit, "which lands unloaded, like any deployment");
    assert!(unloaded.started_load, "so it is loaded now");
    assert_eq!(reloaded.asked_for, 3);
    assert!(reloaded.hit, "and the next batch is served it");
}

/// Fork graph created for the test
///            1 - 2
///
/// Three programs deployed in one slot, all named by one batch. `extract`
/// hands back at most one loading task per call, so every round loads one
/// more and the batch takes three of them to see all three programs.
#[test_case(PhantomData::<V1>; "v1")]
fn a_batch_is_handed_one_load_at_a_time<R: Runner>(_: PhantomData<R>) {
    let deploy = |program| Op::Deploy {
        program,
        at: 1,
        owner: Owner::LoaderV3,
        env: 0,
    };
    let extract = || Op::Extract {
        programs: vec![0, 1, 2],
        fork_tip: 2,
    };
    let finish_load = |program| Op::FinishLoad {
        program,
        result: LoadResult::Loaded,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        ops: vec![
            deploy(0),
            deploy(1),
            deploy(2),
            extract(),
            finish_load(0),
            extract(),
            finish_load(1),
            extract(),
            finish_load(2),
            extract(),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    assert_eq!(report.extractions.len(), 12, "four batches of three");
    for (round, extractions) in report.extractions.chunks(3).enumerate() {
        let hits = extractions.iter().filter(|entry| entry.hit).count();
        let tasks = extractions
            .iter()
            .filter(|entry| entry.started_load)
            .count();
        assert_eq!(hits, round, "one more program is loaded every round");
        assert_eq!(
            tasks,
            usize::from(round < 3),
            "and a batch is handed one load at a time"
        );
    }
}

/// Fork graph created for the test
///            1 - 2 - 3
///            |
///            `-- a deployment whose bytecode does not verify
///
/// A load which fails verification still produces an entry, and every later
/// batch is handed that rather than being asked to load the program again.
#[test_case(PhantomData::<V1>; "v1")]
fn a_load_which_fails_verification_is_not_retried<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3]]),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: 1,
                owner: Owner::LoaderV3,
                env: 0,
            },
            extract(2),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::FailedVerification,
            },
            extract(2),
            extract(3),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [miss, failed, later] = report.extractions.as_slice() else {
        panic!("three extractions: {:?}", report.extractions);
    };
    assert!(!miss.hit, "the deployment starts out unloaded");
    assert!(miss.started_load);
    assert_eq!(
        failed.kind,
        Some(EntryKind::FailedVerification),
        "the load produced a failure, and the cache kept it"
    );
    assert!(!failed.started_load, "so nothing reloads it");
    assert_eq!(
        later.kind,
        Some(EntryKind::FailedVerification),
        "a later slot is handed the same failure"
    );
    assert!(!later.started_load);
}

/// Fork graph created for the test
///                 1              <-- the only deployment
///        .-----.--+--.-----.
///        2     3     4     5
///        |     |     |     |
///        6     7     8     9     <-- every batch is served (1)
///
/// One deployment, on the slot every fork descends from. Whichever fork is
/// first to name it pays for the load, and every other fork is then served
/// that same entry - one version in the cache, not one per fork.
#[test_case(PhantomData::<V1>; "v1")]
fn one_version_serves_every_fork<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 6], &[1, 3, 7], &[1, 4, 8], &[1, 5, 9]]),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: 1,
                owner: Owner::LoaderV3,
                env: 0,
            },
            extract(6),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            extract(6),
            extract(7),
            extract(8),
            extract(9),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [first, rest @ ..] = report.extractions.as_slice() else {
        panic!("five extractions: {:?}", report.extractions);
    };
    assert_eq!(first.asked_for, 1);
    assert!(!first.hit, "the first fork to name it finds it unloaded");
    assert!(first.started_load, "and pays for the load");
    assert_eq!(rest.len(), 4);
    for served in rest {
        assert_eq!(served.asked_for, 1);
        assert!(served.hit, "every fork is served the same entry");
        assert!(!served.started_load, "and none of them reloads it");
    }

    assert_eq!(
        report.fingerprint.len(),
        1,
        "one version, not one per fork: {:?}",
        report.fingerprint
    );
}

/// Fork graph created for the test
///                1        <-- a deployment both forks can see
///               / \
///              2   3      <-- redeployment on (2) only
///              |   |
///              4   5      <-- a batch on (5) is still served (1)
///
/// Two versions of one program sit in the cache at once. The branch which
/// redeployed names the newer slot, and the sibling - whose account state
/// never saw that deployment - names the shared one and must be handed it.
#[test_case(PhantomData::<V1>; "v1")]
fn a_deployment_on_one_branch_does_not_reach_the_other<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy {
        program: 0,
        at,
        owner: Owner::LoaderV3,
        env: 0,
    };
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 4], &[1, 3, 5]]),
        ops: vec![
            deploy(1),
            extract(4),
            finish_load(),
            deploy(2),
            extract(4),
            finish_load(),
            extract(5),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [shared, branch, sibling] = report.extractions.as_slice() else {
        panic!("three extractions: {:?}", report.extractions);
    };
    assert_eq!(shared.asked_for, 1);
    assert_eq!(branch.asked_for, 2, "the branch names its own redeployment");
    assert_eq!(sibling.asked_for, 1, "the sibling never saw it");
    assert!(sibling.hit, "and is served the shared deployment");
    assert!(!sibling.started_load, "which was already loaded for it");
}

/// Fork graph created for the test
///            1 - 2 - 3 - 4 - 5
///            |       |   |
///            |       |   `-- root moves here
///            |       `------ redeployment
///            `-------------- deployment
///
/// Both deployments sit below the new root, so no fork can name the older one
/// again and only the newest is worth keeping. Prune drops the rest, and the
/// survivor is still served above the root without a reload.
#[test_case(PhantomData::<V1>; "v1")]
fn prune_keeps_only_the_newest_version_below_the_root<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy {
        program: 0,
        at,
        owner: Owner::LoaderV3,
        env: 0,
    };
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3, 4, 5]]),
        ops: vec![
            deploy(1),
            extract(2),
            finish_load(),
            deploy(3),
            extract(4),
            finish_load(),
            Op::Prune { root: 4 },
            extract(5),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    assert_eq!(
        report.fingerprint.len(),
        1,
        "only the newest version survives: {:?}",
        report.fingerprint
    );
    assert!(
        report.fingerprint[0].deployment_slot == 3,
        "and it is the redeployment: {:?}",
        report.fingerprint
    );

    let served = report.extractions.last().expect("an extraction");
    assert_eq!(served.asked_for, 3);
    assert!(served.hit, "the survivor is still served above the root");
    assert!(!served.started_load, "and the prune cost no reload");
}

/// Fork graph created for the test
///                1
///               / \
///              2   4      <-- the root moves to (2), abandoning (4)
///              |   |
///              3   5      <-- and nothing runs on (5) afterwards
///
/// Once the root moves to one branch, replay abandons the other: its banks are
/// gone from `BankForks`, so no batch executes there, nothing is deployed
/// there, and there is no block left to dump. Every op naming the abandoned
/// branch is ignored.
#[test_case(PhantomData::<V1>; "v1")]
fn a_fork_the_root_left_behind_is_never_worked_on<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0, 1],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3], &[1, 4, 5]]),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: 4,
                owner: Owner::LoaderV3,
                env: 0,
            },
            extract(5),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            extract(5),
            Op::Prune { root: 2 },
            extract(5),
            Op::Deploy {
                program: 1,
                at: 5,
                owner: Owner::LoaderV3,
                env: 0,
            },
            Op::PurgeSlot { slot: 5 },
            extract(5),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [miss, hit] = report.extractions.as_slice() else {
        panic!(
            "only the batches from before the root moved: {:?}",
            report.extractions
        );
    };
    assert_eq!(miss.asked_for, 4);
    assert!(miss.started_load, "the branch works while it is still live");
    assert_eq!(hit.kind, Some(EntryKind::Loaded));

    assert!(
        report.fingerprint.is_empty(),
        "and nothing it did afterwards landed: {:?}",
        report.fingerprint
    );
}

/// Fork graph created for the test
///                1
///               / \
///              2   4      <-- a load is asked for from (5), then the root
///              |   |          moves to (2) and the load finishes after
///              3   5
///
/// `set_root` drains nothing on the branches it leaves behind, so a load
/// started on one can finish after the prune which would have caught it, and
/// land as an orphan. Dumping the block cannot clean that up - there is no
/// bank left to dump - so the orphan stays.
#[test_case(PhantomData::<V1>; "v1")]
fn an_orphan_on_an_abandoned_fork_cannot_be_dumped<R: Runner>(_: PhantomData<R>) {
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3], &[1, 4, 5]]),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: 4,
                owner: Owner::LoaderV3,
                env: 0,
            },
            Op::Extract {
                programs: vec![0],
                fork_tip: 5,
            },
            Op::Prune { root: 2 },
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            Op::PurgeSlot { slot: 4 },
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [started] = report.extractions.as_slice() else {
        panic!(
            "one batch, from before the root moved: {:?}",
            report.extractions
        );
    };
    assert_eq!(started.asked_for, 4);
    assert!(started.started_load, "and it asked for the load");

    assert_eq!(
        report.fingerprint.len(),
        1,
        "the load landed after the prune: {:?}",
        report.fingerprint
    );
    assert!(
        report.fingerprint[0].deployment_slot == 4,
        "and the dump could not reach it: {:?}",
        report.fingerprint
    );
}

/// Fork graph created for the test
///                4        <-- a deployment both forks can see
///               / \
///              6   5      <-- redeployment, on the fork about to be doomed
///              |   |
///              8   7      <-- a batch on (7) asks for a load of (5)
///              |
///              9          <-- and a batch here names (4)
///
/// `set_root` drains nothing on the fork it leaves behind, so the load started
/// from (7) finishes after the prune which would have caught it and the orphan
/// lands at slot 5. Two things follow, and the scenario pins both.
///
/// `prune` keeps it: once the orphan's slot is behind the root it reads as
/// in-branch, so it holds memory and counts against `MAX_LOADED_ENTRY_COUNT`
/// until the root passes it. But nothing is ever served it - a caller names
/// the deployment its own account state holds, and slot 5 is not on its fork.
#[test_case(PhantomData::<V1>; "v1")]
fn an_orphan_is_kept_but_never_served<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy {
        program: 0,
        at,
        owner: Owner::LoaderV3,
        env: 0,
    };
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };
    let scenario = Scenario {
        tree: tree(&[&[4, 6, 8, 9], &[4, 5, 7]]),
        ops: vec![
            deploy(4),
            extract(6),
            finish_load(),
            deploy(5),
            extract(7),
            Op::Prune { root: 6 },
            finish_load(),
            extract(9),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    // Without this the scenario could stop reaching the race and still pass.
    assert!(
        report
            .fingerprint
            .iter()
            .any(|entry| entry.deployment_slot == 5),
        "the orphan outlived the prune: {:?}",
        report.fingerprint
    );

    let served = report.extractions.last().expect("an extraction");
    assert_eq!(served.asked_for, 4, "the caller names its own deployment");
    assert_eq!(
        served.kind,
        Some(EntryKind::Loaded),
        "and is served that, not the orphan"
    );
}

/// Fork graph created for the test
///            1 - 2 - 3
///            |   |   |
///            |   |   `-- and every batch here runs on the new one
///            |   `------ the epoch turns over here
///            `---------- deployment, on the outgoing environment
///
/// Ahead of an epoch boundary the preparation phase recompiles a program for
/// the environment which is coming. It cannot reuse the entry already in the
/// cache - that one was built for the outgoing environment - so it loads its
/// own. Crossing the boundary then makes the upcoming environment the one
/// every batch runs on, and sweeps what was built for the old one.
#[test_case(PhantomData::<V1>; "v1")]
fn crossing_an_epoch_boundary_sweeps_the_old_environment<R: Runner>(_: PhantomData<R>) {
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3]]),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: 1,
                owner: Owner::LoaderV3,
                env: 0,
            },
            Op::Extract {
                programs: vec![0],
                fork_tip: 2,
            },
            finish_load(),
            Op::RecompileForEpoch {
                program: 0,
                fork_tip: 2,
            },
            finish_load(),
            Op::CrossEpochBoundary { root: 2 },
            Op::Extract {
                programs: vec![0],
                fork_tip: 3,
            },
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [_, recompile, after] = report.extractions.as_slice() else {
        panic!("three extractions: {:?}", report.extractions);
    };
    assert!(
        !recompile.hit,
        "the entry in the cache is built for the outgoing environment"
    );
    assert!(recompile.started_load, "so the recompile loads its own");
    assert_eq!(
        after.kind,
        Some(EntryKind::Loaded),
        "which is what runs after the boundary"
    );
    assert!(!after.started_load, "with no reload to pay for");

    assert_eq!(
        report.fingerprint.len(),
        1,
        "and the outgoing environment's entry is swept: {:?}",
        report.fingerprint
    );
    assert!(
        report.fingerprint[0].env == Some(1),
        "leaving only the new one: {:?}",
        report.fingerprint
    );
}

/// Fork graph created for the test
///            1 - 2
///            |
///            `-- deployed here, a batch below asks for the load, then the
///                block is dumped and replayed
///
/// A dumped block never happened, so the load it asked for never happened
/// either. Every dump path drains the bank before clearing it, and
/// `replenish_program_cache` finishes its loads inside the batch which asked,
/// so nothing the block started is still running once it is gone.
///
/// Letting one survive puts a `Loaded` entry at the dumped slot, and replaying
/// the block then assigns an `Unloaded` one over it - a transition
/// `assign_program` has no arm for, which fires its debug assertion.
///
/// Found by the fuzzer.
#[test_case(PhantomData::<V1>; "v1")]
fn a_dumped_block_leaves_no_load_behind<R: Runner>(_: PhantomData<R>) {
    let deploy = || Op::Deploy {
        program: 0,
        at: 1,
        owner: Owner::LoaderV3,
        env: 0,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        ops: vec![
            deploy(),
            Op::Extract {
                programs: vec![0],
                fork_tip: 2,
            },
            Op::PurgeSlot { slot: 1 },
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            // The block replays, deploying the same program at the same slot.
            deploy(),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    assert!(
        report
            .extractions
            .iter()
            .any(|extraction| extraction.started_load),
        "the dumped block's batch did ask for a load: {:?}",
        report.extractions
    );
    assert_eq!(
        report.fingerprint.len(),
        1,
        "and the replay leaves one entry, not two: {:?}",
        report.fingerprint
    );
    assert!(
        report.fingerprint[0].kind == EntryKind::Unloaded,
        "the replayed deployment, with nothing left behind: {:?}",
        report.fingerprint
    );
}
