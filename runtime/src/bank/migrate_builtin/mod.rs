mod bpf;
mod bpf_upgradeable;
mod builtin;
pub(crate) mod error;

use {
    crate::{bank::Bank, builtins::Builtin},
    bpf::BpfConfig,
    builtin::BuiltinConfig,
    error::MigrateBuiltinError,
    solana_sdk::{account::AccountSharedData, pubkey::Pubkey},
    std::sync::atomic::Ordering::Relaxed,
};

/// Migrate a built-in program to a BPF (non-upgradeable) program using a BPF
/// version of the program deployed at some arbitrary address.
///
/// Note!!!: This function should be used within a feature activation, and the
/// and the feature ID used to activate the feature _must_ also be added to the
/// corresponding builtin's `disabled_feature_id` field.
/// See `runtime/src/builtin.rs`.
#[allow(dead_code)] // Code is off the hot path until a migration is due
pub(crate) fn migrate_builtin_to_bpf(
    bank: &Bank,
    target_program: &Builtin,
    source_program_address: &Pubkey,
    datapoint_name: &'static str,
) -> Result<(), MigrateBuiltinError> {
    datapoint_info!(datapoint_name, ("slot", bank.slot, i64));

    let target = BuiltinConfig::new_checked(bank, target_program)?;
    let source = BpfConfig::new_checked(bank, source_program_address)?;

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

    Ok(())
}
