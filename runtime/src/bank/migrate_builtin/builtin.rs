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
/// This struct is used to validate the built-in program's account before the
/// migration is performed.
#[derive(Debug)]
pub(crate) struct BuiltinConfig {
    pub(crate) program_address: Pubkey,
    pub(crate) program_account: Account,
    pub(crate) program_data_address: Pubkey,
    pub(crate) total_data_size: usize,
}
impl BuiltinConfig {
    /// Creates a new migration config for the given built-in program,
    /// validating the built-in program's account
    pub(crate) fn new_checked(bank: &Bank, builtin: &Builtin) -> Result<Self, MigrateBuiltinError> {
        let program_address = builtin.program_id();
        let program_account: Account = if builtin.program_should_exist(bank) {
            // The program account should exist
            let program_account: Account = bank
                .get_account_with_fixed_root(&program_address)
                .ok_or(MigrateBuiltinError::AccountNotFound(program_address))?
                .into();

            // The program account should be owned by the native loader and be
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
        solana_sdk::{
            bpf_loader_upgradeable::{UpgradeableLoaderState, ID as BPF_LOADER_UPGRADEABLE_ID},
            feature, feature_set,
        },
        test_case::test_case,
    };

    fn store_account<T: serde::Serialize>(
        bank: &Bank,
        address: &Pubkey,
        data: (&T, Option<&[u8]>),
        executable: bool,
        owner: &Pubkey,
    ) {
        let (data, additional_data) = data;
        let mut data = bincode::serialize(data).unwrap();
        if let Some(additional_data) = additional_data {
            data.extend_from_slice(additional_data);
        }
        let data_len = data.len();
        let lamports = bank.get_minimum_balance_for_rent_exemption(data_len);
        let account = AccountSharedData::from(Account {
            data,
            executable,
            lamports,
            owner: *owner,
            ..Account::default()
        });
        bank.store_account_and_update_capitalization(address, &account);
    }

    #[test_case(Builtin::AddressLookupTable)]
    #[test_case(Builtin::BpfLoader)]
    #[test_case(Builtin::BpfLoaderDeprecated)]
    #[test_case(Builtin::BpfLoaderUpgradeable)]
    #[test_case(Builtin::ComputeBudget)]
    #[test_case(Builtin::Config)]
    #[test_case(Builtin::FeatureGate)]
    #[test_case(Builtin::LoaderV4)]
    #[test_case(Builtin::NativeLoader)]
    #[test_case(Builtin::Stake)]
    #[test_case(Builtin::System)]
    #[test_case(Builtin::Vote)]
    #[test_case(Builtin::ZkTokenProof)]
    fn test_builtin_config(builtin: Builtin) {
        let bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();
        let program_account: Account = bank
            .get_account_with_fixed_root(&program_id)
            .unwrap_or_default() // `AccountSharedData::default()` if not exists
            .into();
        let program_data_address = get_program_data_address(&program_id).0;

        let builtin_config = BuiltinConfig::new_checked(&bank, &builtin).unwrap();

        assert_eq!(builtin_config.program_address, program_id);
        assert_eq!(builtin_config.program_account, program_account);
        assert_eq!(builtin_config.program_data_address, program_data_address);
        assert_eq!(builtin_config.total_data_size, program_account.data.len());
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
    fn test_builtin_config_bad_program_account(builtin: Builtin) {
        let bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();

        // Fail if the program account is not owned by the native loader
        store_account(
            &bank,
            &program_id,
            (&String::from("some built-in program"), None),
            true,
            &Pubkey::new_unique(), // Not the native loader
        );
        assert_eq!(
            BuiltinConfig::new_checked(&bank, &builtin).unwrap_err(),
            MigrateBuiltinError::IncorrectOwner(program_id)
        );

        // Fail if the program account is not executable
        store_account(
            &bank,
            &program_id,
            (&String::from("some built-in program"), None),
            false, // Not executable
            &NATIVE_LOADER_ID,
        );
        assert_eq!(
            BuiltinConfig::new_checked(&bank, &builtin).unwrap_err(),
            MigrateBuiltinError::AccountNotExecutable(program_id)
        );
    }

    #[test_case(Builtin::AddressLookupTable)]
    #[test_case(Builtin::BpfLoader)]
    #[test_case(Builtin::BpfLoaderDeprecated)]
    #[test_case(Builtin::BpfLoaderUpgradeable)]
    #[test_case(Builtin::ComputeBudget)]
    #[test_case(Builtin::Config)]
    #[test_case(Builtin::FeatureGate)]
    #[test_case(Builtin::LoaderV4)]
    #[test_case(Builtin::NativeLoader)]
    #[test_case(Builtin::Stake)]
    #[test_case(Builtin::System)]
    #[test_case(Builtin::Vote)]
    #[test_case(Builtin::ZkTokenProof)]
    fn test_builtin_config_program_data_account_exists(builtin: Builtin) {
        let bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Fail if the program data account exists
        store_account(
            &bank,
            &program_data_address,
            (
                &UpgradeableLoaderState::ProgramData {
                    slot: 0,
                    upgrade_authority_address: Some(Pubkey::new_unique()),
                },
                Some(&[4u8; 200]),
            ),
            false,
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_eq!(
            BuiltinConfig::new_checked(&bank, &builtin).unwrap_err(),
            MigrateBuiltinError::ProgramHasDataAccount(program_id)
        );
    }

    #[test_case(Builtin::FeatureGate)]
    #[test_case(Builtin::LoaderV4)]
    #[test_case(Builtin::NativeLoader)]
    #[test_case(Builtin::ZkTokenProof)]
    fn test_builtin_config_program_account_exists_but_should_not(builtin: Builtin) {
        let bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();

        // Fail if the program account exists
        store_account(
            &bank,
            &program_id,
            (&String::from("some built-in program"), None),
            true,
            &NATIVE_LOADER_ID,
        );
        assert_eq!(
            BuiltinConfig::new_checked(&bank, &builtin).unwrap_err(),
            MigrateBuiltinError::AccountExists(program_id)
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
    fn test_builtin_config_program_account_does_not_exist_but_should(builtin: Builtin) {
        let bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();

        // Fail if the program account does not exist
        bank.store_account_and_update_capitalization(&program_id, &AccountSharedData::default());
        assert_eq!(
            BuiltinConfig::new_checked(&bank, &builtin).unwrap_err(),
            MigrateBuiltinError::AccountNotFound(program_id)
        );
    }

    #[test_case(
        Builtin::LoaderV4,
        feature_set::enable_program_runtime_v2_and_loader_v4::id()
    )]
    #[test_case(Builtin::ZkTokenProof, feature_set::zk_token_sdk_enabled::id())]
    fn test_builtin_config_features_enabled(builtin: Builtin, activation_feature: Pubkey) {
        let mut bank = create_simple_test_bank(0);

        let program_id = builtin.program_id();
        let program_data_address = get_program_data_address(&program_id).0;

        // Activate the feature to enable the built-in program
        bank.store_account(
            &activation_feature,
            &feature::create_account(
                &feature::Feature { activated_at: None },
                bank.get_minimum_balance_for_rent_exemption(feature::Feature::size_of()),
            ),
        );
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        let program_account: Account = bank
            .get_account_with_fixed_root(&program_id)
            .unwrap() // Should exist now
            .into();

        let builtin_config = BuiltinConfig::new_checked(&bank, &builtin).unwrap();

        assert_eq!(builtin_config.program_address, program_id);
        assert_eq!(builtin_config.program_account, program_account);
        assert_eq!(builtin_config.program_data_address, program_data_address);
        assert_eq!(builtin_config.total_data_size, program_account.data.len());
    }
}
