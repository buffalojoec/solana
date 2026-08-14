//! Single-fork happy path.

use {
    agave_program_cache_harness::{Entry, Genesis, Step, Timeline, run},
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

    let timeline = Timeline {
        steps: vec![
            Step::Advance { slot: 5 },
            Step::Assert(vec![
                // Genesis seeded these two, so they are cached before anything
                // executes. The cold program has an account but no entry.
                Entry::new_loaded(cached, 0),
                Entry::new_builtin(builtin),
            ]),
            Step::Invoke {
                slot: 5,
                targets: vec![cached, cold, builtin],
                served: vec![
                    Entry::new_loaded(cached, 0),
                    Entry::new_loaded(cold, 0),
                    Entry::new_builtin(builtin),
                ],
            },
            Step::Assert(vec![
                // Invoking the cold program drove the real extraction path.
                Entry::new_loaded(cached, 0),
                Entry::new_loaded(cold, 0),
                Entry::new_builtin(builtin),
            ]),
            Step::Advance { slot: 6 },
            Step::Invoke {
                slot: 6,
                targets: vec![cached],
                served: vec![Entry::new_loaded(cached, 0)],
            },
        ],
    };

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

    let timeline = Timeline {
        steps: vec![
            Step::Advance { slot: 5 },
            Step::Invoke {
                slot: 5,
                targets: vec![a, b],
                served: vec![Entry::new_loaded(a, 0), Entry::new_loaded(b, 0)],
            },
            Step::Assert(vec![
                // `c` was never invoked, so it never reached the cache.
                Entry::new_loaded(a, 0),
                Entry::new_loaded(b, 0),
            ]),
        ],
    };

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

    let timeline = Timeline {
        steps: vec![
            Step::Advance { slot: 5 },
            Step::Invoke {
                slot: 5,
                targets: vec![a, b],
                served: vec![Entry::new_loaded(a, 0), Entry::new_loaded(b, 0)],
            },
            Step::Assert(vec![
                // `c` was never invoked, so its tombstone still stands.
                Entry::new_loaded(a, 0),
                Entry::new_loaded(b, 0),
                Entry::new_unloaded(c, 0),
            ]),
        ],
    };

    run(genesis, timeline);
}

/// A fresh deployment and an upgrade in one flow.
///
/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5 ─ 6
///     │           │   │   └─ invoke: fresh, then upgrade: existing
///     │           │   └───── deploy: fresh
///     │           └───────── genesis tip
///     └───────────────────── root
/// ```
#[test]
fn sanity_deployments() {
    let existing = Pubkey::new_unique();
    let fresh = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(vec![Entry::new_loaded(existing, 0)], 4);

    let timeline = Timeline {
        steps: vec![
            Step::Advance { slot: 5 },
            Step::Deploy {
                slot: 5,
                targets: vec![fresh],
            },
            // A fresh deployment lands unloaded: nothing has invoked it, and
            // delay visibility means nothing can until the next slot.
            Step::Assert(vec![
                Entry::new_loaded(existing, 0),
                Entry::new_unloaded(fresh, 5),
            ]),
            Step::Advance { slot: 6 },
            Step::Invoke {
                slot: 6,
                targets: vec![fresh],
                served: vec![Entry::new_loaded(fresh, 5)],
            },
            // Invoking it compiles the entry, still at its deployment slot.
            Step::Assert(vec![
                Entry::new_loaded(existing, 0),
                Entry::new_loaded(fresh, 5),
            ]),
            Step::Deploy {
                slot: 6,
                targets: vec![existing],
            },
            // An upgrade leaves the old version in place and adds a second,
            // ordered after it by deployment slot.
            Step::Assert(vec![
                Entry::new_loaded(existing, 0),
                Entry::new_unloaded(existing, 6),
                Entry::new_loaded(fresh, 5),
            ]),
            Step::Advance { slot: 7 },
            // With two versions to choose between, the newer one resolves.
            Step::Invoke {
                slot: 7,
                targets: vec![existing],
                served: vec![Entry::new_loaded(existing, 6)],
            },
        ],
    };

    run(genesis, timeline);
}
