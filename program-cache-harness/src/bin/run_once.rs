//! Single-invocation entrypoint: runs the spike scenario once.
//!
//! Without `shuttle-test` this is one plain run with std threads. With it,
//! one execution under shuttle's random scheduler.

use agave_program_cache_harness::{runner::run_epoch_boundary_preparation, scenario::Scenario};

#[cfg(not(feature = "shuttle-test"))]
fn main() {
    agave_logger::setup();
    println!("{:#?}", run_epoch_boundary_preparation(&Scenario::spike()));
}

#[cfg(feature = "shuttle-test")]
fn main() {
    agave_logger::setup();
    shuttle::check_random(
        || println!("{:#?}", run_epoch_boundary_preparation(&Scenario::spike())),
        1,
    );
}
