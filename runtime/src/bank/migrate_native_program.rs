use {
    super::Bank,
    solana_sdk::{
        account::{Account, AccountSharedData},
        bpf_loader_upgradeable::{UpgradeableLoaderState, ID as BPF_LOADER_UPGRADEABLE_ID},
        native_loader::ID as NATIVE_LOADER_ID,
        pubkey::Pubkey,
    },
    std::sync::atomic::Ordering::Relaxed,
};

pub enum NativeProgram {
    AddressLookupTable,
    BpfLoader,
    BpfLoaderUpgradeable,
    ComputeBudget,
    Config,
    Ed25519,
    FeatureGate,
    NativeLoader,
    Secp256k1,
    System,
    Stake,
    Vote,
    // ZkTokenProof,
}

impl NativeProgram {
    fn id(&self) -> Pubkey {
        match self {
            NativeProgram::AddressLookupTable => solana_sdk::address_lookup_table::program::id(),
            NativeProgram::BpfLoader => solana_sdk::bpf_loader::id(),
            NativeProgram::BpfLoaderUpgradeable => solana_sdk::bpf_loader_upgradeable::id(),
            NativeProgram::ComputeBudget => solana_sdk::compute_budget::id(),
            NativeProgram::Config => solana_sdk::config::program::id(),
            NativeProgram::Ed25519 => solana_sdk::ed25519_program::id(),
            NativeProgram::FeatureGate => solana_sdk::feature::id(),
            NativeProgram::NativeLoader => solana_sdk::native_loader::id(),
            NativeProgram::Secp256k1 => solana_sdk::secp256k1_program::id(),
            NativeProgram::System => solana_sdk::system_program::id(),
            NativeProgram::Stake => solana_sdk::stake::program::id(),
            NativeProgram::Vote => solana_sdk::vote::program::id(),
            // NativeProgram::ZkTokenProof => solana_zk_token_proof_program::id(),
        }
    }
}

struct MigrationConfig {
    program_address: Pubkey,
    program_account: Account,
    program_data_address: Pubkey,
    program_data_account: Option<Account>,
}

fn get_program_data_address(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[program_id.as_ref()], &BPF_LOADER_UPGRADEABLE_ID).0
}

/// Run checks on the deployed BPF program intended to replace a native
/// program at its address.
fn check_source(bank: &Bank, source_address: &Pubkey) -> MigrationConfig {
    let program_address = *source_address;
    // The program account should exist
    let program_account: Account = bank
        .get_account_with_fixed_root(&program_address)
        .unwrap()
        .into();
    // The program account should be owned by the upgradeable loader
    if program_account.owner != BPF_LOADER_UPGRADEABLE_ID {
        panic!();
    }
    // The program account should hold the address of the program data account
    let program_data_address = get_program_data_address(&program_address);
    let program_account_data =
        bincode::deserialize::<UpgradeableLoaderState>(&program_account.data).unwrap();
    match program_account_data {
        UpgradeableLoaderState::Program {
            programdata_address,
        } => {
            if programdata_address != program_data_address {
                panic!();
            }
        }
        _ => panic!(),
    }
    // The program data account should exist
    let program_data_account: Account = bank
        .get_account_with_fixed_root(&program_data_address)
        .unwrap()
        .into();
    // The program data account should be owned by the upgradeable loader
    if program_data_account.owner != BPF_LOADER_UPGRADEABLE_ID {
        panic!();
    }
    let program_data_account = Some(program_data_account);
    MigrationConfig {
        program_address,
        program_account,
        program_data_address,
        program_data_account,
    }
}

/// Run checks on the target native program to be replaced by a deployed BPF
/// program.
fn check_target(bank: &Bank, target: NativeProgram) -> MigrationConfig {
    let program_address = target.id();
    // The program account should exist
    let program_account: Account = bank
        .get_account_with_fixed_root(&program_address)
        .unwrap()
        .into();
    // The program account should be owned by the native loader
    if program_account.owner != NATIVE_LOADER_ID {
        panic!();
    }
    // The program data account should _not_ exist
    let program_data_address = get_program_data_address(&program_address);
    if bank
        .get_account_with_fixed_root(&program_data_address)
        .is_some()
    {
        panic!();
    }
    MigrationConfig {
        program_address,
        program_account,
        program_data_address,
        program_data_account: None,
    }
}

fn move_program(bank: &Bank, source: MigrationConfig, target: MigrationConfig) {
    // Burn lamports in the target program account
    bank.capitalization
        .fetch_sub(target.program_account.lamports, Relaxed);
    // If a target program data account was provided, burn lamports in the
    // target program data account
    if let Some(target_program_data_account) = target.program_data_account {
        bank.capitalization
            .fetch_sub(target_program_data_account.lamports, Relaxed);
    }
    // Transfer source program data account to target program data account,
    // clear the source program data account, and update the accounts data size
    // delta
    let source_program_data_account = source.program_data_account.unwrap();
    let (old_data_size, new_data_size) = (0, source_program_data_account.data.len());
    bank.store_account(&target.program_data_address, &source_program_data_account);
    bank.store_account(&source.program_data_address, &AccountSharedData::default());
    bank.calculate_and_update_accounts_data_size_delta_off_chain(old_data_size, new_data_size);
    // Transfer source program account to target program account, clear the
    // source data account, and update the accounts data size delta
    let (old_data_size, new_data_size) = (
        target.program_account.data.len(),
        source.program_account.data.len(),
    );
    bank.store_account(&target.program_address, &source.program_account);
    bank.store_account(&source.program_address, &AccountSharedData::default());
    bank.calculate_and_update_accounts_data_size_delta_off_chain(old_data_size, new_data_size);
}

/// Migrate a native program to BPF using a deployed BPF version of the
/// program.
///
/// This function will move the deployed BPF program in place of the native
/// program by replacing the account at the native program's address with the
/// deployed BPF program's account.
///
/// This function performs a complete overwrite of the account, including the
/// owner program. The strict requirements for this swap can be found in each
/// "check" function.
// Note: This function is off the hot path until a program migration is due.
#[allow(dead_code)]
pub fn migrate_native_program(bank: &Bank, target: NativeProgram, source_address: &Pubkey) {
    let source = check_source(bank, source_address);
    let target = check_target(bank, target);
    move_program(bank, source, target);
}
