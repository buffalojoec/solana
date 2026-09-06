//! Independently tracks what the account state on each fork is expected to say
//! about a program.
//!
//! This allows us to catch potential bugs wherein a fork's account state shows
//! something unexpected given the ops that were performed on the fork graph.
//! It can also allow us to do deep-comparison on entries that might have
//! landed in equivocated slots.

use {
    crate::{entry::EntryKind, scenario::ForkTree},
    solana_clock::Slot,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountState {
    pub deployment_slot: Slot,
    pub owner: ProgramCacheEntryOwner,
    pub env: u8,
    pub kind: EntryKind,
}

#[derive(Default)]
pub struct Ledger {
    deployments: Vec<(u8, Slot, AccountState)>,
}

impl Ledger {
    /// Reports what the ledger says the account state *should* be for a
    /// program, given the provided fork tree.
    ///
    /// Essentially locates the fork tip in the tree and then queries its own
    /// ledger for the deployment that account state should reflect.
    ///
    /// Returns `None` if the program was never deployed on this fork.
    pub fn account_state(
        &self,
        tree: &ForkTree,
        program: u8,
        fork_tip: Slot,
    ) -> Option<AccountState> {
        let ancestry = tree.ancestry(fork_tip);
        self.deployments
            .iter()
            .filter(|(at_program, at_slot, _)| *at_program == program && ancestry.contains(at_slot))
            .max_by_key(|(_, at_slot, _)| *at_slot)
            .map(|(_, _, state)| *state)
    }

    pub fn deploy(&mut self, program: u8, slot: Slot, state: AccountState) -> bool {
        if self.deployed_at(program, slot).is_some() {
            return false;
        }
        self.deployments.push((program, slot, state));
        true
    }

    pub fn deployed_at(&self, program: u8, slot: Slot) -> Option<AccountState> {
        self.deployments
            .iter()
            .find(|(at_program, at_slot, _)| *at_program == program && *at_slot == slot)
            .map(|(_, _, state)| *state)
    }

    pub fn purge(&mut self, slot: Slot) {
        self.deployments.retain(|(_, at_slot, _)| *at_slot != slot);
    }
}
