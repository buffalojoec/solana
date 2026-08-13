//! The in-out harness.

use crate::{entry::Entry, genesis::Genesis, runtime::TestRuntime, timeline::Timeline};

/// Run `timeline` against the environment prepared by `genesis`, returning the
/// contents of the global program cache after each step.
pub fn run(genesis: Genesis, timeline: Timeline) -> Vec<Vec<Entry>> {
    let mut test = TestRuntime::new_from_genesis(genesis);
    timeline
        .steps
        .into_iter()
        .map(|step| test.step(step))
        .collect()
}
