//! V2 target.

#![no_main]

use {
    libfuzzer_sys::fuzz_target,
    solana_program_cache_harness::{Scenario, V2, run_twice},
};

fuzz_target!(|scenario: Scenario| {
    let report = run_twice::<V2>(&scenario);
    assert!(!report.failed(), "{:#?}\n{}", scenario, report.describe(),);
});
