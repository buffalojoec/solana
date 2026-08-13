//! Harness constants.

use {
    solana_native_token::LAMPORTS_PER_SOL,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner,
    solana_pubkey::Pubkey,
    solana_sdk_ids::{
        bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable, compute_budget, system_program,
        vote, zk_elgamal_proof_program, zk_token_proof_program,
    },
};

/// The loader owning every non-builtin entry.
pub(crate) const DEFAULT_ENTRY_OWNER: ProgramCacheEntryOwner = ProgramCacheEntryOwner::LoaderV2;
/// Number of slots from the root to the canonical forks' head slot.
/// In other words, max fork length.
pub(crate) const FINALITY_SLOTS: u64 = 4;
/// Lamports minted to the genesis mint account.
pub(crate) const MINT_LAMPORTS: u64 = 1_000_000 * LAMPORTS_PER_SOL;
/// Lamports funded to each transaction's fee payer.
pub(crate) const PAYER_LAMPORTS: u64 = LAMPORTS_PER_SOL;
/// Number of slots in each epoch.
pub(crate) const SLOTS_PER_EPOCH: u64 = 32;

/// The builtins every genesis bank installs, which no timeline seeds itself.
pub(crate) const NATIVE_BUILTINS: [Pubkey; 8] = [
    system_program::ID,
    bpf_loader_deprecated::ID,
    bpf_loader::ID,
    bpf_loader_upgradeable::ID,
    compute_budget::ID,
    vote::ID,
    zk_elgamal_proof_program::ID,
    zk_token_proof_program::ID,
];
