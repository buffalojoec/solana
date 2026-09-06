//! The fork topology a scenario runs over.

use solana_clock::Slot;

#[derive(Clone, Debug, Default)]
pub struct ForkTree {
    pub forks: Vec<Vec<Slot>>,
}

impl ForkTree {
    pub fn insert_fork(&mut self, fork: &[Slot]) {
        let mut fork = fork.to_vec();
        fork.sort_unstable();
        fork.dedup();
        self.forks.push(fork);
    }

    /// Every slot in the tree, ascending, genesis excluded.
    pub fn slots(&self) -> Vec<Slot> {
        let mut slots: Vec<Slot> = self.forks.iter().flatten().copied().collect();
        slots.sort_unstable();
        slots.dedup();
        slots.retain(|slot| *slot != 0);
        slots
    }

    pub fn parent(&self, slot: Slot) -> Slot {
        self.forks
            .iter()
            .find_map(|fork| {
                let index = fork.iter().position(|at| *at == slot)?;
                Some(index.checked_sub(1).map(|prev| fork[prev]).unwrap_or(0))
            })
            .unwrap_or(0)
    }

    /// The ancestry of `slot`, from genesis up to and including it.
    pub fn ancestry(&self, slot: Slot) -> Vec<Slot> {
        let mut ancestry = vec![slot];
        let mut at = slot;
        while at != 0 {
            at = self.parent(at);
            ancestry.push(at);
        }
        ancestry.reverse();
        ancestry
    }

    pub fn contains(&self, slot: Slot) -> bool {
        slot == 0 || self.forks.iter().any(|fork| fork.contains(&slot))
    }
}

pub fn tree(forks: &[&[Slot]]) -> ForkTree {
    let mut tree = ForkTree::default();
    for fork in forks {
        tree.insert_fork(fork);
    }
    tree
}
