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

/// Identifies the type of built-in program targeted for Core BPF migration.
/// The type of target determines whether the program should have a program
/// account or not, which is checked before migration.
#[derive(Debug)]
pub(crate) enum CoreBpfMigrationTargetType {
    /// A standard (stateful) builtin program must have a program account.
    Builtin,
    /// A stateless builtin must not have a program account.
    Stateless,
}

/// Configuration for migrating a built-in program to Core BPF.
#[derive(Debug)]
pub(crate) struct CoreBpfMigrationConfig {
    /// The program ID of the source program to be used to replace the builtin.
    pub source_program_id: Pubkey,
    /// The feature gate to trigger the migration to Core BPF.
    /// Note: This feature gate should never be the same as any builtin's
    /// `enable_feature_id`. It should always be a feature gate that will be
    /// activated after the builtin is already enabled.
    pub feature_id: Pubkey,
    /// The type of target to replace.
    pub migration_target: CoreBpfMigrationTargetType,
    /// Static message used to emit datapoint logging.
    /// This is used to identify the migration in the logs.
    /// Should be unique to the migration, ie:
    /// "migrate_{builtin/stateless}_to_core_bpf_{program_name}".
    pub datapoint_name: &'static str,
}

fn checked_add(a: usize, b: usize) -> Result<usize, CoreBpfMigrationError> {
    a.checked_add(b)
        .ok_or(CoreBpfMigrationError::ArithmeticOverflow)
}

/// Create an `AccountSharedData` with data initialized to
/// `UpgradeableLoaderState::Program` populated with the target's new data
/// account address.
///
/// Note that the account's data is initialized manually, but the rest of the
/// account's fields are inherited from the source program account, including
/// the lamports.
fn new_target_program_account(
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
        let new_target_program_account = new_target_program_account(&target, &source)?;

        // Update the account data size delta.
        // The old data size is the total size of all accounts involved.
        // The new data size is the total size of the source program accounts,
        // since the target program account is replaced with a new program
        // account of the same size as the source program account, and the
        // source program data account is copied to the target program data
        // account before both source program accounts are cleared.
        let target_program_len = target.program_account.data().len();
        let source_program_len = source.program_account.data().len();
        let source_program_data_len = source.program_data_account.data().len();
        let old_data_size = checked_add(
            target_program_len,
            checked_add(source_program_len, source_program_data_len)?,
        )?;
        let new_data_size = checked_add(source_program_len, source_program_data_len)?;
        bank.calculate_and_update_accounts_data_size_delta_off_chain(old_data_size, new_data_size);

        // Burn lamports from the target program account, since it will be
        // replaced.
        bank.capitalization
            .fetch_sub(target.program_account.lamports(), Relaxed);

        // Replace the target builtin account with the
        // `new_target_program_account` and clear the source program account.
        bank.store_account(&target.program_address, &new_target_program_account);
        bank.store_account(&source.program_address, &AccountSharedData::default());

        // Copy the source program data account into the account at the target
        // builtin program's data address, which was verified to be empty by
        // `TargetBuiltin::new_checked`, then clear the source program data
        // account.
        bank.store_account(&target.program_data_address, &source.program_data_account);
        bank.store_account(&source.program_data_address, &AccountSharedData::default());

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

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::bank::tests::create_simple_test_bank,
        solana_program_runtime::loaded_programs::LoadedProgram,
        solana_sdk::{
            account_utils::StateMut,
            bpf_loader_upgradeable::{self, get_program_data_address},
            clock::Slot,
            native_loader,
        },
    };

    const PROGRAM_DATA_OFFSET: usize = UpgradeableLoaderState::size_of_programdata_metadata();

    struct TestContext {
        builtin_id: Pubkey,
        source_program_id: Pubkey,
        slot: Slot,
        upgrade_authority_address: Option<Pubkey>,
        elf: Vec<u8>,
    }
    impl TestContext {
        // Initialize some test values and set up the source BPF upgradeable
        // program in the bank.
        fn new(bank: &Bank) -> Self {
            let builtin_id = Pubkey::new_unique();
            let source_program_id = Pubkey::new_unique();
            let slot = 99;
            let upgrade_authority_address = Some(Pubkey::new_unique());
            let elf = vec![4; 2000];

            let source_program_data_address = get_program_data_address(&source_program_id);

            let source_program_account = {
                let data = bincode::serialize(&UpgradeableLoaderState::Program {
                    programdata_address: source_program_data_address,
                })
                .unwrap();

                let data_len = data.len();
                let lamports = bank.get_minimum_balance_for_rent_exemption(data_len);

                let mut account =
                    AccountSharedData::new(lamports, data_len, &bpf_loader_upgradeable::id());
                account.set_data(data);
                account
            };

            let source_program_data_account = {
                let mut data = bincode::serialize(&UpgradeableLoaderState::ProgramData {
                    slot,
                    upgrade_authority_address,
                })
                .unwrap();
                data.extend_from_slice(&elf);

                let data_len = data.len();
                let lamports = bank.get_minimum_balance_for_rent_exemption(data_len);

                let mut account =
                    AccountSharedData::new(lamports, data_len, &bpf_loader_upgradeable::id());
                account.set_data(data);
                account
            };

            bank.store_account_and_update_capitalization(
                &source_program_id,
                &source_program_account,
            );
            bank.store_account_and_update_capitalization(
                &source_program_data_address,
                &source_program_data_account,
            );

            Self {
                builtin_id,
                source_program_id,
                slot,
                upgrade_authority_address,
                elf,
            }
        }

        // Evaluate the account state of the builtin and source post-migration.
        // Ensure the builtin program account is now a BPF upgradeable program,
        // the source program account and data account have been cleared, and
        // the bank's builtin IDs and cache have been updated.
        fn run_program_checks_post_migration(&self, bank: &Bank) {
            // Verify both the source program account and source program data
            // account have been cleared.
            assert!(bank.get_account(&self.source_program_id).is_none());
            assert!(bank
                .get_account(&get_program_data_address(&self.source_program_id))
                .is_none());

            let program_account = bank.get_account(&self.builtin_id).unwrap();
            let program_data_address = get_program_data_address(&self.builtin_id);

            // Program account is owned by the upgradeable loader.
            assert_eq!(program_account.owner(), &bpf_loader_upgradeable::id());

            // Program account has the correct state, with a pointer to its program
            // data address.
            let program_account_state: UpgradeableLoaderState = program_account.state().unwrap();
            assert_eq!(
                program_account_state,
                UpgradeableLoaderState::Program {
                    programdata_address: program_data_address
                }
            );

            let program_data_account = bank.get_account(&program_data_address).unwrap();

            // Program data account is owned by the upgradeable loader.
            assert_eq!(program_data_account.owner(), &bpf_loader_upgradeable::id());

            // Program data account has the correct state.
            // It should exactly match the original, including upgrade authority
            // and slot.
            let program_data_account_state_metadata: UpgradeableLoaderState =
                bincode::deserialize(&program_data_account.data()[..PROGRAM_DATA_OFFSET]).unwrap();
            assert_eq!(
                program_data_account_state_metadata,
                UpgradeableLoaderState::ProgramData {
                    slot: self.slot,
                    upgrade_authority_address: self.upgrade_authority_address
                },
            );
            assert_eq!(
                &program_data_account.data()[PROGRAM_DATA_OFFSET..],
                &self.elf,
            );

            // The bank's builtins should no longer contain the builtin
            // program ID.
            assert!(!bank.builtin_program_ids.contains(&self.builtin_id));

            // The cache should have unloaded both programs.
            let program_cache = bank.transaction_processor.program_cache.read().unwrap();
            assert!(!program_cache
                .get_flattened_entries(true, true)
                .iter()
                .any(|(id, _)| id == &self.builtin_id || id == &self.source_program_id));
        }
    }

    #[test]
    fn test_migrate_builtin() {
        let mut bank = create_simple_test_bank(0);

        let test_context = TestContext::new(&bank);

        let TestContext {
            builtin_id,
            source_program_id,
            ..
        } = test_context;

        // This will be checked by `TargetBuiltinProgram::new_checked`, but set
        // up the mock builtin and ensure it exists as configured.
        let builtin_account = {
            let builtin_name = String::from("test_builtin");
            let account =
                AccountSharedData::new_data(1, &builtin_name, &native_loader::id()).unwrap();
            bank.store_account_and_update_capitalization(&builtin_id, &account);
            bank.add_builtin(builtin_id, builtin_name, LoadedProgram::default());
            account
        };
        assert_eq!(&bank.get_account(&builtin_id).unwrap(), &builtin_account);

        let core_bpf_migration_config = CoreBpfMigrationConfig {
            source_program_id,
            feature_id: Pubkey::new_unique(),
            migration_target: CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "test_migrate_builtin",
        };

        // Gather bank information to check later.
        let bank_pre_migration_capitalization = bank.capitalization();
        let bank_pre_migration_accounts_data_size_delta_off_chain =
            bank.accounts_data_size_delta_off_chain.load(Relaxed);

        // Perform the migration.
        core_bpf_migration_config
            .migrate_builtin_to_core_bpf(&mut bank, &builtin_id)
            .unwrap();

        // Run the post-migration program checks.
        test_context.run_program_checks_post_migration(&bank);

        // The bank's capitalization should reflect the burned lamports
        // from the replaced builtin program account.
        assert_eq!(
            bank.capitalization(),
            bank_pre_migration_capitalization - builtin_account.lamports()
        );

        // The bank's accounts data size delta off-chain should reflect the
        // change in data size from the replaced builtin program account.
        assert_eq!(
            bank.accounts_data_size_delta_off_chain.load(Relaxed),
            bank_pre_migration_accounts_data_size_delta_off_chain
                - builtin_account.data().len() as i64,
        );
    }

    #[test]
    fn test_migrate_stateless_builtin() {
        let mut bank = create_simple_test_bank(0);

        let test_context = TestContext::new(&bank);

        let TestContext {
            builtin_id,
            source_program_id,
            ..
        } = test_context;

        // This will be checked by `TargetBuiltinProgram::new_checked`, but
        // assert the stateless builtin account doesn't exist.
        assert!(bank.get_account(&builtin_id).is_none());

        let core_bpf_migration_config = CoreBpfMigrationConfig {
            source_program_id,
            feature_id: Pubkey::new_unique(),
            migration_target: CoreBpfMigrationTargetType::Stateless,
            datapoint_name: "test_migrate_stateless_builtin",
        };

        // Gather bank information to check later.
        let bank_pre_migration_capitalization = bank.capitalization();
        let bank_pre_migration_accounts_data_size_delta_off_chain =
            bank.accounts_data_size_delta_off_chain.load(Relaxed);

        // Perform the migration.
        core_bpf_migration_config
            .migrate_builtin_to_core_bpf(&mut bank, &builtin_id)
            .unwrap();

        // Run the post-migration program checks.
        test_context.run_program_checks_post_migration(&bank);

        // The bank's capitalization should be exactly the same.
        assert_eq!(bank.capitalization(), bank_pre_migration_capitalization);

        // The bank's accounts data size delta off-chain should be exactly the
        // same.
        assert_eq!(
            bank.accounts_data_size_delta_off_chain.load(Relaxed),
            bank_pre_migration_accounts_data_size_delta_off_chain,
        );
    }
}
