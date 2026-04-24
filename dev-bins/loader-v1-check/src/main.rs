use clap::Parser;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sbpf::{
    ebpf::{self, CALL_IMM, INSN_SIZE},
    elf_parser::Elf64,
};
use solana_pubkey::Pubkey;
use std::{
    collections::HashMap,
    str::FromStr,
    time::Instant,
};

/// Target syscall names, taken from agave/syscalls/src/lib.rs register_function() calls.
const TARGET_SYSCALLS: &[&str] = &[
    "sol_get_clock_sysvar",
    "sol_get_epoch_schedule_sysvar",
    "sol_get_fees_sysvar",
    "sol_get_rent_sysvar",
    "sol_get_last_restart_slot",
    "sol_get_epoch_rewards_sysvar",
    "sol_get_sysvar",
];

const BPFLOADER1: &str = "BPFLoader1111111111111111111111111111111111";

#[derive(Parser)]
#[command(about = "Check BPFLoader1 programs for sysvar syscall references")]
struct Cli {
    /// RPC endpoint URL
    #[arg(long)]
    rpc: String,

    /// Print details for every program, not just matches
    #[arg(long, short)]
    verbose: bool,
}

/// A single detection hit.
struct Hit {
    syscall: &'static str,
    method: String,
    offset: usize,
}

fn main() {
    let cli = Cli::parse();

    // Pre-compute target hashes using the authoritative sbpf hash function.
    let target_hashes: HashMap<u32, &'static str> = TARGET_SYSCALLS
        .iter()
        .map(|name| (ebpf::hash_symbol_name(name.as_bytes()), *name))
        .collect();

    println!("{}", "=".repeat(70));
    println!("Syscall Hash Table (solana_sbpf::ebpf::hash_symbol_name)");
    println!("{}", "=".repeat(70));
    for name in TARGET_SYSCALLS {
        let h = ebpf::hash_symbol_name(name.as_bytes());
        let le = h.to_le_bytes();
        println!(
            "  {:<40} -> 0x{:08X}  LE=[{}]",
            name,
            h,
            le.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join("")
        );
    }
    println!("{}", "=".repeat(70));
    println!();

    // Fetch programs
    println!("Fetching BPFLoader1 programs from {} ...", cli.rpc);
    let client = RpcClient::new(&cli.rpc);
    let loader_pubkey = Pubkey::from_str(BPFLOADER1).unwrap();
    let accounts = client
        .get_program_accounts(&loader_pubkey)
        .expect("Failed to fetch program accounts");

    println!("Found {} program account(s)", accounts.len());
    println!();

    let start = Instant::now();
    let mut matches_by_program: Vec<(Pubkey, Vec<Hit>)> = Vec::new();
    let mut elf_count: usize = 0;
    let mut gifted: usize = 0;
    let mut non_elf_nonzero: usize = 0;

    for (idx, (pubkey, account)) in accounts.iter().enumerate() {
        let data = &account.data;

        let is_elf = data.len() >= 4 && &data[..4] == b"\x7fELF";
        if is_elf {
            elf_count += 1;
        } else {
            if data.iter().all(|&b| b == 0) {
                gifted += 1;
                continue;
            }
            non_elf_nonzero += 1;
        }

        let mut hits = Vec::new();

        // Method A: parse ELF dynamic symbol table for syscall name strings
        if is_elf {
            if let Ok(elf) = Elf64::parse(data) {
                search_dynstr(&elf, &mut hits);
                search_bytecode(&elf, data, &target_hashes, &mut hits);
            } else {
                // ELF parse failed, fall back to raw string search
                search_strings_raw(data, &mut hits);
                scan_instructions_raw(data, &target_hashes, &mut hits);
            }
        } else {
            search_strings_raw(data, &mut hits);
            scan_instructions_raw(data, &target_hashes, &mut hits);
        }

        if !hits.is_empty() {
            matches_by_program.push((*pubkey, hits));
        } else if cli.verbose {
            println!("  [{}/{}] {}: no matches", idx + 1, accounts.len(), pubkey);
        }

        if (idx + 1) % 500 == 0 {
            println!(
                "  Progress: {}/{} programs scanned ...",
                idx + 1,
                accounts.len()
            );
        }
    }

    let elapsed = start.elapsed();

    println!();
    println!("{}", "=".repeat(70));
    println!("RESULTS");
    println!("{}", "=".repeat(70));
    println!(
        "Programs scanned: {}  (ELF: {}, gifted: {}, non-ELF non-zero: {})",
        accounts.len(),
        elf_count,
        gifted,
        non_elf_nonzero
    );
    println!("Scan time: {:.2}s", elapsed.as_secs_f64());
    println!();

    if matches_by_program.is_empty() {
        println!("No programs reference any of the target syscalls.");
    } else {
        println!(
            "Found {} program(s) with target syscall references:",
            matches_by_program.len()
        );
        println!();
        for (pubkey, hits) in &matches_by_program {
            println!("  Program: {}", pubkey);
            // Deduplicate by (syscall, method)
            let mut seen = std::collections::HashSet::new();
            for hit in hits {
                let key = (hit.syscall, &hit.method);
                if seen.insert(key) {
                    println!(
                        "    - {}  (detected via {} at offset 0x{:X})",
                        hit.syscall, hit.method, hit.offset
                    );
                }
            }
            println!();
        }
    }

    println!("{}", "=".repeat(70));
}

/// Method A: walk the ELF dynamic symbol table and check names against targets.
fn search_dynstr(elf: &Elf64, hits: &mut Vec<Hit>) {
    let Some(dynsym) = elf.dynamic_symbol_table() else {
        return;
    };
    for sym in dynsym {
        let Ok(name_bytes) = elf.dynamic_symbol_name(sym.st_name) else {
            continue;
        };
        let Ok(name) = std::str::from_utf8(name_bytes) else {
            continue;
        };
        for target in TARGET_SYSCALLS {
            if name == *target {
                hits.push(Hit {
                    syscall: target,
                    method: "dynstr".to_string(),
                    offset: sym.st_name as usize,
                });
            }
        }
    }
}

/// Method B: scan .text section bytecode for call_imm (0x85) with matching hashes.
fn search_bytecode(
    elf: &Elf64,
    elf_bytes: &[u8],
    target_hashes: &HashMap<u32, &'static str>,
    hits: &mut Vec<Hit>,
) {
    for section_header in elf.section_header_table() {
        let Ok(name) = elf.section_name(section_header.sh_name) else {
            continue;
        };
        if name != b".text" {
            continue;
        }
        let Ok(text_data) =
            Elf64::slice_from_section_header::<u8>(elf_bytes, section_header)
        else {
            continue;
        };
        scan_instructions(text_data, target_hashes, hits);
        return;
    }
}

/// Fallback string search for non-ELF or unparseable data.
fn search_strings_raw(data: &[u8], hits: &mut Vec<Hit>) {
    for target in TARGET_SYSCALLS {
        let target_bytes = target.as_bytes();
        if let Some(pos) = data
            .windows(target_bytes.len())
            .position(|w| w == target_bytes)
        {
            hits.push(Hit {
                syscall: target,
                method: "string".to_string(),
                offset: pos,
            });
        }
    }
}

/// Scan raw bytecode for call_imm instructions with target hashes.
fn scan_instructions(data: &[u8], target_hashes: &HashMap<u32, &'static str>, hits: &mut Vec<Hit>) {
    let num_insns = data.len() / INSN_SIZE;
    for i in 0..num_insns {
        let offset = i * INSN_SIZE;
        if data[offset] == CALL_IMM {
            let imm = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            if let Some(name) = target_hashes.get(&imm) {
                hits.push(Hit {
                    syscall: name,
                    method: "hash/call_imm".to_string(),
                    offset,
                });
            }
        }
    }
}

/// Fallback instruction scan for non-ELF data.
fn scan_instructions_raw(data: &[u8], target_hashes: &HashMap<u32, &'static str>, hits: &mut Vec<Hit>) {
    scan_instructions(data, target_hashes, hits);
}