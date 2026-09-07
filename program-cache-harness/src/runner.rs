//! Drive a [`Scenario`] against the global program cache.

pub mod v1;
pub mod v2;

use {
    crate::{
        entry::Entry,
        invariants::{Severity, Violation},
        report::Report,
        scenario::{Op, Scenario},
    },
    solana_clock::Slot,
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

// Run a scenario against both runners.
pub fn run_both(scenario: &Scenario) -> Report {
    let mut report = run::<v1::Harness>(scenario);
    let second = run::<v2::Harness>(scenario);
    let (modelled, driven) = (asked(&report), asked(&second));
    if modelled != driven {
        report.violations.push(Violation {
            invariant: "runners-agree",
            severity: Severity::Critical,
            detail: format!("the runners asked differently:\n{modelled:#?}\n---\n{driven:#?}"),
        });
    }
    // Whatever the second runner found is a finding either way.
    report.violations.extend(second.violations);
    report
}

#[derive(Debug, PartialEq, Eq)]
struct Asked {
    program: u8,
    batch_slot: Slot,
    deployment_slot: Slot,
}

fn asked(report: &Report) -> Vec<Asked> {
    report
        .extractions
        .iter()
        .map(|extraction| Asked {
            program: extraction.program,
            batch_slot: extraction.batch_slot,
            deployment_slot: extraction.asked_for,
        })
        .collect()
}

fn render(fingerprint: &[Entry]) -> String {
    fingerprint
        .iter()
        .map(Entry::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}
