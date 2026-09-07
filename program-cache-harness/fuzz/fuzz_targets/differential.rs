//! Differential testing across runners.

#![no_main]

use {
    libfuzzer_sys::fuzz_target,
    solana_program_cache_harness::{Scenario, run_both},
};

fuzz_target!(|scenario: Scenario| {
    let report = run_both(&scenario);
    assert!(!report.failed(), "{:#?}\n{}", scenario, report.describe(),);
});
