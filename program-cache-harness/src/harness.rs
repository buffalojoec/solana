//! The in-out harness.

use crate::{genesis::Genesis, runtime::TestRuntime, timeline::Frame};

/// Run `timeline` against the environment prepared by `genesis`.
pub fn run(genesis: Genesis, timeline: Vec<Frame>) {
    let mut test = TestRuntime::new_from_genesis(genesis);
    timeline.into_iter().for_each(|frame| test.frame(frame));
}
