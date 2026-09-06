//! # V2: Production Runner
//!
//! Unlike the V1 runner, this runner actually executes across production code.
//!
//! This runner drives real `Bank` nodes in a real `BankForks` fork tree,
//! enacts deployments and closures through real transactions, and produces
//! program cache entry extractions through the real transaction processing
//! pipeline by invoking the program with a real transaction.
//!
//! It also uses a real ProgramRuntimeEnvironment, seeded by a select handful
//! of SVMFeatureSet candidates (syscall feature activations).
//!
//! This version is obviously much slower and therefore takes much longer to
//! saturate, but coverage is much richer.

use crate::{
    report::Report,
    scenario::{Op, Scenario},
};

pub struct Harness;

impl super::Runner for Harness {
    fn new(_scenario: &Scenario) -> Self {
        todo!("v2: build a `BankForks` from the scenario's tree")
    }

    fn step(&mut self, _op: &Op) {
        todo!("v2: apply the op as transactions against the bank at its slot")
    }

    fn finish(self) -> Report {
        todo!("v2: fingerprint the bank's cache and collect what the run saw")
    }
}
