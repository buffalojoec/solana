//! Rules for an acceptable op.

use {
    crate::scenario::{ForkTree, Op},
    solana_clock::Slot,
};

/// Whether a slot still has a bank, determined from the tree and the root.
pub fn is_live(tree: &ForkTree, root: Slot, slot: Slot) -> bool {
    tree.contains(slot) && tree.ancestry(slot).contains(&root)
}

/// A deployment lands only on a live slot above the root. A rooted slot is
/// already replayed, and an abandoned branch has no bank to deploy on.
pub fn can_deploy(live: bool, at: Slot, root: Slot) -> bool {
    live && at > root
}

/// No bank, no batch. Replay abandons a branch the root left behind, so
/// nothing executes there again.
pub fn can_extract(live: bool) -> bool {
    live
}

/// Only an unrooted block on a live branch can be dumped.
pub fn can_dump(live: bool, slot: Slot, root: Slot) -> bool {
    live && slot > root
}

/// The root cannot move backwards, nor onto a branch which is already gone.
pub fn can_root(live: bool, new_root: Slot, root: Slot) -> bool {
    live && new_root >= root
}

/// Where an op leaves the root, so the generator tracks what the runner does.
pub fn advance_root(tree: &ForkTree, root: Slot, op: &Op) -> Slot {
    match op {
        Op::Prune { root: moved_to } | Op::CrossEpochBoundary { root: moved_to } => {
            if can_root(is_live(tree, root, *moved_to), *moved_to, root) {
                *moved_to
            } else {
                root
            }
        }
        _ => root,
    }
}
