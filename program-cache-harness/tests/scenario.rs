//! Known edge cases, written as scenarios.

#![allow(clippy::arithmetic_side_effects)]

use {
    solana_program_cache_harness::{
        EntryKind, LoadResult, Op, Owner, Runner, Scenario, Seed, V1, V2, run_twice,
        slots_in_new_epoch, tree,
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
#[test_case(PhantomData::<V2>; "v2")]
fn sanity<R: Runner>(_: PhantomData<R>) {
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
#[test_case(PhantomData::<V2>; "v2")]
fn a_deployment_is_not_visible_in_its_own_slot<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: Vec::new(),
        ops: vec![
            Op::Deploy { program: 0, at: 1 },
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
#[test_case(PhantomData::<V2>; "v2")]
fn a_redeployment_costs_one_reload<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy { program: 0, at };
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
        seeds: Vec::new(),
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
///
/// v1 only. `replenish_program_cache` goes round again until nothing is
/// missing, so a batch driving the real pipeline loads every program it names
/// before it returns. The round-by-round property is one of modelling the
/// cache, not of driving it.
#[test_case(PhantomData::<V1>; "v1")]
fn a_batch_is_handed_one_load_at_a_time<R: Runner>(_: PhantomData<R>) {
    let seed = |program, owner| Seed {
        program,
        owner,
        verifies: true,
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
        seeds: vec![
            seed(0, Owner::LoaderV1),
            seed(1, Owner::LoaderV2),
            seed(2, Owner::LoaderV3),
        ],
        ops: vec![
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
#[test_case(PhantomData::<V2>; "v2")]
fn a_load_which_fails_verification_is_not_retried<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV2,
            verifies: false,
        }],
        ops: vec![extract(2), extract(2), extract(3)],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [_, failed, later] = report.extractions.as_slice() else {
        panic!("three extractions: {:?}", report.extractions);
    };
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
#[test_case(PhantomData::<V2>; "v2")]
fn one_version_serves_every_fork<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 6], &[1, 3, 7], &[1, 4, 8], &[1, 5, 9]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV1,
            verifies: true,
        }],
        ops: vec![
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
    assert_eq!(first.asked_for, 0, "the seed sits at genesis");
    assert!(!first.hit, "the first fork to name it finds it unloaded");
    assert!(first.started_load, "and pays for the load");
    assert_eq!(rest.len(), 4);
    for served in rest {
        assert_eq!(served.asked_for, 0);
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
#[test_case(PhantomData::<V2>; "v2")]
fn a_deployment_on_one_branch_does_not_reach_the_other<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy { program: 0, at };
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
        seeds: Vec::new(),
        ops: vec![
            deploy(1),
            extract(5),
            finish_load(),
            deploy(2),
            extract(4),
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
#[test_case(PhantomData::<V2>; "v2")]
fn prune_keeps_only_the_newest_version_below_the_root<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy { program: 0, at };
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
        seeds: Vec::new(),
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
#[test_case(PhantomData::<V2>; "v2")]
fn a_fork_the_root_left_behind_is_never_worked_on<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0, 1],
        fork_tip,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3], &[1, 4, 5]]),
        seeds: Vec::new(),
        ops: vec![
            Op::Deploy { program: 0, at: 4 },
            extract(5),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            extract(5),
            Op::Prune { root: 2 },
            extract(5),
            Op::Deploy { program: 1, at: 5 },
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
#[test_case(PhantomData::<V2>; "v2")]
fn an_orphan_on_an_abandoned_fork_cannot_be_dumped<R: Runner>(_: PhantomData<R>) {
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3], &[1, 4, 5]]),
        seeds: Vec::new(),
        ops: vec![
            Op::Deploy { program: 0, at: 4 },
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
#[test_case(PhantomData::<V2>; "v2")]
fn an_orphan_is_kept_but_never_served<R: Runner>(_: PhantomData<R>) {
    let deploy = |at| Op::Deploy { program: 0, at };
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
        seeds: Vec::new(),
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
///            1 - 2
///
/// The preparation phase builds an entry for the environment which is coming,
/// alongside the one the cache already holds for the environment running now.
/// It cannot reuse that one - it was compiled against the outgoing environment
/// - so the program ends up held twice, once for each.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn a_recompile_builds_for_the_upcoming_environment<R: Runner>(_: PhantomData<R>) {
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV4,
            verifies: true,
        }],
        ops: vec![
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
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    // The recompile is not a batch, so the only extraction is the one which
    // ran before it.
    let [_] = report.extractions.as_slice() else {
        panic!("one extraction: {:?}", report.extractions);
    };

    let [first, second] = report.fingerprint.as_slice() else {
        panic!("two entries, one per environment: {:?}", report.fingerprint);
    };
    for held in [first, second] {
        assert_eq!(held.kind, EntryKind::Loaded, "{held}");
        assert_eq!(held.owner, Owner::LoaderV4, "{held}");
    }
    assert_ne!(
        first.env, second.env,
        "one for the outgoing environment and one for the upcoming"
    );
}

/// Fork graph created for the test
///           30 - 32 - 33
///           |    |
///           |    `-- the root moves here, into the next epoch
///           `------- deployment, on the outgoing environment
///
/// Ahead of the boundary the preparation phase recompiles a program for the
/// environment which is coming. It cannot reuse the entry already in the cache
/// - that one was built for the outgoing environment - so it loads its own.
/// Moving the root across the boundary then makes the upcoming environment the
/// one every later batch runs on, and sweeps what was built for the old one.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn crossing_an_epoch_boundary_sweeps_the_old_environment<R: Runner>(_: PhantomData<R>) {
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };
    let before = slots_in_new_epoch(0) - 2;
    let crossed = slots_in_new_epoch(0);
    let after = slots_in_new_epoch(1);
    let scenario = Scenario {
        tree: tree(&[&[before, crossed, after]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV4,
            verifies: true,
        }],
        ops: vec![
            Op::Extract {
                programs: vec![0],
                fork_tip: before,
            },
            finish_load(),
            Op::RecompileForEpoch {
                program: 0,
                fork_tip: before,
            },
            finish_load(),
            Op::Prune { root: crossed },
            Op::Extract {
                programs: vec![0],
                fork_tip: after,
            },
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    // The recompile is not a batch, so it leaves no extraction behind. What it
    // did is visible in what the batch after the boundary is spared.
    let [_, after] = report.extractions.as_slice() else {
        panic!("two extractions: {:?}", report.extractions);
    };
    assert_eq!(
        after.kind,
        Some(EntryKind::Loaded),
        "the recompiled entry is what runs after the boundary"
    );
    assert!(!after.started_load, "with no reload to pay for");
    let [held] = report.fingerprint.as_slice() else {
        panic!(
            "the outgoing environment was swept: {:?}",
            report.fingerprint
        );
    };
    assert_eq!(held.kind, EntryKind::Loaded);
    assert_eq!(
        held.env,
        Some(1),
        "leaving only the environment the boundary brought in"
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
#[test_case(PhantomData::<V2>; "v2")]
fn a_dumped_block_leaves_no_load_behind<R: Runner>(_: PhantomData<R>) {
    let deploy = || Op::Deploy { program: 0, at: 1 };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: Vec::new(),
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

/// Fork graph created for the test
///            1 - 2
///
/// A seed is written into the account rather than deployed through a loader,
/// so it can hold bytecode no loader would ever have accepted - which is how a
/// snapshot delivers a program deployed under an environment that has since
/// moved on. The cache holds what it cannot load as a tombstone, and serves
/// that to every batch which names it.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn a_seed_which_does_not_verify_is_a_tombstone<R: Runner>(_: PhantomData<R>) {
    let extract = || Op::Extract {
        programs: vec![0],
        fork_tip: 2,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV2,
            verifies: false,
        }],
        ops: vec![extract(), extract()],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    // The first batch is where the runners differ - v1 seeds the tombstone
    // into the cache up front, v2 loads it on demand - so the second is what
    // both can be held to.
    let served = report.extractions.last().expect("an extraction");
    assert_eq!(served.asked_for, 0, "a seed is deployed at genesis");
    assert_eq!(
        served.kind,
        Some(EntryKind::FailedVerification),
        "the batch is served the tombstone"
    );
    assert!(!served.started_load, "and is not handed the load again");

    let [held] = report.fingerprint.as_slice() else {
        panic!("one entry: {:?}", report.fingerprint);
    };
    assert_eq!(held.kind, EntryKind::FailedVerification);
    assert_eq!(held.owner, Owner::LoaderV2, "under its own loader");
    assert_eq!(held.deployment_slot, 0);
}

/// Fork graph created for the test
///            1 - 2
///
/// A program which arrived with the snapshot keeps the loader it arrived
/// under. Loader V3 is the only one which still accepts a deployment, so a
/// seed is the only route any other owner has into the cache - and it must not
/// be defaulted to V3 on the way in.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn a_seeded_program_keeps_its_own_loader<R: Runner>(_: PhantomData<R>) {
    let extract = || Op::Extract {
        programs: vec![0, 1],
        fork_tip: 2,
    };
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV1,
            verifies: true,
        }],
        ops: vec![
            Op::Deploy { program: 1, at: 1 },
            extract(),
            Op::FinishLoad {
                program: 0,
                result: LoadResult::Loaded,
            },
            extract(),
            Op::FinishLoad {
                program: 1,
                result: LoadResult::Loaded,
            },
            extract(),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let seeded = report
        .extractions
        .iter()
        .find(|extraction| extraction.program == 0)
        .expect("the seeded program is named");
    assert_eq!(seeded.asked_for, 0, "a seed is deployed at genesis");

    let deployed = report
        .extractions
        .iter()
        .find(|extraction| extraction.program == 1)
        .expect("the deployed program is named");
    assert_eq!(deployed.asked_for, 1, "the deployment names its own slot");

    for owner in [Owner::LoaderV1, Owner::LoaderV3] {
        assert!(
            report.fingerprint.iter().any(|held| held.owner == owner),
            "the cache holds an entry under {owner:?}: {:?}",
            report.fingerprint
        );
    }
}

/// Fork graph created for the test
///            1 - 2
///
/// Every loader arrives the same way: a seed writes the program at genesis and
/// the cache holds it there, under the loader the seed named. The account
/// shapes differ - V1 and V2 carry no deployment slot at all, V3 keeps one in
/// the programdata account the program account points at, and V4 in its own
/// header - and every route has to land on the same answer.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn every_seeded_loader_is_held_at_genesis<R: Runner>(_: PhantomData<R>) {
    for owner in [
        Owner::LoaderV1,
        Owner::LoaderV2,
        Owner::LoaderV3,
        Owner::LoaderV4,
    ] {
        let scenario = Scenario {
            tree: tree(&[&[1, 2]]),
            seeds: vec![Seed {
                program: 0,
                owner,
                verifies: true,
            }],
            ops: vec![Op::Extract {
                programs: vec![0],
                fork_tip: 2,
            }],
        };

        let report = run_twice::<R>(&scenario);
        report.assert_clean();

        let [asked] = report.extractions.as_slice() else {
            panic!("one extraction under {owner:?}: {:?}", report.extractions);
        };
        assert_eq!(asked.asked_for, 0, "{owner:?} is asked for at genesis");
        let [held] = report.fingerprint.as_slice() else {
            panic!("one entry under {owner:?}: {:?}", report.fingerprint);
        };
        assert_eq!(held.deployment_slot, 0, "{owner:?} is held at genesis");
        assert_eq!(held.owner, owner, "{owner:?} keeps its own loader");
    }
}

/// Fork graph created for the test
///            1 - 2
///            |
///            `-- the seeded program is redeployed here
///
/// A seed arrives upgradeable, so Loader V3 still accepts a deployment over
/// one. The redeployment names its own slot, the way any other deployment
/// does, and the program the seed put at genesis is left behind.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn a_seeded_program_can_be_deployed_over<R: Runner>(_: PhantomData<R>) {
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV3,
            verifies: true,
        }],
        ops: vec![
            Op::Deploy { program: 0, at: 1 },
            Op::Extract {
                programs: vec![0],
                fork_tip: 2,
            },
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [asked] = report.extractions.as_slice() else {
        panic!("one extraction: {:?}", report.extractions);
    };
    assert_eq!(asked.asked_for, 1, "the redeployment took");
    assert!(
        report
            .fingerprint
            .iter()
            .any(|held| held.deployment_slot == 1 && held.owner == Owner::LoaderV3),
        "the cache holds the redeployment: {:?}",
        report.fingerprint
    );
}

/// Fork graph created for the test
///            1 - 2
///
/// Only Loader V3 accepts a deployment, and no loader acts on an account
/// another one owns. A deployment naming a program which arrived under some
/// other loader is dropped, and the program keeps what it had.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn a_deployment_cannot_take_another_loaders_account<R: Runner>(_: PhantomData<R>) {
    let scenario = Scenario {
        tree: tree(&[&[1, 2]]),
        seeds: vec![Seed {
            program: 0,
            owner: Owner::LoaderV1,
            verifies: true,
        }],
        ops: vec![
            Op::Deploy { program: 0, at: 1 },
            Op::Extract {
                programs: vec![0],
                fork_tip: 2,
            },
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let served = report.extractions.last().expect("an extraction");
    assert_eq!(served.asked_for, 0, "the deployment never happened");

    assert_eq!(
        report.fingerprint.len(),
        1,
        "the deployment left nothing behind: {:?}",
        report.fingerprint
    );
    assert_eq!(
        report.fingerprint[0].owner,
        Owner::LoaderV1,
        "and the program keeps its own loader: {:?}",
        report.fingerprint
    );
}

/// Fork graph created for the test
///            1 - 2 - 3
///            |   |
///            |   `-- both programs closed here
///            `-- program (0) deployed here
///
/// A close writes the program account, so the same loader-ownership rule
/// applies as a deployment. Loader V3 closes its own program and leaves a
/// tombstone; the program which arrived under another loader keeps what it
/// had.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn a_close_cannot_take_another_loaders_account<R: Runner>(_: PhantomData<R>) {
    let scenario = Scenario {
        tree: tree(&[&[1, 2, 3]]),
        seeds: vec![Seed {
            program: 1,
            owner: Owner::LoaderV1,
            verifies: true,
        }],
        ops: vec![
            Op::Deploy { program: 0, at: 1 },
            Op::Close { program: 0, at: 2 },
            Op::Close { program: 1, at: 2 },
            Op::Extract {
                programs: vec![0, 1],
                fork_tip: 3,
            },
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let closed: Vec<_> = report
        .fingerprint
        .iter()
        .filter(|held| held.kind == EntryKind::Closed)
        .collect();
    assert_eq!(
        closed.len(),
        1,
        "only Loader V3's own program is closed: {:?}",
        report.fingerprint
    );
    assert_eq!(closed[0].program, 0);
    assert_eq!(
        closed[0].deployment_slot, 2,
        "the tombstone names the close"
    );

    let seeded: Vec<_> = report
        .fingerprint
        .iter()
        .filter(|held| held.program == 1)
        .collect();
    assert_eq!(
        seeded.len(),
        1,
        "the close left nothing behind: {:?}",
        report.fingerprint
    );
    assert_eq!(seeded[0].owner, Owner::LoaderV1);
    assert_eq!(seeded[0].deployment_slot, 0, "still the seed at genesis");
}

/// Fork graph created for the test
///           25 - 26 -+- 32 - 33                    fork A
///                    `- 27 -+- 34 - 35             fork B
///                           `- 28 -+- 36 - 37      fork C
///
/// An epoch is thirty-two slots and a bank is in the epoch its own slot names,
/// so the boundary falls at slot 32 exactly: 31 is the last slot of the first
/// epoch and 32 is the first of the second. Nothing about a fork enters into
/// it. So three forks off one deployment each cross at their own step, and at
/// a different depth of their own lineage:
///
///     fork    last slot in the first epoch    first slot in the second
///     A       26                              32
///     B       27                              34
///     C       28                              36
///
/// A batch runs on the environment its own slot's epoch names, so the three
/// below the boundary all query with the first environment and the three above
/// it all query with the second - whatever step each fork crossed at, and
/// whichever fork crossed first.
#[test_case(PhantomData::<V1>; "v1")]
#[test_case(PhantomData::<V2>; "v2")]
fn forks_cross_epoch_boundary_independently<R: Runner>(_: PhantomData<R>) {
    let extract = |fork_tip| Op::Extract {
        programs: vec![0],
        fork_tip,
    };
    let finish_load = || Op::FinishLoad {
        program: 0,
        result: LoadResult::Loaded,
    };

    let deployed = slots_in_new_epoch(0) - 7;
    let (below_a, below_b, below_c) = (
        slots_in_new_epoch(0) - 6,
        slots_in_new_epoch(0) - 5,
        slots_in_new_epoch(0) - 4,
    );
    let (above_a, above_b, above_c) = (
        slots_in_new_epoch(0),
        slots_in_new_epoch(2),
        slots_in_new_epoch(4),
    );

    let scenario = Scenario {
        tree: tree(&[
            &[deployed, below_a, above_a, slots_in_new_epoch(1)],
            &[deployed, below_a, below_b, above_b, slots_in_new_epoch(3)],
            &[
                deployed,
                below_a,
                below_b,
                below_c,
                above_c,
                slots_in_new_epoch(5),
            ],
        ]),
        seeds: Vec::new(),
        ops: vec![
            Op::Deploy {
                program: 0,
                at: deployed,
            },
            // Each fork's last slot before it crosses.
            extract(below_a),
            finish_load(),
            extract(below_b),
            extract(below_c),
            // And each fork's first slot after.
            extract(above_a),
            finish_load(),
            extract(above_b),
            extract(above_c),
        ],
    };

    let report = run_twice::<R>(&scenario);
    report.assert_clean();

    let [first_epoch, second_epoch] = [[below_a, below_b, below_c], [above_a, above_b, above_c]];
    let queried: Vec<(u64, u8)> = report
        .extractions
        .iter()
        .map(|served| (served.batch_slot, served.env))
        .collect();
    assert_eq!(
        queried,
        first_epoch
            .iter()
            .map(|slot| (*slot, 0))
            .chain(second_epoch.iter().map(|slot| (*slot, 1)))
            .collect::<Vec<_>>(),
        "each fork queries with the environment its own slot's epoch names"
    );

    for served in &report.extractions {
        assert_eq!(
            served.asked_for, deployed,
            "one deployment, named by every fork"
        );
    }
}
