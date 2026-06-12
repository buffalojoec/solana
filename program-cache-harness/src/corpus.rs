use {agave_feature_set as feature_set, solana_pubkey::Pubkey};

pub const NOOP_SBPF_V0_ELF: &[u8] =
    include_bytes!("../../programs/bpf_loader/test_elfs/out/noop_aligned.so");

/// The environment-changing feature epoch boundary preparation scenarios
/// stage mid-epoch.
pub const TRIGGER_FEATURE_ID: Pubkey = feature_set::enable_sha512_syscall::ID;
