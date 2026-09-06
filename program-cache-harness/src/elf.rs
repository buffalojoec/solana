//! Test ELFs.

pub const NOOP_OK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../programs/bpf_loader/test_elfs/out/sbpfv3_return_ok.so"
));
