//! Nested CPI resource consumption: compute units, peak memory and copied bytes.
//!
//! Prints the measurements and dumps to a file so that a follow-on script can
//! refine the data.

use {
    cpi_increase_bencher::{
        accounts::{DEFAULT_ACCOUNT_DATA_LEN, program_elf},
        harness::Harness,
        scenario::Scenario,
    },
    std::io::Write,
};

const KIB: usize = 1024;
const MIB: usize = KIB * KIB;

fn main() {
    let elf = program_elf();
    let mut rows = Vec::new();

    for account_data_len in [
        DEFAULT_ACCOUNT_DATA_LEN,
        DEFAULT_ACCOUNT_DATA_LEN * 10,
        DEFAULT_ACCOUNT_DATA_LEN * 100,
        DEFAULT_ACCOUNT_DATA_LEN * 1000,
    ] {
        println!("\n=== 2 accounts of {} ===", size(account_data_len));
        println!(
            "{:<10} {:>3} {:>5} {:>10} {:>12} {:>12}",
            "scenario", "frm", "dm", "CUs", "peak_bytes", "copied"
        );
        for (name, depth, levels) in [
            ("64x0", 0u8, vec![0u8; 64]),
            ("32x1", 1, vec![1u8; 32]),
            ("12x4_1x3", 4, [vec![4u8; 12], vec![3]].concat()),
            ("7x8_1x0", 8, [vec![8u8; 7], vec![0]].concat()),
        ] {
            for direct_mapping in [true, false] {
                let scenario = Scenario {
                    track_memory: true,
                    nesting_levels: levels.clone(),
                    account_data_len,
                    direct_mapping,
                };
                let mut harness = Harness::new(&elf, &scenario.feature_set());
                let measurement = harness.measure(&scenario);
                println!(
                    "{name:<10} {:>3} {:>5} {:>10} {:>12} {:>12}",
                    scenario.frames(),
                    direct_mapping,
                    measurement.compute_units,
                    measurement.peak_mapped_bytes,
                    measurement.copied_bytes,
                );
                rows.push(format!(
                    r#"{{"scenario":"{name}","depth":{depth},"account_data_len":{account_data_len},"instructions":{},"cpis":{},"frames":{},"direct_mapping":{direct_mapping},"compute_units":{},"peak_mapped_bytes":{},"copied_bytes":{}}}"#,
                    levels.len(),
                    levels.iter().map(|level| *level as usize).sum::<usize>(),
                    scenario.frames(),
                    measurement.compute_units,
                    measurement.peak_mapped_bytes,
                    measurement.copied_bytes,
                ));
            }
        }
    }

    let path = std::env::args()
        .nth(1)
        .expect("usage: resources <report.json>");
    let mut file = std::fs::File::create(&path).expect("failed to create report");
    writeln!(file, "[{}]", rows.join(",")).expect("failed to write report");
}

fn size(bytes: usize) -> String {
    if bytes >= MIB {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{} KiB", bytes / KIB)
    }
}
