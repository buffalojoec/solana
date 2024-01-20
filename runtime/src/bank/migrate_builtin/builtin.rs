use {
    super::error::MigrateBuiltinError,
    crate::{bank::Bank, builtins::Builtin},
    solana_sdk::{
        account::{Account, AccountSharedData},
        bpf_loader_upgradeable::get_program_data_address,
        native_loader::ID as NATIVE_LOADER_ID,
        pubkey::Pubkey,
    },
};

/// Struct for holding the configuration of a built-in program that is being
/// migrated to a BPF program.
///
/// This struct is used to validate the built-in program's account and data
/// account before the migration is performed.
#[derive(Debug)]
pub(crate) struct BuiltinConfig {
    pub(crate) program_address: Pubkey,
    pub(crate) program_account: Account,
    pub(crate) program_data_address: Pubkey,
    pub(crate) total_data_size: usize,
}
impl BuiltinConfig {
    /// Creates a new migration config for the given built-in program,
    /// validating the built-in program's account and data account
    pub(crate) fn new_checked(bank: &Bank, builtin: &Builtin) -> Result<Self, MigrateBuiltinError> {
        let program_address = builtin.program_id();
        let program_account: Account = if builtin.program_should_exist(bank) {
            // The program account should exist
            let program_account: Account = bank
                .get_account_with_fixed_root(&program_address)
                .ok_or(MigrateBuiltinError::AccountNotFound(program_address))?
                .into();

            // The program account should be owned by the built-in loader and be
            // executable
            if program_account.owner != NATIVE_LOADER_ID {
                return Err(MigrateBuiltinError::IncorrectOwner(program_address));
            }
            // See `https://github.com/solana-labs/solana/issues/33970`:
            // Neither feature `disable_bpf_loader_instructions` nor
            // `deprecate_executable_meta_update_in_bpf_loader` should
            // have an effect on checking the `executable` flag for existing built-in
            // programs.
            // It's possible this check should be removed altogether.
            if !program_account.executable {
                return Err(MigrateBuiltinError::AccountNotExecutable(program_address));
            }

            program_account
        } else {
            // The program account should _not_ exist
            if bank.get_account_with_fixed_root(&program_address).is_some() {
                return Err(MigrateBuiltinError::AccountExists(program_address));
            }

            AccountSharedData::default().into()
        };

        // The program data account should _not_ exist
        let (program_data_address, _) = get_program_data_address(&program_address);
        if bank
            .get_account_with_fixed_root(&program_data_address)
            .is_some()
        {
            return Err(MigrateBuiltinError::ProgramHasDataAccount(program_address));
        }

        // The total data size is the size of the program account's data
        let total_data_size = program_account.data.len();

        Ok(Self {
            program_address,
            program_account,
            program_data_address,
            total_data_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::bank::{tests::create_simple_test_bank, ApplyFeatureActivationsCaller},
        assert_matches::assert_matches,
        solana_sdk::{
            bpf_loader_upgradeable::ID as BPF_LOADER_UPGRADEABLE_ID, feature, feature_set,
        },
        test_case::test_case,
    };

    fn store_default_account(bank: &Bank, address: &Pubkey, executable: bool, owner: &Pubkey) {
        let lamports = bank.get_minimum_balance_for_rent_exemption(0);
        let account = AccountSharedData::from(Account {
            executable,
            lamports,
            owner: *owner,
            ..Account::default()
        });
        bank.store_account_and_update_capitalization(address, &account);
    }

    fn store_empty_account(bank: &Bank, address: &Pubkey) {
        bank.store_account_and_update_capitalization(address, &AccountSharedData::default());
    }

    fn run_checks_for_program_exists(
        bank: &Bank,
        builtin: &Builtin,
        program_id: &Pubkey,
        program_data_address: &Pubkey,
    ) {
        // Fail if the program data account exists
        store_default_account(
            bank,
            program_data_address,
            false,
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_matches!(
            BuiltinConfig::new_checked(bank, builtin).unwrap_err(),
            MigrateBuiltinError::ProgramHasDataAccount(_)
        );

        // Fail if the program account does not exist
        store_empty_account(bank, program_id);
        assert_matches!(
            BuiltinConfig::new_checked(bank, builtin).unwrap_err(),
            MigrateBuiltinError::AccountNotFound(_)
        );

        // Fail if the owner is not the native loader
        store_default_account(bank, program_id, true, &Pubkey::new_unique());
        assert_matches!(
            BuiltinConfig::new_checked(bank, builtin).unwrap_err(),
            MigrateBuiltinError::IncorrectOwner(_)
        );

        // Fail if the program account is not executable
        store_default_account(bank, program_id, false, &NATIVE_LOADER_ID);
        assert_matches!(
            BuiltinConfig::new_checked(bank, builtin).unwrap_err(),
            MigrateBuiltinError::AccountNotExecutable(_)
        );

        // Reset when finished
        store_empty_account(bank, program_id);
        store_empty_account(bank, program_data_address);
    }

    fn run_checks_for_program_does_not_exist(
        bank: &Bank,
        builtin: &Builtin,
        program_id: &Pubkey,
        program_data_address: &Pubkey,
    ) {
        // Fail if the program data account exists
        store_default_account(
            bank,
            program_data_address,
            false,
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_matches!(
            BuiltinConfig::new_checked(bank, builtin).unwrap_err(),
            MigrateBuiltinError::ProgramHasDataAccount(_)
        );

        // Fail if the program account exists
        store_default_account(bank, program_id, true, &NATIVE_LOADER_ID);
        assert_matches!(
            BuiltinConfig::new_checked(bank, builtin).unwrap_err(),
            MigrateBuiltinError::AccountExists(_)
        );

        // Reset when finished
        store_empty_account(bank, program_id);
        store_empty_account(bank, program_data_address);
    }

    #[test_case(
        Builtin::AddressLookupTable,
        solana_sdk::address_lookup_table::program::id()
    )]
    #[test_case(Builtin::BpfLoader, solana_sdk::bpf_loader::id())]
    #[test_case(Builtin::BpfLoaderDeprecated, solana_sdk::bpf_loader_deprecated::id())]
    #[test_case(
        Builtin::BpfLoaderUpgradeable,
        solana_sdk::bpf_loader_upgradeable::id()
    )]
    #[test_case(Builtin::ComputeBudget, solana_sdk::compute_budget::id())]
    #[test_case(Builtin::Config, solana_sdk::config::program::id())]
    #[test_case(Builtin::FeatureGate, solana_sdk::feature::id())]
    #[test_case(Builtin::LoaderV4, solana_sdk::loader_v4::id())]
    #[test_case(Builtin::NativeLoader, solana_sdk::native_loader::id())]
    #[test_case(Builtin::Stake, solana_sdk::stake::program::id())]
    #[test_case(Builtin::System, solana_sdk::system_program::id())]
    #[test_case(Builtin::Vote, solana_sdk::vote::program::id())]
    #[test_case(
        Builtin::ZkTokenProof,
        solana_zk_token_sdk::zk_token_proof_program::id()
    )]
    fn test_builtin_config(builtin: Builtin, check_program_id: Pubkey) {
        let bank = create_simple_test_bank(0);

        let check_program_account: Account = bank
            .get_account_with_fixed_root(&check_program_id)
            .unwrap_or_default()
            .into();
        let check_program_data_address = get_program_data_address(&check_program_id).0;

        let builtin_config = BuiltinConfig::new_checked(&bank, &builtin).unwrap();

        assert_eq!(builtin_config.program_address, check_program_id);
        assert_eq!(builtin_config.program_account, check_program_account);
        assert_eq!(
            builtin_config.program_data_address,
            check_program_data_address
        );
        assert_eq!(
            builtin_config.total_data_size,
            check_program_account.data.len()
        );
    }

    #[test_case(Builtin::AddressLookupTable)]
    #[test_case(Builtin::BpfLoader)]
    #[test_case(Builtin::BpfLoaderDeprecated)]
    #[test_case(Builtin::BpfLoaderUpgradeable)]
    #[test_case(Builtin::ComputeBudget)]
    #[test_case(Builtin::Config)]
    #[test_case(Builtin::Stake)]
    #[test_case(Builtin::System)]
    #[test_case(Builtin::Vote)]
    fn test_builtin_config_program_exists(builtin: Builtin) {
        let bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Specifically check the program account's data is not the default
        let builtin_config = BuiltinConfig::new_checked(&bank, &builtin).unwrap();
        let default_account: Account = AccountSharedData::default().into();
        assert_ne!(builtin_config.program_account, default_account);

        run_checks_for_program_exists(&bank, &builtin, &program_id, &program_data_address);
    }

    #[test_case(Builtin::FeatureGate)]
    #[test_case(Builtin::NativeLoader)]
    fn test_builtin_config_program_does_not_exist(builtin: Builtin) {
        let bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Specifically check the program account's data _is_ the default
        let builtin_config = BuiltinConfig::new_checked(&bank, &builtin).unwrap();
        let default_account: Account = AccountSharedData::default().into();
        assert_eq!(builtin_config.program_account, default_account);

        run_checks_for_program_does_not_exist(&bank, &builtin, &program_id, &program_data_address);
    }

    #[test_case(
        Builtin::LoaderV4,
        feature_set::enable_program_runtime_v2_and_loader_v4::id()
    )]
    #[test_case(Builtin::ZkTokenProof, feature_set::zk_token_sdk_enabled::id())]
    fn test_builtin_config_program_is_not_deployed(builtin: Builtin, activation_feature: Pubkey) {
        let mut bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Specifically check the program account's data _is_ the default
        let builtin_config = BuiltinConfig::new_checked(&bank, &builtin).unwrap();
        let default_account: Account = AccountSharedData::default().into();
        assert_eq!(builtin_config.program_account, default_account);

        // First the program shouldn't exist
        run_checks_for_program_does_not_exist(&bank, &builtin, &program_id, &program_data_address);

        // Activate the feature to enable the built-in program
        bank.store_account(
            &activation_feature,
            &feature::create_account(
                &feature::Feature { activated_at: None },
                bank.get_minimum_balance_for_rent_exemption(feature::Feature::size_of()),
            ),
        );
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Specifically check the program account's data is not the default
        let builtin_config = BuiltinConfig::new_checked(&bank, &builtin).unwrap();
        assert_ne!(builtin_config.program_account, default_account);

        run_checks_for_program_exists(&bank, &builtin, &program_id, &program_data_address);
    }
}
