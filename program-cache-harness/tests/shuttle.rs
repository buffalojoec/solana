#![cfg(feature = "shuttle-test")]

use agave_program_cache_harness::{runner::run_epoch_boundary_preparation, scenario::Scenario};

#[test]
fn test_epoch_boundary_preparation_spike_random_scheduler() {
    shuttle::check_random(
        || {
            let report = run_epoch_boundary_preparation(&Scenario::spike());
            assert!(report.saw_recompiled_entry);
        },
        3,
    );
}
