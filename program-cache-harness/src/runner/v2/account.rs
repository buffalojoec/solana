//! Account builders.

use {
    solana_account::{AccountSharedData, ReadableAccount as _, WritableAccount as _},
    solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState},
    solana_loader_v4_interface::state::LoaderV4Status,
    solana_program_runtime::program_cache_entry::ProgramCacheEntryOwner,
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_sdk_ids::{
        bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable, loader_v4, native_loader,
        system_program, sysvar,
    },
    solana_svm::program_loader::test_utils,
};

const PAYER_LAMPORTS: u64 = 1_000_000_000_000_000;

pub fn genesis(payer: &Pubkey, pending: &Pubkey) -> Vec<(Pubkey, AccountSharedData)> {
    vec![
        (*payer, self::payer()),
        // Pending from genesis, so it is already pending before any bank is
        // built, therefore priming a new program runtime env at the boundary.
        (*pending, pending_feature()),
        (sysvar::rent::id(), rent()),
        (system_program::id(), builtin("system_program")),
        (
            bpf_loader_deprecated::id(),
            builtin("solana_bpf_loader_deprecated_program"),
        ),
        (bpf_loader::id(), builtin("solana_bpf_loader_program")),
        (
            bpf_loader_upgradeable::id(),
            builtin("solana_bpf_loader_upgradeable_program"),
        ),
        (loader_v4::id(), builtin("solana_loader_v4_program")),
    ]
}

pub fn buffer(authority: &Pubkey, elf: &[u8]) -> AccountSharedData {
    let header = UpgradeableLoaderState::size_of_buffer_metadata();
    let mut account = rent_exempt(
        header.saturating_add(elf.len()),
        &bpf_loader_upgradeable::id(),
    );
    bincode::serialize_into(
        account.data_as_mut_slice(),
        &UpgradeableLoaderState::Buffer {
            authority_address: Some(*authority),
        },
    )
    .expect("serialize buffer state");
    account.data_as_mut_slice()[header..].copy_from_slice(elf);
    account
}

pub fn deployed_under(
    owner: ProgramCacheEntryOwner,
    program: &Pubkey,
    authority: &Pubkey,
    elf: &[u8],
) -> Vec<(Pubkey, AccountSharedData)> {
    match owner {
        ProgramCacheEntryOwner::LoaderV1 => {
            let account = rent_exempt_elf(elf, &bpf_loader_deprecated::id());
            vec![(*program, account)]
        }
        ProgramCacheEntryOwner::LoaderV2 => {
            let account = rent_exempt_elf(elf, &bpf_loader::id());
            vec![(*program, account)]
        }
        ProgramCacheEntryOwner::LoaderV3 => {
            let programdata_address = get_program_data_address(program);
            let account = test_utils::loader_v3_program_account(programdata_address);
            let programdata = test_utils::loader_v3_programdata_account(0, Some(*authority), elf);
            vec![
                (*program, funded(account)),
                (programdata_address, funded(programdata)),
            ]
        }
        ProgramCacheEntryOwner::NativeLoader => Vec::new(),
        ProgramCacheEntryOwner::LoaderV4 => {
            let account = test_utils::loader_v4_account(0, LoaderV4Status::Deployed, elf);
            vec![(*program, funded(account))]
        }
    }
}

fn builtin(name: &str) -> AccountSharedData {
    let mut account = AccountSharedData::new(1, name.len(), &native_loader::id());
    account.data_as_mut_slice().copy_from_slice(name.as_bytes());
    account.set_executable(true);
    account
}

fn pending_feature() -> AccountSharedData {
    solana_feature_gate_interface::create_account(
        &solana_feature_gate_interface::Feature { activated_at: None },
        Rent::default()
            .minimum_balance(solana_feature_gate_interface::Feature::size_of())
            .max(1),
    )
}

fn payer() -> AccountSharedData {
    AccountSharedData::new(PAYER_LAMPORTS, 0, &system_program::id())
}

fn rent() -> AccountSharedData {
    let rent = Rent::default();
    let mut account = AccountSharedData::new(
        1,
        wincode::serialized_size(&rent).unwrap() as usize,
        &sysvar::id(),
    );
    wincode::serialize_into(account.data_as_mut_slice(), &rent).unwrap();
    account
}

fn funded(mut account: AccountSharedData) -> AccountSharedData {
    account.set_lamports(Rent::default().minimum_balance(account.data().len()).max(1));
    account
}

fn rent_exempt(size: usize, owner: &Pubkey) -> AccountSharedData {
    AccountSharedData::new(Rent::default().minimum_balance(size).max(1), size, owner)
}

fn rent_exempt_elf(elf: &[u8], owner: &Pubkey) -> AccountSharedData {
    let mut account = rent_exempt(elf.len(), owner);
    account.data_as_mut_slice().copy_from_slice(elf);
    account.set_executable(true);
    account
}
