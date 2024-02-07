mod bpf_upgradeable;
mod builtin;
pub(crate) mod error;

use {
    crate::bank::Bank,
    bpf_upgradeable::BpfUpgradeableConfig,
    builtin::BuiltinConfig,
    error::CoreBpfMigrationError,
    solana_sdk::{
        account::{Account, AccountSharedData},
        bpf_loader_upgradeable::{UpgradeableLoaderState, ID as BPF_LOADER_UPGRADEABLE_ID},
        pubkey::Pubkey,
    },
    std::sync::atomic::Ordering::Relaxed,
};

/// Sets up a Core BPF migration for a built-in program.
pub enum CoreBpfMigration {
    Builtin,
    Ephemeral,
}

/// Configurations for migrating a built-in program to Core BPF.
pub struct CoreBpfMigrationConfig {
    pub source_program_id: Pubkey,
    pub feature_id: Pubkey,
    pub datapoint_name: &'static str,
}

impl std::fmt::Debug for CoreBpfMigrationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut builder = f.debug_struct("CoreBpfMigrationConfig");
        builder.field("source_program_id", &self.source_program_id);
        builder.field("feature_id", &self.feature_id);
        builder.finish()
    }
}

/// Create a new `Account` with a pointer to the target's new data account.
///
/// Note the pointer is created manually, as well as the owner and
/// executable values. The rest is inherited from the source program
/// account, including the lamports.
fn create_new_target_program_account(
    target: &BuiltinConfig,
    source: &BpfUpgradeableConfig,
) -> Result<AccountSharedData, CoreBpfMigrationError> {
    let state = UpgradeableLoaderState::Program {
        programdata_address: target.program_data_address,
    };
    let data = bincode::serialize(&state).map_err(|_| CoreBpfMigrationError::FailedToSerialize)?;
    let account = Account {
        data,
        owner: BPF_LOADER_UPGRADEABLE_ID,
        executable: true,
        ..source.program_account
    };
    Ok(AccountSharedData::from(account))
}

impl CoreBpfMigrationConfig {
    pub(crate) fn migrate_builtin_to_core_bpf(
        &self,
        bank: &mut Bank,
        program_id: &Pubkey,
        migration: CoreBpfMigration,
    ) -> Result<(), CoreBpfMigrationError> {
        datapoint_info!(self.datapoint_name, ("slot", bank.slot, i64));

        let target = BuiltinConfig::new_checked(bank, program_id, migration)?;
        let source = BpfUpgradeableConfig::new_checked(bank, &self.source_program_id)?;

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

        // Remove the built-in program from the bank's list of built-ins
        bank.builtin_programs.remove(&target.program_address);

        // Unload the programs from the bank's cache
        bank.loaded_programs_cache
            .write()
            .unwrap()
            .remove_programs([source.program_address, target.program_address].into_iter());

        Ok(())
    }
}
