//! The program ELFs to benchmark against.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub const NOOP_ALIGNED: &[u8] =
    include_bytes!("../../programs/bpf_loader/test_elfs/out/noop_aligned.so");

/// Points at a directory of cluster programs dumped by
/// `solana program dump <PROGRAM_ID> <FILE>`, sorted into `v0` and `v3`
/// subdirectories by SBPF version.
const PROGRAMS_DIR_VAR: &str = "AGAVE_BENCH_PROGRAMS_DIR";

pub struct Program {
    pub name: String,
    pub elf: Vec<u8>,
}

fn collect(dir: &Path, prefix: &str, programs: &mut Vec<Program>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("cannot read directory {}: {err}", dir.display()));
    for entry in entries {
        let path = entry.expect("cannot read directory entry").path();
        let stem = path
            .file_stem()
            .expect("directory entry has a file stem")
            .to_string_lossy();
        if path.is_dir() {
            collect(&path, &format!("{prefix}{stem}_"), programs);
        } else if path.extension().is_some_and(|extension| extension == "so") {
            let name = format!("{prefix}{stem}");
            let elf = fs::read(&path).unwrap_or_else(|err| panic!("cannot read {name}: {err}"));
            programs.push(Program { name, elf });
        }
    }
}

/// `noop_aligned`, plus every `*.so` under the programs directory. Corpus
/// programs are named after their path relative to that directory, so
/// `v0/<program_id>.so` is named `v0_<program_id>`.
pub fn programs() -> Vec<Program> {
    let mut programs = vec![Program {
        name: "noop_aligned".to_string(),
        elf: NOOP_ALIGNED.to_vec(),
    }];

    let Some(dir) = env::var_os(PROGRAMS_DIR_VAR).map(PathBuf::from) else {
        eprintln!(
            "{PROGRAMS_DIR_VAR} is unset, benchmarking `noop_aligned` only. Point it at a \
             directory of program ELFs.",
        );
        return programs;
    };

    let mut corpus = Vec::new();
    collect(&dir, "", &mut corpus);
    // Sorted so that benchmark ids are stable from run to run.
    corpus.sort_by(|left, right| left.name.cmp(&right.name));
    programs.append(&mut corpus);
    programs
}
