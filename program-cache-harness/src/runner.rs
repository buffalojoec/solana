//! Drive a [`Scenario`] against the global program cache.

pub mod v1;
pub mod v2;

use crate::{
    entry::Entry,
    invariants::{Severity, Violation},
    report::Report,
    scenario::{Op, Scenario},
};

pub trait Runner: Sized {
    fn new(scenario: &Scenario) -> Self;
    fn step(&mut self, op: &Op);
    fn finish(self) -> Report;
}

pub fn run<R: Runner>(scenario: &Scenario) -> Report {
    let mut runner = R::new(scenario);
    for op in &scenario.ops {
        runner.step(op);
    }
    runner.finish()
}

/// Run a scenario twice against fresh caches and report any divergence.
pub fn run_twice<R: Runner>(scenario: &Scenario) -> Report {
    let first = run::<R>(scenario);
    let second = run::<R>(scenario);
    let mut report = first;
    if report.fingerprint != second.fingerprint {
        report.violations.push(Violation {
            invariant: "deterministic",
            severity: Severity::Critical,
            detail: format!(
                "two runs of one scenario left different contents:\n{}\n---\n{}",
                render(&report.fingerprint),
                render(&second.fingerprint)
            ),
        });
    }
    report
}

fn render(fingerprint: &[Entry]) -> String {
    fingerprint
        .iter()
        .map(Entry::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}
