#![allow(dead_code)] // Removed in later commit
pub(crate) mod error;
mod source_upgradeable_bpf;
mod target_builtin;

use {
    crate::bank::Bank,
    error::CoreBpfMigrationError,
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        bpf_loader_upgradeable::UpgradeableLoaderState,
        pubkey::Pubkey,
    },
    source_upgradeable_bpf::SourceUpgradeableBpf,
    std::sync::atomic::Ordering::Relaxed,
    target_builtin::TargetBuiltin,
};

/// Sets up a Core BPF migration for a built-in program.
#[derive(Debug)]
pub(crate) enum CoreBpfMigrationTargetType {
    /// Builtin programs should have a program account.
    Builtin,
    /// Stateless builtins should not have a program account.
    Stateless,
}

/// Configurations for migrating a built-in program to Core BPF.
#[derive(Debug)]
pub(crate) struct CoreBpfMigrationConfig {
    /// The source program ID to replace the builtin with.
    pub source_program_id: Pubkey,
    /// The feature gate to trigger the migration to Core BPF.
    /// Note: This feature gate should never be the same as any builtin's
    /// `enable_feature_id`. It should always be a feature gate that will be
    /// activated after the builtin is already enabled.
    pub feature_id: Pubkey,
    /// The type of migration to perform.
    pub migration_target: CoreBpfMigrationTargetType,
    pub datapoint_name: &'static str,
}

/// Create a new `Account` with a pointer to the target's new data account.
///
/// Note the pointer is created manually, as well as the owner and
/// executable values. The rest is inherited from the source program
/// account, including the lamports.
fn create_new_target_program_account(
    target: &TargetBuiltin,
    source: &SourceUpgradeableBpf,
) -> Result<AccountSharedData, CoreBpfMigrationError> {
    let state = UpgradeableLoaderState::Program {
        programdata_address: target.program_data_address,
    };
    let data = bincode::serialize(&state)?;
    // The source program account has the same state, so it should already have
    // a sufficient lamports balance to cover rent for this state.
    // Out of an abundance of caution, first ensure the source program
    // account's data is the same length as the serialized state.
    if source.program_account.data().len() != data.len() {
        return Err(CoreBpfMigrationError::InvalidProgramAccount(
            source.program_address,
        ));
    }
    // Then copy the source account's contents and overwrite the data with the
    // newly created target program account data.
    let mut account = source.program_account.clone();
    account.set_data_from_slice(&data);
    Ok(account)
}

impl CoreBpfMigrationConfig {
    pub(crate) fn migrate_builtin_to_core_bpf(
        &self,
        bank: &mut Bank,
        program_id: &Pubkey,
    ) -> Result<(), CoreBpfMigrationError> {
        datapoint_info!(self.datapoint_name, ("slot", bank.slot, i64));

        let target = TargetBuiltin::new_checked(bank, program_id, &self.migration_target)?;
        let source = SourceUpgradeableBpf::new_checked(bank, &self.source_program_id)?;

        // Attempt serialization first before touching the bank.
        let new_target_program_account = create_new_target_program_account(&target, &source)?;

        // Burn lamports from the target program account, since it will be
        // replaced.
        bank.capitalization
            .fetch_sub(target.program_account.lamports(), Relaxed);

        // Replace the native program account with the created to point to the new data
        // account and clear the source program account.
        bank.store_account(&target.program_address, &new_target_program_account);
        bank.store_account(&source.program_address, &AccountSharedData::default());

        // Copy the upgradeable BPF program's data account into the native
        // program's data address, which is checked to be empty, then clear the
        // upgradeable BPF program's data account.
        bank.store_account(&target.program_data_address, &source.program_data_account);
        bank.store_account(&source.program_data_address, &AccountSharedData::default());

        // Update the account data size delta.
        // The old data size is the total size of all accounts involved.
        // The new data size is the total size of the source program accounts,
        // since the target program account is replaced.
        //
        // [B] Builtin      =>      [S]  Source         =       [S]  New Target
        //                  =>      [SD] SourceData     =       [SD] SourceData
        //
        //       Old Data Size: [B + S + SD]            New Data Size: [S + SD]
        //
        let old_data_size = source
            .total_data_size
            .saturating_add(target.total_data_size);
        let new_data_size = source.total_data_size;
        bank.calculate_and_update_accounts_data_size_delta_off_chain(old_data_size, new_data_size);

        // Remove the built-in program from the bank's list of built-ins.
        bank.builtin_program_ids.remove(&target.program_address);

        // Unload the programs from the bank's cache.
        bank.transaction_processor
            .program_cache
            .write()
            .unwrap()
            .remove_programs([source.program_address, target.program_address].into_iter());

        Ok(())
    }
}
