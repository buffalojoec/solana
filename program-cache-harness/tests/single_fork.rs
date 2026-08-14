//! Single-fork happy path.

use {
    agave_program_cache_harness::{Build, Entry, Env, Frame, Genesis, Run, run},
    solana_pubkey::Pubkey,
};

/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5 ─ 6
///         │       │   │   └─ invoke: cached
///         │       │   └───── invoke: cached, cold, builtin
///         │       └───────── genesis tip
///         └───────────────── root
/// ```
#[test]
fn sanity() {
    let cached = Pubkey::new_unique();
    let cold = Pubkey::new_unique();
    let builtin = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![
            Entry::new_loaded(cached, 0),
            Entry::new_cold(cold, 0),
            Entry::new_builtin(builtin),
        ],
        4,
    );

    let timeline = vec![
        Frame {
            build: vec![Build::Advance { slot: 5 }],
            run: vec![Run::Invoke {
                slot: 5,
                targets: vec![cached, cold, builtin],
                served: vec![
                    Entry::new_loaded(cached, 0),
                    Entry::new_loaded(cold, 0),
                    Entry::new_builtin(builtin),
                ],
            }],
            assert: vec![
                // Genesis seeded `cached` and the builtin; invoking the cold
                // program drove the real extraction path for the third.
                Entry::new_loaded(cached, 0),
                Entry::new_loaded(cold, 0),
                Entry::new_builtin(builtin),
            ],
        },
        Frame {
            build: vec![Build::Advance { slot: 6 }],
            run: vec![Run::Invoke {
                slot: 6,
                targets: vec![cached],
                served: vec![
                    Entry::new_loaded(cached, 0),
                    // Builtins are always served.
                    Entry::new_builtin(builtin),
                ],
            }],
            assert: vec![
                Entry::new_loaded(cached, 0),
                Entry::new_loaded(cold, 0),
                Entry::new_builtin(builtin),
            ],
        },
    ];

    run(genesis, timeline);
}

/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
///     │           │   └─ invoke: a, b
///     │           └───── genesis tip
///     └───────────────── root
/// ```
#[test]
fn sanity_all_cold() {
    let a = Pubkey::new_unique();
    let b = Pubkey::new_unique();
    let c = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![
            Entry::new_cold(a, 0),
            Entry::new_cold(b, 0),
            Entry::new_cold(c, 0),
        ],
        4,
    );

    let timeline = vec![Frame {
        build: vec![Build::Advance { slot: 5 }],
        run: vec![Run::Invoke {
            slot: 5,
            targets: vec![a, b],
            served: vec![Entry::new_loaded(a, 0), Entry::new_loaded(b, 0)],
        }],
        assert: vec![
            // `c` was never invoked, so it never reached the cache.
            Entry::new_loaded(a, 0),
            Entry::new_loaded(b, 0),
        ],
    }];

    run(genesis, timeline);
}

/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
///     │           │   └─ invoke: a, b
///     │           └───── genesis tip
///     └───────────────── root
/// ```
#[test]
fn sanity_all_unloaded() {
    let a = Pubkey::new_unique();
    let b = Pubkey::new_unique();
    let c = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![
            Entry::new_unloaded(a, 0),
            Entry::new_unloaded(b, 0),
            Entry::new_unloaded(c, 0),
        ],
        4,
    );

    let timeline = vec![Frame {
        build: vec![Build::Advance { slot: 5 }],
        run: vec![Run::Invoke {
            slot: 5,
            targets: vec![a, b],
            served: vec![Entry::new_loaded(a, 0), Entry::new_loaded(b, 0)],
        }],
        assert: vec![
            // `c` was never invoked, so its tombstone still stands.
            Entry::new_loaded(a, 0),
            Entry::new_loaded(b, 0),
            Entry::new_unloaded(c, 0),
        ],
    }];

    run(genesis, timeline);
}

/// Two batches racing each other on one bank, sharing a program.
///
/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
///     │           │   ├─ invoke: shared, a
///     │           │   └─ invoke: shared, b   (at the same moment)
///     │           └───── genesis tip
///     └───────────────── root
/// ```
#[test]
fn sanity_concurrent() {
    let shared = Pubkey::new_unique();
    let a = Pubkey::new_unique();
    let b = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![
            Entry::new_cold(shared, 0),
            Entry::new_cold(a, 0),
            Entry::new_cold(b, 0),
        ],
        4,
    );

    let timeline = vec![Frame {
        build: vec![Build::Advance { slot: 5 }],
        run: vec![
            Run::Invoke {
                slot: 5,
                targets: vec![shared, a],
                served: vec![Entry::new_loaded(shared, 0), Entry::new_loaded(a, 0)],
            },
            Run::Invoke {
                slot: 5,
                targets: vec![shared, b],
                served: vec![Entry::new_loaded(shared, 0), Entry::new_loaded(b, 0)],
            },
        ],
        assert: vec![
            // Whichever batch loaded `shared` first, both were served it and
            // only one entry for it exists.
            Entry::new_loaded(shared, 0),
            Entry::new_loaded(a, 0),
            Entry::new_loaded(b, 0),
        ],
    }];

    run(genesis, timeline);
}

/// A fresh deployment and an upgrade in one flow.
///
/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5 ─ 6 ─ 7
///     │           │   │   │   └─ invoke: existing
///     │           │   │   └───── invoke: fresh, upgrade: existing (at once)
///     │           │   └───────── deploy: fresh
///     │           └───────────── genesis tip
///     └───────────────────────── root
/// ```
#[test]
fn sanity_deployments() {
    let existing = Pubkey::new_unique();
    let fresh = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(vec![Entry::new_loaded(existing, 0)], 4);

    let timeline = vec![
        Frame {
            build: vec![Build::Advance { slot: 5 }],
            run: vec![Run::Deploy {
                slot: 5,
                targets: vec![fresh],
            }],
            // A fresh deployment lands unloaded: nothing has invoked it, and
            // delay visibility means nothing can until the next slot.
            assert: vec![
                Entry::new_loaded(existing, 0),
                Entry::new_unloaded(fresh, 5),
            ],
        },
        Frame {
            build: vec![Build::Advance { slot: 6 }],
            run: vec![
                Run::Invoke {
                    slot: 6,
                    targets: vec![fresh],
                    served: vec![Entry::new_loaded(fresh, 5)],
                },
                Run::Deploy {
                    slot: 6,
                    targets: vec![existing],
                },
            ],
            // Invoking `fresh` compiles it, still at its deployment slot. The
            // upgrade beside it leaves the old version of `existing` in place
            // and adds a second, ordered after it by deployment slot.
            assert: vec![
                Entry::new_loaded(existing, 0),
                Entry::new_unloaded(existing, 6),
                Entry::new_loaded(fresh, 5),
            ],
        },
        Frame {
            build: vec![Build::Advance { slot: 7 }],
            // With two versions to choose between, the newer one resolves.
            run: vec![Run::Invoke {
                slot: 7,
                targets: vec![existing],
                served: vec![Entry::new_loaded(existing, 6)],
            }],
            assert: vec![
                Entry::new_loaded(existing, 0),
                Entry::new_loaded(existing, 6),
                Entry::new_loaded(fresh, 5),
            ],
        },
    ];

    run(genesis, timeline);
}

/// Close a program, leaving a tombstone behind at the slot that ran it.
///
/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
///     │           │   └─ close: prog
///     │           └───── genesis tip
///     └───────────────── root
/// ```
#[test]
fn sanity_close() {
    let prog = Pubkey::new_unique();
    let other = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![Entry::new_loaded(prog, 0), Entry::new_loaded(other, 0)],
        4,
    );

    let timeline = vec![Frame {
        build: vec![Build::Advance { slot: 5 }],
        run: vec![Run::Close {
            slot: 5,
            targets: vec![prog],
        }],
        // The tombstone sits at the closing slot, and the version it replaced
        // stays in the index beside it.
        assert: vec![
            Entry::new_loaded(prog, 0),
            Entry::new_closed(prog, 5),
            Entry::new_loaded(other, 0),
        ],
    }];

    run(genesis, timeline);
}

/// The same program cached under two environments. `extract` skips the entry
/// whose environment does not match the bank's, so invoking reloads the
/// program and leaves both entries side by side at one deployment slot.
///
/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5
///     │           │   └─ invoke: prog
///     │           └───── genesis tip
///     └───────────────── root
/// ```
#[test]
fn sanity_environments() {
    let prog = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![Entry::new_unloaded(prog, 0).in_env(Env::Alternate)],
        4,
    );

    let timeline = vec![Frame {
        build: vec![Build::Advance { slot: 5 }],
        run: vec![Run::Invoke {
            slot: 5,
            targets: vec![prog],
            served: vec![Entry::new_loaded(prog, 0)],
        }],
        assert: vec![
            // The seeded entry is untouched; the reload sits beside it.
            Entry::new_unloaded(prog, 0).in_env(Env::Alternate),
            Entry::new_loaded(prog, 0),
        ],
    }];

    run(genesis, timeline);
}

/// Crossing an epoch boundary sweeps entries whose environment is not the
/// rerooting bank's. Ordinary rerooting inside an epoch does not: `prune` only
/// receives an environment to match against on the reroot that concludes an
/// epoch transition.
///
/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5 … 9 ─ 16 ─ 32 … 36
///                       │         │      └─ invoke: prog
///                       │         └──────── epoch 1 begins
///                       └────────────────── still epoch 0
/// ```
#[test]
fn sanity_epoch_transition() {
    let prog = Pubkey::new_unique();
    let stale = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![
            Entry::new_loaded(prog, 0),
            Entry::new_unloaded(stale, 0).in_env(Env::Alternate),
        ],
        4,
    );

    let timeline = vec![
        Frame {
            // Five reroots, all inside epoch 0.
            build: (5..=9).map(|slot| Build::Advance { slot }).collect(),
            run: vec![Run::Invoke {
                slot: 9,
                targets: vec![prog],
                served: vec![Entry::new_loaded(prog, 0)],
            }],
            // The alternate-environment entry is untouched by an ordinary
            // reroot, however many of them run.
            assert: vec![
                Entry::new_loaded(prog, 0),
                Entry::new_unloaded(stale, 0).in_env(Env::Alternate),
            ],
        },
        Frame {
            // Slot 16 enters the recompilation phase, which arms the upcoming
            // environment; slot 32 begins epoch 1; the root reaches 32 last.
            build: vec![16, 32, 33, 34, 35, 36]
                .into_iter()
                .map(|slot| Build::Advance { slot })
                .collect(),
            run: vec![Run::Invoke {
                slot: 36,
                targets: vec![prog],
                served: vec![Entry::new_loaded(prog, 0)],
            }],
            // The transition swept it.
            assert: vec![Entry::new_loaded(prog, 0)],
        },
    ];

    run(genesis, timeline);
}
