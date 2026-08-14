//! Multiple-fork happy path.

use {
    agave_program_cache_harness::{Entry, Genesis, Step, Timeline, run},
    solana_pubkey::Pubkey,
};

/// Two siblings off the genesis tip, each invoking its own programs. The one
/// global program cache serves both, so a program first loaded on the
/// non-canonical fork is still cached when viewed from the canonical one.
///
/// ```text
///                   ┌─ 5   invoke: cold_a, warm, builtin (canonical tip)
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ┤
///     ▲             └─ 6   invoke: cold_b (non-canonical)
///     └─ root
/// ```
#[test]
fn sanity() {
    let cold_a = Pubkey::new_unique();
    let cold_b = Pubkey::new_unique();
    let warm = Pubkey::new_unique();
    let builtin = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(
        vec![
            Entry::new_cold(cold_a, 0),
            Entry::new_cold(cold_b, 0),
            Entry::new_loaded(warm, 0),
            Entry::new_builtin(builtin),
        ],
        4,
    );

    let timeline = Timeline {
        steps: vec![
            Step::Advance { slot: 5 },
            Step::Invoke {
                slot: 5,
                targets: vec![cold_a, warm, builtin],
                served: vec![
                    Entry::new_loaded(cold_a, 0),
                    Entry::new_loaded(warm, 0),
                    Entry::new_builtin(builtin),
                ],
            },
            Step::Assert(vec![
                // Only the fork that ran a transaction has warmed the cache.
                Entry::new_loaded(cold_a, 0),
                Entry::new_loaded(warm, 0),
                Entry::new_builtin(builtin),
            ]),
            // Branching off the genesis tip rather than extending slot 5, and
            // leaving the canonical tip where it is.
            Step::NewSlotOn { parent: 4, slot: 6 },
            Step::Invoke {
                slot: 6,
                targets: vec![cold_b],
                served: vec![
                    Entry::new_loaded(cold_b, 0),
                    // Builtins are always served.
                    Entry::new_builtin(builtin),
                ],
            },
            Step::Assert(vec![
                // The cache is global, so both forks' programs are in it.
                Entry::new_loaded(cold_a, 0),
                Entry::new_loaded(cold_b, 0),
                Entry::new_loaded(warm, 0),
                Entry::new_builtin(builtin),
            ]),
        ],
    };

    run(genesis, timeline);
}

/// ```text
///                   ┌─ 5   invoke: a
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ┤
///     ▲             └─ 6   invoke: b
///     └─ root
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
                targets: vec![a],
                served: vec![Entry::new_loaded(a, 0)],
            },
            Step::NewSlotOn { parent: 4, slot: 6 },
            Step::Invoke {
                slot: 6,
                targets: vec![b],
                served: vec![Entry::new_loaded(b, 0)],
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
///                   ┌─ 5   invoke: a
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ┤
///     ▲             └─ 6   invoke: b
///     └─ root
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
                targets: vec![a],
                served: vec![Entry::new_loaded(a, 0)],
            },
            Step::NewSlotOn { parent: 4, slot: 6 },
            Step::Invoke {
                slot: 6,
                targets: vec![b],
                served: vec![Entry::new_loaded(b, 0)],
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

/// A fresh deployment on one fork and an upgrade on the other.
///
/// ```text
///                   ┌─ 5   deploy: fresh, canonical tip
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ┤
///     ▲             └─ 6   upgrade: existing, non-canonical
///     └─ root
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
            Step::Assert(vec![
                Entry::new_loaded(existing, 0),
                Entry::new_unloaded(fresh, 5),
            ]),
            Step::NewSlotOn { parent: 4, slot: 6 },
            Step::Deploy {
                slot: 6,
                targets: vec![existing],
            },
            // Both forks feed the one cache: the fresh deployment from the
            // canonical fork, and the upgrade from the branch beside it.
            Step::Assert(vec![
                Entry::new_loaded(existing, 0),
                Entry::new_unloaded(existing, 6),
                Entry::new_unloaded(fresh, 5),
            ]),
        ],
    };

    run(genesis, timeline);
}

/// Upgrade on a branch, then invoke on each fork and assert which entry each
/// one was actually served. The global cache holds both versions; only the
/// fork-scoped lookup says which one a given fork sees.
///
/// ```text
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ─ 5      served the slot-0 version
///                 └── 6 ─ 7   upgraded at 6; served the slot-6 version
/// ```
#[test]
fn sanity_extract() {
    let prog = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(vec![Entry::new_loaded(prog, 0)], 4);

    let timeline = Timeline {
        steps: vec![
            Step::Advance { slot: 5 },
            Step::NewSlotOn { parent: 4, slot: 6 },
            Step::Deploy {
                slot: 6,
                targets: vec![prog],
            },
            // The canonical fork never saw slot 6, so it still resolves the
            // original even though the index now holds both versions.
            Step::Invoke {
                slot: 5,
                targets: vec![prog],
                served: vec![Entry::new_loaded(prog, 0)],
            },
            Step::NewSlotOn { parent: 6, slot: 7 },
            // This fork descends from the upgrade, so it resolves the new one.
            Step::Invoke {
                slot: 7,
                targets: vec![prog],
                served: vec![Entry::new_loaded(prog, 6)],
            },
            Step::Assert(vec![Entry::new_loaded(prog, 0), Entry::new_loaded(prog, 6)]),
        ],
    };

    run(genesis, timeline);
}

/// Close on a branch. The canonical fork never saw the close, so it still
/// resolves the live program.
///
/// ```text
///                   ┌─ 5   invoke: prog, canonical tip
/// 0 ─ 1 ─ 2 ─ 3 ─ 4 ┤
///     ▲             └─ 6   close: prog
///     └─ root
/// ```
#[test]
fn sanity_close() {
    let prog = Pubkey::new_unique();

    let genesis = Genesis::new_with_features_all_enabled(vec![Entry::new_loaded(prog, 0)], 4);

    let timeline = Timeline {
        steps: vec![
            Step::NewSlotOn { parent: 4, slot: 6 },
            Step::Close {
                slot: 6,
                targets: vec![prog],
            },
            Step::Advance { slot: 5 },
            // The close is not on this fork, so the program still resolves.
            Step::Invoke {
                slot: 5,
                targets: vec![prog],
                served: vec![Entry::new_loaded(prog, 0)],
            },
            Step::Assert(vec![Entry::new_loaded(prog, 0), Entry::new_closed(prog, 6)]),
        ],
    };

    run(genesis, timeline);
}
