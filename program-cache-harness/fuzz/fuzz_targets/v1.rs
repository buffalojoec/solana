//! V1 target.

#![no_main]

use {
    libfuzzer_sys::fuzz_target,
    solana_program_cache_harness::{Scenario, V1, run_twice},
};

fuzz_target!(|scenario: Scenario| {
    let report = run_twice::<V1>(&scenario);
    assert!(!report.failed(), "{:#?}\n{}", scenario, report.describe());
});
