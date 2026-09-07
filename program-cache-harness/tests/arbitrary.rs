//! How much of a generated scenario actually does anything.

#![allow(clippy::arithmetic_side_effects)]

use {
    arbitrary::{Arbitrary, Unstructured},
    solana_program_cache_harness::{Op, Scenario, V1, run},
    std::collections::BTreeSet,
};

const ACCEPTABLE_RATIO: f64 = 0.8;
const INPUT_LEN: usize = 128;
const SCENARIOS: usize = 150;

#[test]
fn most_generated_scenarios_check_something() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_byte = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 24) as u8
    };

    let mut bytes = [0u8; INPUT_LEN];
    let mut generated = 0usize;
    let mut checked = 0usize;
    let mut extractions = 0usize;
    for _ in 0..SCENARIOS {
        bytes.fill_with(&mut next_byte);
        let Ok(scenario) = Scenario::arbitrary(&mut Unstructured::new(&bytes)) else {
            continue;
        };
        generated += 1;

        let report = run::<V1>(&scenario);
        if !report.extractions.is_empty() {
            checked += 1;
        }
        extractions += report.extractions.len();
    }

    assert!(generated > 0, "nothing generated");
    let share = checked as f64 / generated as f64;
    assert!(
        share >= ACCEPTABLE_RATIO,
        "only {checked} of {generated} scenarios extracted anything ({extractions} extractions)"
    );
}

// Building a bank freezes every bank before it, so a block which has been
// built on takes no more transactions.
#[test]
fn no_generated_write_lands_on_a_block_already_built_on() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next_byte = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 24) as u8
    };

    let mut bytes = [0u8; INPUT_LEN];
    let (mut writes, mut onto_sealed) = (0usize, 0usize);
    for _ in 0..SCENARIOS {
        bytes.fill_with(&mut next_byte);
        let Ok(scenario) = Scenario::arbitrary(&mut Unstructured::new(&bytes)) else {
            continue;
        };

        let mut sealed: BTreeSet<u64> = BTreeSet::new();
        for op in &scenario.ops {
            let named = match op {
                Op::Deploy { at, .. } | Op::Close { at, .. } => {
                    writes += 1;
                    if sealed.contains(at) {
                        onto_sealed += 1;
                    }
                    Some(*at)
                }
                Op::Extract { fork_tip, .. } | Op::RecompileForEpoch { fork_tip, .. } => {
                    Some(*fork_tip)
                }
                Op::Prune { root } => Some(*root),
                Op::PurgeSlot { slot } => Some(*slot),
                Op::FinishLoad { .. } => None,
            };
            if let Some(at) = named {
                sealed.extend(scenario.tree.ancestry(at).into_iter().filter(|s| *s != at));
            }
        }
    }

    assert!(writes > 0, "the generator still deploys");
    assert_eq!(
        onto_sealed, 0,
        "{onto_sealed} of {writes} writes landed on a block already built on"
    );
}
