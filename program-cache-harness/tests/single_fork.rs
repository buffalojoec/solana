//! Single-fork happy path.

use {
    agave_program_cache_harness::{Entry, Expect, Genesis, Step, Timeline, run},
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
            Step::NewSlot {
                parent: 4,
                slot: 5,
                canonical: true,
            },
            Step::Assert(vec![
                // Genesis seeded these two, so they are cached before anything
                // executes. The cold program has an account but no entry.
                Expect::Present(Entry::new_loaded(cached, 0)),
                Expect::Present(Entry::new_builtin(builtin)),
                Expect::Absent(cold),
            ]),
            Step::Invoke {
                slot: 5,
                targets: vec![cached, cold, builtin],
            },
            Step::Assert(vec![
                // Invoking the cold program drove the real extraction path.
                Expect::Present(Entry::new_loaded(cold, 0)),
            ]),
            Step::NewSlot {
                parent: 5,
                slot: 6,
                canonical: true,
            },
            Step::Invoke {
                slot: 6,
                targets: vec![cached],
            },
        ],
    };

    run(genesis, timeline);
}
