//! Turning fuzzer bytes into a [`Scenario`].
//!
//! The generator has to respect fork graph rules and do its best to trace the
//! fork tree in the same way the harnesses and production do, in order to
//! avoid generating useless bytes.

#![allow(clippy::arithmetic_side_effects)]

use {
    super::{ForkTree, LoadResult, NUM_ENVIRONMENTS, NUM_PROGRAMS, Op, Scenario},
    crate::rules,
    arbitrary::{Arbitrary, Result, Unstructured},
    solana_clock::Slot,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner,
};

const MAX_OPS: usize = 24;

impl<'a> Arbitrary<'a> for Scenario {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        let tree = tree(u)?;
        let mut root: Slot = 0;
        let mut live = tree.slots();

        let seeded = seed_deployments(u, &live)?;
        let mut searchable = reachable_from(&tree, &seeded.slots);
        let mut ops = seeded.ops;

        while ops.len() < MAX_OPS && !u.is_empty() {
            let emitted = ops.len();
            let set_piece = match u.int_in_range(0..=3)? {
                // One in four: cross-fork race conditions.
                0 => cross_fork_load_race(u, &tree, root)?,
                // One in four: stacked deployments.
                1 => stacked_versions(u, &tree, root)?,
                _ => None,
            };
            match set_piece {
                Some(piece) => ops.extend(piece),
                // Otherwise, and whenever the tree admits no set piece, a
                // random op.
                None => ops.push(random_op(u, &live, &searchable)?),
            }
            for op in ops.iter().skip(emitted) {
                root = rules::advance_root(&tree, root, op);
            }
            live.retain(|slot| rules::is_live(&tree, root, *slot));
            searchable.retain(|slot| rules::is_live(&tree, root, *slot));
        }

        Ok(Scenario { tree, ops })
    }
}

// Generate a `ForkTree`.
fn tree(u: &mut Unstructured<'_>) -> Result<ForkTree> {
    // How many slots the tree holds.
    let count = u.int_in_range(3..=8)?;

    // Generate slot topology arbitrarily.
    let mut placed: Vec<(Slot, Slot)> = Vec::new();
    let mut next: Slot = 1;
    for _ in 0..count {
        let slot = next;
        let anc = u.int_in_range(0..=placed.len())?;
        let parent = if anc == 0 { 0 } else { placed[anc - 1].0 };
        placed.push((slot, parent));

        // Skip one, maybe two slots.
        let gap = u.int_in_range(1..=2u8)?;
        next += u64::from(gap);
    }

    let parent_of = |slot: Slot| {
        placed
            .iter()
            .find(|(at, _)| *at == slot)
            .map_or(0, |(_, parent)| *parent)
    };
    let is_leaf = |slot: &Slot| !placed.iter().any(|(_, parent)| parent == slot);
    let mut tree = ForkTree::default();
    for leaf in placed.iter().map(|(slot, _)| *slot).filter(is_leaf) {
        let mut path = vec![leaf];
        let mut at = parent_of(leaf);
        while at != 0 {
            path.push(at);
            at = parent_of(at);
        }
        path.reverse();
        tree.insert_fork(&path);
    }

    Ok(tree)
}

struct SeededDeployments {
    ops: Vec<Op>,
    slots: Vec<Slot>,
}

// Seed the scenario with a deployment per program before generating ops.
fn seed_deployments(u: &mut Unstructured<'_>, live: &[Slot]) -> Result<SeededDeployments> {
    let mut ops = Vec::new();
    let mut slots = Vec::new();
    for program in 0..NUM_PROGRAMS {
        let at = pick_low(u, live)?;
        slots.push(at);
        ops.push(Op::Deploy {
            program,
            at,
            owner: owner(u)?,
            env: env(u)?,
        });
    }
    Ok(SeededDeployments { ops, slots })
}

// Slots which can see one of the seeded deployments.
fn reachable_from(tree: &ForkTree, seeded: &[Slot]) -> Vec<Slot> {
    tree.slots()
        .into_iter()
        .filter(|slot| {
            let ancestry = tree.ancestry(*slot);
            seeded.iter().any(|at| ancestry.contains(at))
        })
        .collect()
}

// Emits the sequence which strands an entry on a fork the root has left
// behind, if the tree admits one.
//
// ```
//          branch          <-- a deployment both forks can see
//         /      \
//     doomed      root     <-- the root moves here, stranding `doomed`
//                    \
//                   victim <-- and a batch here must not be served it
// ```
fn cross_fork_load_race(
    u: &mut Unstructured<'_>,
    tree: &ForkTree,
    root: Slot,
) -> Result<Option<Vec<Op>>> {
    let candidates = race_candidates(tree, root);
    if candidates.is_empty() {
        return Ok(None);
    }

    let Race {
        branch,
        doomed,
        doomed_tip,
        root,
        victim,
    } = pick(u, &candidates)?;
    let program = program(u)?;
    let owner = owner(u)?;
    let env = env(u)?;
    // Asked for by the preparation phase instead, so the entry it strands is
    // built for the environment which is coming rather than the one in use.
    let prepared: bool = u.arbitrary()?;
    // One in three strands a second program on the same doomed fork, so two
    // orphans land from one prune.
    let second: Option<u8> = (u.int_in_range(0..=2u8)? == 0)
        .then(|| (0..NUM_PROGRAMS).find(|candidate| *candidate != program))
        .flatten();

    let mut ops = Vec::new();
    if let Some(other) = second {
        ops.extend([
            Op::Deploy {
                program: other,
                at: branch,
                owner,
                env,
            },
            Op::Deploy {
                program: other,
                at: doomed,
                owner,
                env,
            },
        ]);
    }
    ops.extend([
        Op::Deploy {
            program,
            at: branch,
            owner,
            env,
        },
        Op::Deploy {
            program,
            at: doomed,
            owner,
            env,
        },
        if prepared {
            Op::RecompileForEpoch {
                program,
                fork_tip: doomed_tip,
            }
        } else {
            Op::Extract {
                programs: vec![program],
                fork_tip: doomed_tip,
            }
        },
    ]);
    if let Some(other) = second {
        ops.push(Op::Extract {
            programs: vec![other],
            fork_tip: doomed_tip,
        });
    }

    // Pruning occurs next.
    ops.push(Op::Prune { root });

    // The loads land after the prune which would have caught them.
    ops.push(Op::FinishLoad {
        program,
        result: LoadResult::Loaded,
    });
    if let Some(other) = second {
        ops.push(Op::FinishLoad {
            program: other,
            result: LoadResult::Loaded,
        });
    }

    // Two batches on the surviving fork with a load between them. The first
    // is where the orphan can be served as a wrong entry; the load and the
    // batch after it show a reload the next batch cannot see.
    let searched: Vec<u8> = Some(program).into_iter().chain(second).collect();
    ops.extend([
        Op::Extract {
            programs: searched.clone(),
            fork_tip: victim,
        },
        Op::FinishLoad {
            program,
            result: LoadResult::Loaded,
        },
        Op::Extract {
            programs: searched,
            fork_tip: victim,
        },
    ]);
    Ok(Some(ops))
}

#[derive(Clone, Copy)]
struct Race {
    branch: Slot,
    doomed: Slot,
    doomed_tip: Slot,
    root: Slot,
    victim: Slot,
}

fn race_candidates(tree: &ForkTree, root: Slot) -> Vec<Race> {
    let slots = live_slots(tree, root);
    let mut candidates = Vec::new();
    for &victim in &slots {
        let victim_ancestry = tree.ancestry(victim);
        for &doomed in &slots {
            if victim_ancestry.contains(&doomed) {
                continue;
            }
            // A deployment both forks can see has to exist above the split.
            let Some(&branch) = tree
                .ancestry(doomed)
                .iter()
                .rfind(|slot| **slot != 0 && victim_ancestry.contains(slot))
            else {
                continue;
            };
            // The batch which asks for the load has to sit *below* the
            // deployment, or it lands in the delay visibility window and is
            // handed a tombstone instead of missing.
            let Some(&doomed_tip) = slots
                .iter()
                .find(|slot| **slot > doomed && tree.ancestry(**slot).contains(&doomed))
            else {
                continue;
            };
            // And the root has to be able to move past the doomed fork while
            // the victim's own fork survives.
            if let Some(&root) = victim_ancestry.iter().find(|slot| **slot > doomed) {
                candidates.push(Race {
                    branch,
                    doomed,
                    doomed_tip,
                    root,
                    victim,
                });
            }
        }
    }
    candidates
}

fn stacked_versions(
    u: &mut Unstructured<'_>,
    tree: &ForkTree,
    root: Slot,
) -> Result<Option<Vec<Op>>> {
    let slots = live_slots(tree, root);
    if slots.len() < 3 {
        return Ok(None);
    }
    // One fork, so every deployment is on the same lineage and prune has to
    // choose between them rather than discard them as orphans.
    let tip = pick(u, &slots)?;
    let lineage: Vec<Slot> = tree
        .ancestry(tip)
        .into_iter()
        .filter(|slot| *slot != 0)
        .collect();
    if lineage.len() < 3 {
        return Ok(None);
    }

    let program = program(u)?;
    let owner = owner(u)?;
    let mut ops: Vec<Op> = lineage
        .iter()
        .enumerate()
        .map(|(index, at)| Op::Deploy {
            program,
            at: *at,
            owner,
            env: (index as u8) % NUM_ENVIRONMENTS,
        })
        .collect();

    // Sometimes one more version of the same program on a fork the lineage
    // does not include.
    let sibling: bool = u.arbitrary()?;
    if sibling && let Some(at) = slots.iter().copied().find(|slot| !lineage.contains(slot)) {
        ops.push(Op::Deploy {
            program,
            at,
            owner,
            env: (lineage.len() as u8) % NUM_ENVIRONMENTS,
        });
    }

    // Sometimes root above the stack.
    let reroot: bool = u.arbitrary()?;
    if reroot {
        ops.push(Op::Prune { root: tip });
    }

    ops.extend([
        Op::Extract {
            programs: vec![program],
            fork_tip: tip,
        },
        Op::FinishLoad {
            program,
            result: LoadResult::Loaded,
        },
    ]);
    Ok(Some(ops))
}

fn random_op(u: &mut Unstructured<'_>, live: &[Slot], searchable: &[Slot]) -> Result<Op> {
    // Weights:
    //
    //   0, 1  Deploy              2/10
    //   2     Close               1/10
    //   3, 4  Extract             2/10
    //   5     FinishLoad          1/10
    //   6     Prune               1/10
    //   7     RecompileForEpoch   1/10
    //   8     CrossEpochBoundary  1/10
    //   9     PurgeSlot           1/10
    //
    let op = match u.int_in_range(0..=9)? {
        0 | 1 => Op::Deploy {
            program: program(u)?,
            at: pick(u, live)?,
            owner: owner(u)?,
            env: env(u)?,
        },
        2 => Op::Close {
            program: program(u)?,
            at: pick(u, live)?,
        },
        3 | 4 => Op::Extract {
            programs: {
                let mut programs = Vec::new();
                for program in 0..NUM_PROGRAMS {
                    if u.arbitrary()? {
                        programs.push(program);
                    }
                }
                if programs.is_empty() {
                    programs.push(program(u)?);
                }
                programs
            },
            fork_tip: pick(u, or_live(searchable, live))?,
        },
        5 => Op::FinishLoad {
            program: program(u)?,
            result: if u.int_in_range(0..=7)? == 0 {
                // One load in eight fails to verify.
                LoadResult::FailedVerification
            } else {
                LoadResult::Loaded
            },
        },
        6 => Op::Prune {
            root: pick(u, live)?,
        },
        7 => Op::RecompileForEpoch {
            program: program(u)?,
            fork_tip: pick(u, or_live(searchable, live))?,
        },
        8 => Op::CrossEpochBoundary {
            root: pick(u, live)?,
        },
        _ => Op::PurgeSlot {
            slot: pick(u, live)?,
        },
    };

    Ok(op)
}

fn live_slots(tree: &ForkTree, root: Slot) -> Vec<Slot> {
    tree.slots()
        .into_iter()
        .filter(|slot| rules::is_live(tree, root, *slot))
        .collect()
}

fn or_live<'a>(searchable: &'a [Slot], live: &'a [Slot]) -> &'a [Slot] {
    if searchable.is_empty() {
        live
    } else {
        searchable
    }
}

fn pick<T: Copy>(u: &mut Unstructured<'_>, from: &[T]) -> Result<T> {
    if from.is_empty() {
        return Err(arbitrary::Error::IncorrectFormat);
    }
    Ok(from[u.int_in_range(0..=from.len() - 1)?])
}

fn pick_low(u: &mut Unstructured<'_>, from: &[Slot]) -> Result<Slot> {
    let first = pick(u, from)?;
    let second = pick(u, from)?;
    Ok(first.min(second))
}

fn program(u: &mut Unstructured<'_>) -> Result<u8> {
    u.int_in_range(0..=NUM_PROGRAMS - 1)
}

fn env(u: &mut Unstructured<'_>) -> Result<u8> {
    u.int_in_range(0..=NUM_ENVIRONMENTS - 1)
}

fn owner(u: &mut Unstructured<'_>) -> Result<ProgramCacheEntryOwner> {
    Ok(match u.int_in_range(0..=3)? {
        0 => ProgramCacheEntryOwner::LoaderV1,
        1 => ProgramCacheEntryOwner::LoaderV2,
        2 => ProgramCacheEntryOwner::LoaderV3,
        _ => ProgramCacheEntryOwner::LoaderV4,
    })
}
