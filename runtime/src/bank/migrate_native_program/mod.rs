mod bpf_program;
mod bpf_upgradeable_program;
pub(crate) mod error;
mod native_program;

use {
    self::{
        bpf_program::BpfProgramConfig, error::MigrateNativeProgramError,
        native_program::NativeProgramConfig,
    },
    super::Bank,
    crate::bank::migrate_native_program::bpf_upgradeable_program::BpfUpgradeableProgramConfig,
    solana_sdk::{
        account::{Account, AccountSharedData},
        bpf_loader_upgradeable::{UpgradeableLoaderState, ID as BPF_LOADER_UPGRADEABLE_ID},
        pubkey::Pubkey,
    },
    std::sync::atomic::Ordering::Relaxed,
};

/// Enum representing the native programs that can be migrated to BPF
/// programs
#[allow(dead_code)] // Code is off the hot path until a migration is due
#[derive(Clone, Copy)]
pub(crate) enum NativeProgram {
    AddressLookupTable,
    BpfLoader,
    BpfLoaderUpgradeable,
    ComputeBudget,
    Config,
    Ed25519,
    FeatureGate,
    LoaderV4,
    NativeLoader,
    Secp256k1,
    Stake,
    System,
    Vote,
    ZkTokenProof,
}
impl NativeProgram {
    /// The program ID of the native program
    pub(crate) fn id(&self) -> Pubkey {
        match self {
            Self::AddressLookupTable => solana_sdk::address_lookup_table::program::id(),
            Self::BpfLoader => solana_sdk::bpf_loader::id(),
            Self::BpfLoaderUpgradeable => solana_sdk::bpf_loader_upgradeable::id(),
            Self::ComputeBudget => solana_sdk::compute_budget::id(),
            Self::Config => solana_sdk::config::program::id(),
            Self::Ed25519 => solana_sdk::ed25519_program::id(),
            Self::FeatureGate => solana_sdk::feature::id(),
            Self::LoaderV4 => solana_sdk::loader_v4::id(),
            Self::NativeLoader => solana_sdk::native_loader::id(),
            Self::Secp256k1 => solana_sdk::secp256k1_program::id(),
            Self::Stake => solana_sdk::stake::program::id(),
            Self::System => solana_sdk::system_program::id(),
            Self::Vote => solana_sdk::vote::program::id(),
            Self::ZkTokenProof => solana_zk_token_sdk::zk_token_proof_program::id(),
        }
    }

    /// Returns whether or not a native program's program account is synthetic,
    /// meaning it does not actually exist, rather its address is used as an
    /// owner for other accounts
    fn is_synthetic(&self) -> bool {
        match self {
            Self::FeatureGate
            | Self::LoaderV4 // Not deployed
            | Self::NativeLoader
            | Self::ZkTokenProof // Not deployed
            => true,
            _ => false,
        }
    }
}

/// Migrate a native program to a BPF (non-upgradeable) program using a BPF
/// version of the program deployed at some arbitrary address.
#[allow(dead_code)] // Code is off the hot path until a migration is due
pub(crate) fn migrate_native_program_to_bpf_non_upgradeable(
    bank: &mut Bank,
    target_program: NativeProgram,
    source_program_address: &Pubkey,
    datapoint_name: &'static str,
) -> Result<(), MigrateNativeProgramError> {
    datapoint_info!(datapoint_name, ("slot", bank.slot, i64));

    let target = NativeProgramConfig::new_checked(bank, target_program)?;
    let source = BpfProgramConfig::new_checked(bank, source_program_address)?;

    // Burn lamports from the target program account
    bank.capitalization
        .fetch_sub(target.program_account.lamports, Relaxed);

    // Copy the non-upgradeable BPF program's account into the native program's
    // address, then clear the source BPF program account
    bank.store_account(&target.program_address, &source.program_account);
    bank.store_account(&source.program_address, &AccountSharedData::default());

    // Update the account data size delta
    bank.calculate_and_update_accounts_data_size_delta_off_chain(
        target.total_data_size,
        source.total_data_size,
    );

    // Unload the programs from the bank's cache
    bank.loaded_programs_cache
        .write()
        .unwrap()
        .remove_programs([*source_program_address, target.program_address].into_iter());

    // Delete the built-in from the bank's built-in programs
    bank.builtin_programs.remove(&target.program_address);

    Ok(())
}

/// Create a new `Account` with a pointer to the target's new data account.
///
/// Note the pointer is created manually, as well as the owner and
/// executable values. The rest is inherited from the source program
/// account, including the lamports.
fn create_new_target_program_account(
    target: &NativeProgramConfig,
    source: &BpfUpgradeableProgramConfig,
) -> Result<AccountSharedData, MigrateNativeProgramError> {
    let state = UpgradeableLoaderState::Program {
        programdata_address: target.program_data_address,
    };
    let data = bincode::serialize(&state)?;
    let account = Account {
        data,
        owner: BPF_LOADER_UPGRADEABLE_ID,
        executable: true,
        ..source.program_account
    };
    Ok(AccountSharedData::from(account))
}

/// Migrate a native program to an upgradeable BPF program using a BPF version
/// of the program deployed at some arbitrary address.
#[allow(dead_code)] // Code is off the hot path until a migration is due
pub(crate) fn migrate_native_program_to_bpf_upgradeable(
    bank: &mut Bank,
    target_program: NativeProgram,
    source_program_address: &Pubkey,
    datapoint_name: &'static str,
) -> Result<(), MigrateNativeProgramError> {
    datapoint_info!(datapoint_name, ("slot", bank.slot, i64));

    let target = NativeProgramConfig::new_checked(bank, target_program)?;
    let source = BpfUpgradeableProgramConfig::new_checked(bank, source_program_address)?;

    // Attempt serialization first before touching the bank
    let new_target_program_account = create_new_target_program_account(&target, &source)?;

    // Burn lamports from the target program account
    bank.capitalization
        .fetch_sub(target.program_account.lamports, Relaxed);

    // Replace the native program account with the created to point to the new data
    // account and clear the source program account
    bank.store_account(&target.program_address, &new_target_program_account);
    bank.store_account(&source.program_address, &AccountSharedData::default());

    // Copy the upgradeable BPF program's data account into the native
    // program's data address, which is checked to be empty, then clear the
    // upgradeable BPF program's data account.
    bank.store_account(&target.program_data_address, &source.program_data_account);
    bank.store_account(&source.program_data_address, &AccountSharedData::default());

    // Update the account data size delta.
    bank.calculate_and_update_accounts_data_size_delta_off_chain(
        target.total_data_size,
        source.total_data_size,
    );

    // Unload the programs from the bank's cache
    bank.loaded_programs_cache
        .write()
        .unwrap()
        .remove_programs([source.program_address, target.program_address].into_iter());

    // Delete the built-in from the bank's built-in programs
    bank.builtin_programs.remove(&target.program_address);

    Ok(())
}
