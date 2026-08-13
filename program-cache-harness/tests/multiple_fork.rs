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
            Step::NewSlot {
                parent: 4,
                slot: 5,
                canonical: true,
            },
            Step::Invoke {
                slot: 5,
                targets: vec![cold_a, warm, builtin],
            },
            Step::Assert(vec![
                // Only the fork that ran a transaction has warmed the cache.
                Entry::new_loaded(cold_a, 0),
                Entry::new_loaded(warm, 0),
                Entry::new_builtin(builtin),
            ]),
            // Branching off the genesis tip rather than extending slot 5, and
            // leaving the canonical tip where it is.
            Step::NewSlot {
                parent: 4,
                slot: 6,
                canonical: false,
            },
            Step::Invoke {
                slot: 6,
                targets: vec![cold_b],
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
            Step::NewSlot {
                parent: 4,
                slot: 5,
                canonical: true,
            },
            Step::Invoke {
                slot: 5,
                targets: vec![a],
            },
            Step::NewSlot {
                parent: 4,
                slot: 6,
                canonical: false,
            },
            Step::Invoke {
                slot: 6,
                targets: vec![b],
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
            Step::NewSlot {
                parent: 4,
                slot: 5,
                canonical: true,
            },
            Step::Invoke {
                slot: 5,
                targets: vec![a],
            },
            Step::NewSlot {
                parent: 4,
                slot: 6,
                canonical: false,
            },
            Step::Invoke {
                slot: 6,
                targets: vec![b],
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
