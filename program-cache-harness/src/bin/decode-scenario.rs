//! Decodes a fuzzer artifact back into a [`Scenario`], for triage.
//!
//! ```text
//! cargo run -p solana-program-cache-harness --bin decode-scenario -- path/to/crash-...
//! ```

use {
    arbitrary::{Arbitrary, Unstructured},
    solana_program_cache_harness::Scenario,
};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: decode-scenario <artifact>");
        std::process::exit(2);
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{path}: {error}");
            std::process::exit(1);
        }
    };
    match Scenario::arbitrary(&mut Unstructured::new(&bytes)) {
        Ok(scenario) => println!("{scenario:#?}"),
        Err(error) => {
            eprintln!("{path}: {error}");
            std::process::exit(1);
        }
    }
}
