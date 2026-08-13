//! The in-out harness.

use crate::{genesis::Genesis, runtime::TestRuntime, timeline::Timeline};

/// Run `timeline` against the environment prepared by `genesis`.
pub fn run(genesis: Genesis, timeline: Timeline) {
    let mut test = TestRuntime::new_from_genesis(genesis);
    timeline.steps.into_iter().for_each(|step| test.step(step));
}
