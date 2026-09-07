//! Turning fuzzer bytes into a [`Scenario`].
//!
//! The generator has to respect fork graph rules and do its best to trace the
//! fork tree in the same way the harnesses and production do, in order to
//! avoid generating useless bytes.

#![allow(clippy::arithmetic_side_effects)]

use {
    super::{ForkTree, LoadResult, MAX_SLOT, NUM_PROGRAMS, Op, SLOTS_PER_EPOCH, Scenario, Seed},
    crate::rules,
    arbitrary::{Arbitrary, Result, Unstructured},
    solana_clock::Slot,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner,
    std::collections::BTreeSet,
};

const MAX_OPS: usize = 24;

impl<'a> Arbitrary<'a> for Scenario {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        let tree = tree(u)?;
        let mut root: Slot = 0;
        let mut live = tree.slots();

        // Slots which have been built on, and so take no more transactions.
        let mut sealed: BTreeSet<Slot> = BTreeSet::new();

        let seeded = seed_deployments(u, &live)?;
        let mut searchable = reachable_from(&tree, &seeded.slots);
        let mut ops = seeded.ops;
        for op in &ops {
            sealed.extend(rules::sealed_by(&tree, op));
        }

        while ops.len() < MAX_OPS && !u.is_empty() {
            let emitted = ops.len();
            let set_piece = match u.int_in_range(0..=3)? {
                // One in four: cross-fork race conditions.
                0 => cross_fork_load_race(u, &tree, root, &seeded.deployable, &sealed)?,
                // One in four: stacked deployments.
                1 => stacked_versions(u, &tree, root, &seeded.deployable, &sealed)?,
                _ => None,
            };
            match set_piece {
                Some(piece) => ops.extend(piece),
                // Otherwise, and whenever the tree admits no set piece, a
                // random op.
                None => {
                    let writable: Vec<Slot> = live
                        .iter()
                        .copied()
                        .filter(|slot| rules::can_write(&sealed, *slot))
                        .collect();
                    ops.push(random_op(
                        u,
                        &live,
                        &searchable,
                        &seeded.deployable,
                        &writable,
                    )?);
                }
            }
            for op in ops.iter().skip(emitted) {
                root = rules::advance_root(&tree, root, op);
                sealed.extend(rules::sealed_by(&tree, op));
            }
            live.retain(|slot| rules::is_live(&tree, root, *slot));
            searchable.retain(|slot| rules::is_live(&tree, root, *slot));
        }

        Ok(Scenario {
            tree,
            seeds: seeded.seeds,
            ops,
        })
    }
}

// Generate a `ForkTree`.
fn tree(u: &mut Unstructured<'_>) -> Result<ForkTree> {
    // How many slots the tree holds.
    let count = u.int_in_range(3..=8)?;

    // Where the tree sits relative to the epoch boundary.
    //
    //   0              one epoch, no boundary     1/4
    //   1..=count - 1  straddling the boundary    3/4
    //
    let straddle = u.int_in_range(0..=3u8)? != 0;
    let num_below = if straddle {
        u.int_in_range(1..=count - 1)?
    } else {
        0
    };

    // Generate slot topology arbitrarily.
    let mut placed: Vec<(Slot, Slot)> = Vec::new();
    let mut next: Slot = 1;
    for index in 0..count {
        // The first `num_below` slots sit under the boundary.
        let slot = if straddle && index == num_below {
            next = next.max(SLOTS_PER_EPOCH);
            next
        } else {
            next
        };
        let anc = u.int_in_range(0..=placed.len())?;
        let parent = if anc == 0 { 0 } else { placed[anc - 1].0 };
        placed.push((slot, parent));

        // Skip one, maybe two slots.
        let gap = u.int_in_range(1..=2u8)?;
        next = slot.saturating_add(u64::from(gap)).min(MAX_SLOT);
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
    seeds: Vec<Seed>,
    ops: Vec<Op>,
    slots: Vec<Slot>,
    deployable: Vec<u8>,
}

// Seed the scenario with a deployment per program before generating ops.
fn seed_deployments(u: &mut Unstructured<'_>, live: &[Slot]) -> Result<SeededDeployments> {
    let mut seeds = Vec::new();
    let mut ops = Vec::new();
    let mut slots = Vec::new();
    // How many programs arrive with the snapshot instead of being deployed.
    //
    //   0      two seeds    1/16
    //   1, 2   one seed     2/16
    //   3..16  none        13/16
    //
    let seeded = match u.int_in_range(0..=15u8)? {
        0 => 2,
        1 | 2 => 1,
        _ => 0,
    };
    let mut deployable = Vec::new();
    for program in 0..NUM_PROGRAMS {
        if program < seeded {
            let owner = owner(u)?;
            if rules::can_deploy_over(Some(owner)) {
                deployable.push(program);
            }
            // Rarely, since a program which never loads takes the rest of the
            // scenario's ops out of play with it.
            let verifies = u.int_in_range(0..=7u8)? != 0;
            seeds.push(Seed {
                program,
                owner,
                verifies,
            });
            slots.push(0);
            continue;
        }
        deployable.push(program);
        let at = pick_low(u, live)?;
        slots.push(at);
        ops.push(Op::Deploy { program, at });
    }
    ops.sort_by_key(|op| match op {
        Op::Deploy { at, .. } => *at,
        _ => 0,
    });
    Ok(SeededDeployments {
        seeds,
        ops,
        slots,
        deployable,
    })
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
    deployable: &[u8],
    sealed: &BTreeSet<Slot>,
) -> Result<Option<Vec<Op>>> {
    let candidates: Vec<Race> = race_candidates(tree, root)
        .into_iter()
        .filter(|race| {
            rules::can_write(sealed, race.branch) && rules::can_write(sealed, race.doomed)
        })
        .collect();
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
    let program = pick(u, deployable)?;
    // Asked for by the preparation phase instead, so the entry it strands is
    // built for the environment which is coming rather than the one in use.
    let prepared: bool = u.arbitrary()?;
    // One in three strands a second program on the same doomed fork, so two
    // orphans land from one prune.
    let second: Option<u8> = (u.int_in_range(0..=2u8)? == 0)
        .then(|| deployable.iter().copied().find(|other| *other != program))
        .flatten();

    let mut ops = vec![Op::Deploy {
        program,
        at: branch,
    }];
    if let Some(other) = second {
        ops.push(Op::Deploy {
            program: other,
            at: branch,
        });
    }
    ops.push(Op::Deploy {
        program,
        at: doomed,
    });
    if let Some(other) = second {
        ops.push(Op::Deploy {
            program: other,
            at: doomed,
        });
    }
    ops.extend([if prepared {
        Op::RecompileForEpoch {
            program,
            fork_tip: doomed_tip,
        }
    } else {
        Op::Extract {
            programs: vec![program],
            fork_tip: doomed_tip,
        }
    }]);
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
            // The batch which asks for the load has to come *after* the
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
    deployable: &[u8],
    sealed: &BTreeSet<Slot>,
) -> Result<Option<Vec<Op>>> {
    let slots = live_slots(tree, root);
    if slots.len() < 3 {
        return Ok(None);
    }
    // One fork, so every deployment is on the same lineage and prune has to
    // choose between them rather than discard them as orphans.
    let lineage_of = |tip: Slot| -> Vec<Slot> {
        tree.ancestry(tip)
            .into_iter()
            .filter(|slot| *slot != 0)
            .collect()
    };
    let tips: Vec<Slot> = slots
        .iter()
        .copied()
        .filter(|tip| {
            let lineage = lineage_of(*tip);
            lineage.len() >= 3 && lineage.iter().all(|at| rules::can_write(sealed, *at))
        })
        .collect();
    if tips.is_empty() {
        return Ok(None);
    }
    let tip = pick(u, &tips)?;
    let lineage = lineage_of(tip);

    let program = pick(u, deployable)?;
    let mut ops: Vec<Op> = lineage
        .iter()
        .map(|at| Op::Deploy { program, at: *at })
        .collect();

    // Sometimes one more version of the same program on a fork the lineage
    // does not include.
    let sibling: bool = u.arbitrary()?;
    if sibling
        && let Some(at) = slots
            .iter()
            .copied()
            .find(|slot| !lineage.contains(slot) && rules::can_write(sealed, *slot))
    {
        ops.push(Op::Deploy { program, at });
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

fn random_op(
    u: &mut Unstructured<'_>,
    live: &[Slot],
    searchable: &[Slot],
    deployable: &[u8],
    writable: &[Slot],
) -> Result<Op> {
    // Weights:
    //
    //   0, 1  Deploy              2/10
    //   2     Close               1/10
    //   3, 4  Extract             2/10
    //   5     FinishLoad          1/10
    //   6, 7  Prune               2/10
    //   8     RecompileForEpoch   1/10
    //   9     PurgeSlot           1/10
    //
    let op = match u.int_in_range(0..=9)? {
        0 | 1 if !writable.is_empty() => Op::Deploy {
            program: pick(u, deployable)?,
            at: pick(u, writable)?,
        },
        2 if !writable.is_empty() => Op::Close {
            program: pick(u, deployable)?,
            at: pick(u, writable)?,
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
        6 | 7 => Op::Prune {
            root: pick(u, live)?,
        },
        8 => Op::RecompileForEpoch {
            program: program(u)?,
            fork_tip: pick(u, or_live(searchable, live))?,
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

fn owner(u: &mut Unstructured<'_>) -> Result<ProgramCacheEntryOwner> {
    Ok(match u.int_in_range(0..=3)? {
        0 => ProgramCacheEntryOwner::LoaderV1,
        1 => ProgramCacheEntryOwner::LoaderV2,
        2 => ProgramCacheEntryOwner::LoaderV3,
        _ => ProgramCacheEntryOwner::LoaderV4,
    })
}
