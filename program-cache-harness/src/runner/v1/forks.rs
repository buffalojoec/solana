//! Simulated fork graph. Mirrors `BankForks`.

use {
    crate::scenario::ForkTree,
    solana_clock::Slot,
    solana_program_runtime::loaded_programs::{BlockRelation, ForkGraph},
    std::{
        collections::{BTreeMap, BTreeSet, btree_map::Entry},
        sync::{Arc, RwLock, RwLockReadGuard, Weak},
    },
};

pub struct Forks {
    graph: Arc<RwLock<Graph>>,
}

impl Forks {
    pub fn new(tree: &ForkTree) -> Self {
        Self {
            graph: Arc::new(RwLock::new(Graph::new(tree))),
        }
    }

    pub fn weak(&self) -> Weak<RwLock<Graph>> {
        Arc::downgrade(&self.graph)
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Graph> {
        self.graph.read().unwrap()
    }

    pub fn contains(&self, slot: Slot) -> bool {
        self.graph.read().unwrap().contains(slot)
    }

    pub fn root(&self) -> Slot {
        self.graph.read().unwrap().root()
    }

    pub fn set_root(&self, slot: Slot) {
        self.graph.write().unwrap().set_root(slot);
    }
}

#[derive(Debug)]
pub struct Graph {
    ancestors: BTreeMap<Slot, BTreeSet<Slot>>,
    descendants: BTreeMap<Slot, BTreeSet<Slot>>,
    root: Slot,
}

impl Graph {
    fn new(tree: &ForkTree) -> Self {
        let mut graph = Self {
            ancestors: BTreeMap::new(),
            descendants: BTreeMap::new(),
            root: 0,
        };
        graph.insert(0, BTreeSet::from([0]));
        for slot in tree.slots() {
            graph.insert(slot, tree.ancestry(slot).into_iter().collect());
        }
        graph
    }

    fn insert(&mut self, slot: Slot, ancestry: BTreeSet<Slot>) {
        self.descendants.entry(slot).or_default();
        for parent in ancestry.iter().filter(|at| **at != slot) {
            self.descendants.entry(*parent).or_default().insert(slot);
        }
        self.ancestors.insert(slot, ancestry);
    }

    pub fn contains(&self, slot: Slot) -> bool {
        self.ancestors.contains_key(&slot)
    }

    pub fn root(&self) -> Slot {
        self.root
    }

    pub fn set_root(&mut self, root: Slot) {
        assert!(self.contains(root), "root bank didn't exist in bank_forks");
        self.root = root;
        let dropped: Vec<Slot> = self
            .ancestors
            .keys()
            .copied()
            .filter(|slot| *slot != root && !self.descendants[&root].contains(slot))
            .collect();
        for slot in dropped {
            self.remove(slot);
        }
    }

    fn remove(&mut self, slot: Slot) {
        let Some(ancestry) = self.ancestors.remove(&slot) else {
            return;
        };
        for parent in ancestry.iter().filter(|at| **at != slot) {
            let Entry::Occupied(mut entry) = self.descendants.entry(*parent) else {
                unreachable!("every ancestor was given an entry when the slot was inserted");
            };
            entry.get_mut().remove(&slot);
            if entry.get().is_empty() && !self.ancestors.contains_key(parent) {
                entry.remove_entry();
            }
        }
        if self.descendants[&slot].is_empty() {
            self.descendants.remove(&slot);
        }
    }

    fn highest(&self) -> Slot {
        *self
            .ancestors
            .keys()
            .next_back()
            .expect("the root always holds a bank")
    }
}

impl ForkGraph for Graph {
    fn relationship(&self, a: Slot, b: Slot) -> BlockRelation {
        let known = self.root..=self.highest();
        if !known.contains(&a) || !known.contains(&b) {
            return BlockRelation::Unknown;
        }
        if a == b {
            return BlockRelation::Equal;
        }
        if self
            .ancestors
            .get(&b)
            .is_some_and(|slots| slots.contains(&a))
        {
            return BlockRelation::Ancestor;
        }
        if self
            .descendants
            .get(&b)
            .is_some_and(|slots| slots.contains(&a))
        {
            return BlockRelation::Descendant;
        }
        BlockRelation::Unrelated
    }
}

#[cfg(test)]
mod tests {
    //! Test that our simulated fork graph behaves like `BankForks`.

    use {
        super::*,
        crate::scenario::tree,
        solana_leader_schedule::SlotLeader,
        solana_runtime::{bank::Bank, bank_forks::BankForks, genesis_utils::create_genesis_config},
        std::sync::RwLock,
    };

    fn bank_forks(tree: &ForkTree) -> Arc<RwLock<BankForks>> {
        let genesis = create_genesis_config(1_000_000).genesis_config;
        let bank_forks = BankForks::new_rw_arc(Bank::new_for_tests(&genesis));
        for slot in tree.slots() {
            let parent = bank_forks
                .read()
                .unwrap()
                .get(tree.parent(slot))
                .expect("parent bank");
            let bank = Bank::new_from_parent(parent, SlotLeader::default(), slot);
            bank_forks.write().unwrap().insert(bank);
        }
        bank_forks
    }

    fn assert_agrees(graph: &Graph, real: &BankForks, span: Slot, whose: &str) {
        assert_eq!(graph.root(), real.root(), "{whose}: root");
        for slot in 0..=span {
            assert_eq!(
                graph.contains(slot),
                real.get(slot).is_some(),
                "{whose}: contains({slot})"
            );
        }
        for a in 0..=span {
            for b in 0..=span {
                assert_eq!(
                    graph.relationship(a, b),
                    real.relationship(a, b),
                    "{whose}: relationship({a}, {b})"
                );
            }
        }
    }

    #[test]
    fn answers_what_bank_forks_answers() {
        let trees = [
            tree(&[]),
            tree(&[&[1]]),
            tree(&[&[1, 2, 3, 4]]),
            tree(&[&[1, 2, 3], &[1, 2, 4, 5]]),
            tree(&[&[1, 2], &[3, 4]]),
            tree(&[&[1, 3, 5, 7], &[1, 3, 6], &[2, 4]]),
        ];
        for tree in &trees {
            let span = tree.slots().last().copied().unwrap_or(0) + 2;

            let real = bank_forks(tree);
            assert_agrees(&Graph::new(tree), &real.read().unwrap(), span, "fresh");

            for root in std::iter::once(0).chain(tree.slots()) {
                let mut graph = Graph::new(tree);
                let real = bank_forks(tree);
                graph.set_root(root);
                real.write().unwrap().set_root(root, None, None);
                assert_agrees(&graph, &real.read().unwrap(), span, &format!("root {root}"));
            }
        }
    }

    #[test]
    fn agrees_as_the_root_walks_up_a_fork() {
        let tree = tree(&[&[1, 2, 3, 4, 5], &[1, 2, 6, 7], &[8, 9]]);
        let mut graph = Graph::new(&tree);
        let real = bank_forks(&tree);
        for root in [1, 2, 3, 5] {
            graph.set_root(root);
            real.write().unwrap().set_root(root, None, None);
            assert_agrees(&graph, &real.read().unwrap(), 11, &format!("root {root}"));
        }
    }
}
