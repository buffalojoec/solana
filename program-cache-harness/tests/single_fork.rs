//! Single-fork happy path.

use {
    agave_program_cache_harness::{Entry, EntryType, Genesis, Step, Timeline, run},
    solana_pubkey::Pubkey,
};

fn find(entries: &[Entry], id: &Pubkey) -> Option<Entry> {
    entries.iter().find(|entry| entry.id == *id).copied()
}

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
            Step::Invoke {
                slot: 5,
                targets: vec![cached, cold, builtin],
            },
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

    let effects = run(genesis, timeline);
    assert_eq!(effects.len(), 4);

    let (before_execute, after_execute) = (&effects[0], &effects[1]);

    // Genesis seeded these two, so they are cached before anything executes.
    assert_eq!(find(before_execute, &cached).unwrap().ty, EntryType::Loaded);
    assert_eq!(
        find(before_execute, &builtin).unwrap().ty,
        EntryType::Builtin
    );

    // The cold program has an account but no entry, until invoking it drives
    // the real extraction path.
    assert!(find(before_execute, &cold).is_none());
    assert_eq!(find(after_execute, &cold).unwrap().ty, EntryType::Loaded);
}
