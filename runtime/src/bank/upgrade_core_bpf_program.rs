use {
    super::Bank,
    solana_sdk::{
        account::{Account, AccountSharedData},
        bpf_loader::ID as BPF_LOADER_ID,
        bpf_loader_upgradeable::ID as BPF_LOADER_UPGRADEABLE_ID,
        native_loader::ID as NATIVE_LOADER_ID,
        pubkey::Pubkey,
    },
    std::sync::atomic::Ordering::Relaxed,
    thiserror::Error,
};

/// Errors returned by `replace_account` methods
#[derive(Debug, Error)]
pub enum UpgradeCoreBpfProgramError {
    /// Account not executable
    #[error("Account not executable: {0:?}")]
    AccountNotExecutable(Pubkey),
    /// Account not found
    #[error("Account not found: {0:?}")]
    AccountNotFound(Pubkey),
    /// Incorrect account owner
    #[error("Incorrect account owner for {0:?}")]
    IncorrectOwner(Pubkey),
    /// Program has a data account
    #[error("Data account exists for {0:?}")]
    ProgramHasDataAccount(Pubkey),
}

// Note: This enum is off the hot path until a program migration/upgrade is
// due.
#[allow(dead_code)]
pub(crate) enum NativeProgram {
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
    pub(crate) fn id(&self) -> Pubkey {
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

struct UpgradeConfig {
    program_address: Pubkey,
    program_account: Account,
}

fn get_program_data_address(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[program_id.as_ref()], &BPF_LOADER_UPGRADEABLE_ID).0
}

/// Run checks on a core BPF or native program before performing a migration
/// or upgrade.
///
/// In either case, the program should:
/// * Exist
/// * Consist of only a program account, no program data account
/// * Be owned by the proper account:
///  * BPF programs should be owned by the non-upgradeable loader
///  * Native programs should be owned by the native loader
fn check_program(
    bank: &Bank,
    address: &Pubkey,
    owner: &Pubkey,
) -> Result<UpgradeConfig, UpgradeCoreBpfProgramError> {
    let program_address = *address;
    // The program account should exist
    let program_account: Account = bank
        .get_account_with_fixed_root(&program_address)
        .ok_or(UpgradeCoreBpfProgramError::AccountNotFound(program_address))?
        .into();
    // The program account should be owned by the specified program
    if program_account.owner != *owner {
        return Err(UpgradeCoreBpfProgramError::IncorrectOwner(program_address));
    }
    // The program should be executable
    if !program_account.executable {
        return Err(UpgradeCoreBpfProgramError::AccountNotExecutable(
            program_address,
        ));
    }
    // The program data account should _not_ exist
    let program_data_address = get_program_data_address(&program_address);
    if bank
        .get_account_with_fixed_root(&program_data_address)
        .is_some()
    {
        return Err(UpgradeCoreBpfProgramError::ProgramHasDataAccount(
            program_address,
        ));
    }
    Ok(UpgradeConfig {
        program_address,
        program_account,
    })
}

fn move_program(bank: &Bank, source: UpgradeConfig, target: UpgradeConfig) {
    // Burn lamports in the target program account
    bank.capitalization
        .fetch_sub(target.program_account.lamports, Relaxed);
    // Transfer source program account to target program account, clear the
    // source data account, and update the accounts data size delta
    let (old_data_size, new_data_size) = (
        target.program_account.data.len(),
        source.program_account.data.len(),
    );
    bank.store_account(&target.program_address, &source.program_account);
    bank.store_account(&source.program_address, &AccountSharedData::default());
    bank.calculate_and_update_accounts_data_size_delta_off_chain(old_data_size, new_data_size);
    // Unload the programs from the bank's cache
    bank.loaded_programs_cache
        .write()
        .unwrap()
        .remove_programs([source.program_address, target.program_address].into_iter());
}

/// Migrate a native program to BPF using a BPF version of the program,
/// deployed at some arbitrary address.
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
pub(crate) fn migrate_native_program_to_core_bpf(
    bank: &Bank,
    target: NativeProgram,
    source_address: &Pubkey,
    datapoint_name: &'static str,
) -> Result<(), UpgradeCoreBpfProgramError> {
    // Source should be a BPF program owned by the non-upgradeable loader
    // Target should be a native program owned by the native loader
    let source = check_program(bank, source_address, &BPF_LOADER_ID)?;
    let target = check_program(bank, &target.id(), &NATIVE_LOADER_ID)?;
    datapoint_info!(datapoint_name, ("slot", bank.slot, i64));
    move_program(bank, source, target);
    Ok(())
}

/// Upgrade a core BPF program using a modified version of the program,
/// deployed at some arbitrary address.
///
/// This function will move the modified BPF program in place of the existing
/// program by replacing the account at the existing program's address with the
/// modified program's account.
///
/// This function performs a complete overwrite of the account, including the
/// owner program. The strict requirements for this swap can be found in each
/// "check" function.
// Note: This function is off the hot path until a program upgrade is due.
#[allow(dead_code)]
pub(crate) fn upgrade_core_bpf_program(
    bank: &Bank,
    target: NativeProgram,
    source_address: &Pubkey,
    datapoint_name: &'static str,
) -> Result<(), UpgradeCoreBpfProgramError> {
    // Source should be a BPF program owned by the non-upgradeable loader
    // Target should be a BPF program owned by the non-upgradeable loader
    let source = check_program(bank, source_address, &BPF_LOADER_ID)?;
    let target = check_program(bank, &target.id(), &BPF_LOADER_ID)?;
    datapoint_info!(datapoint_name, ("slot", bank.slot, i64));
    move_program(bank, source, target);
    Ok(())
}
