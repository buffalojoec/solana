use {
    super::error::MigrateBuiltinError,
    crate::bank::Bank,
    solana_sdk::{
        account::Account,
        bpf_loader_upgradeable::{
            get_program_data_address, UpgradeableLoaderState, ID as BPF_LOADER_UPGRADEABLE_ID,
        },
        feature_set::deprecate_executable_meta_update_in_bpf_loader,
        pubkey::Pubkey,
    },
};

/// Struct for holding the configuration of a BPF upgradeable program intending
/// to replace a built-in program.
///
/// This struct is used to validate the BPF upgradeable program's account and
/// data account before the migration is performed.
#[derive(Debug)]
pub(crate) struct BpfUpgradeableConfig {
    pub(crate) program_address: Pubkey,
    pub(crate) program_account: Account,
    pub(crate) program_data_address: Pubkey,
    pub(crate) program_data_account: Account,
    pub(crate) total_data_size: usize,
}
impl BpfUpgradeableConfig {
    /// Run checks on the program account
    fn check_program_account(&self, bank: &Bank) -> Result<(), MigrateBuiltinError> {
        // The program account should be owned by the upgradeable loader and
        // be executable
        if self.program_account.owner != BPF_LOADER_UPGRADEABLE_ID {
            return Err(MigrateBuiltinError::IncorrectOwner(self.program_address));
        }

        // See `https://github.com/solana-labs/solana/issues/34425`:
        // Feature `deprecate_executable_meta_update_in_bpf_loader` will cause
        // this check to fail for new upgradeable BPF programs.
        // It's possible this check should be removed altogether.
        if !bank
            .feature_set
            .is_active(&deprecate_executable_meta_update_in_bpf_loader::id())
            && !self.program_account.executable
        {
            return Err(MigrateBuiltinError::AccountNotExecutable(
                self.program_address,
            ));
        }

        // The program account should have a pointer to its data account
        if let UpgradeableLoaderState::Program {
            programdata_address,
        } = bincode::deserialize(&self.program_account.data).map_err::<MigrateBuiltinError, _>(
            |_| MigrateBuiltinError::InvalidProgramAccount(self.program_address),
        )? {
            if programdata_address != self.program_data_address {
                return Err(MigrateBuiltinError::InvalidProgramAccount(
                    self.program_address,
                ));
            }
        }

        Ok(())
    }

    /// Run checks on the program data account
    fn check_program_data_account(&self, bank: &Bank) -> Result<(), MigrateBuiltinError> {
        // The program data account should be owned by the upgradeable loader
        // and _not_ be executable
        if self.program_data_account.owner != BPF_LOADER_UPGRADEABLE_ID {
            return Err(MigrateBuiltinError::IncorrectOwner(
                self.program_data_address,
            ));
        }

        // See `https://github.com/solana-labs/solana/issues/34425`:
        // Feature `deprecate_executable_meta_update_in_bpf_loader` will cause
        // this check to fail for new upgradeable BPF programs.
        // It's possible this check should be removed altogether.
        if !bank
            .feature_set
            .is_active(&deprecate_executable_meta_update_in_bpf_loader::id())
            && self.program_data_account.executable
        {
            return Err(MigrateBuiltinError::AccountIsExecutable(
                self.program_data_address,
            ));
        }

        // The program data account should have the correct state
        let programdata_data_offset = UpgradeableLoaderState::size_of_programdata_metadata();
        if self.program_data_account.data.len() < programdata_data_offset {
            return Err(MigrateBuiltinError::InvalidProgramDataAccount(
                self.program_data_address,
            ));
        }
        // Length checked in previous block
        match bincode::deserialize::<UpgradeableLoaderState>(
            &self.program_data_account.data[..programdata_data_offset],
        ) {
            Ok(UpgradeableLoaderState::ProgramData { .. }) => Ok(()),
            _ => Err(MigrateBuiltinError::InvalidProgramDataAccount(
                self.program_data_address,
            )),
        }
    }

    /// Creates a new migration config for the given BPF upgradeable program,
    /// validating the BPF program's account and data account
    pub(crate) fn new_checked(bank: &Bank, address: &Pubkey) -> Result<Self, MigrateBuiltinError> {
        // The program account should exist
        let program_address = *address;
        let program_account: Account = bank
            .get_account_with_fixed_root(&program_address)
            .ok_or(MigrateBuiltinError::AccountNotFound(program_address))?
            .into();

        // The program data account should exist
        let (program_data_address, _) = get_program_data_address(&program_address);
        let program_data_account: Account = bank
            .get_account_with_fixed_root(&program_data_address)
            .ok_or(MigrateBuiltinError::ProgramHasNoDataAccount(
                program_address,
            ))?
            .into();

        let total_data_size = program_account
            .data
            .len()
            .checked_add(program_data_account.data.len())
            .ok_or(MigrateBuiltinError::ArithmeticOverflow)?;

        let config = Self {
            program_address,
            program_account,
            program_data_address,
            program_data_account,
            total_data_size,
        };

        config.check_program_account(bank)?;
        config.check_program_data_account(bank)?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::bank::{tests::create_simple_test_bank, ApplyFeatureActivationsCaller},
        solana_sdk::{
            account::AccountSharedData, bpf_loader_upgradeable::ID as BPF_LOADER_UPGRADEABLE_ID,
            feature, feature_set,
        },
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

    #[test]
    fn test_bpf_upgradeable_config() {
        let bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Fail if the program account does not exist
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::AccountNotFound(program_id)
        );

        // Store the proper program account
        let proper_program_account_state = UpgradeableLoaderState::Program {
            programdata_address: program_data_address,
        };
        store_account(
            &bank,
            &program_id,
            (&proper_program_account_state, None),
            true,
            &BPF_LOADER_UPGRADEABLE_ID,
        );

        // Fail if the program data account does not exist
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::ProgramHasNoDataAccount(program_id)
        );

        // Store the proper program data account
        let proper_program_data_account_state = UpgradeableLoaderState::ProgramData {
            slot: 0,
            upgrade_authority_address: Some(Pubkey::new_unique()),
        };
        store_account(
            &bank,
            &program_data_address,
            (&proper_program_data_account_state, Some(&[4u8; 200])),
            false,
            &BPF_LOADER_UPGRADEABLE_ID,
        );

        // Success
        let bpf_upgradeable_program_config =
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap();

        let check_program_account_data = bincode::serialize(&proper_program_account_state).unwrap();
        let check_program_account_data_len = check_program_account_data.len();
        let check_program_lamports =
            bank.get_minimum_balance_for_rent_exemption(check_program_account_data_len);
        let check_program_account = Account {
            data: check_program_account_data,
            executable: true,
            lamports: check_program_lamports,
            owner: BPF_LOADER_UPGRADEABLE_ID,
            ..Account::default()
        };

        let mut check_program_data_account_data =
            bincode::serialize(&proper_program_data_account_state).unwrap();
        check_program_data_account_data.extend_from_slice(&[4u8; 200]);
        let check_program_data_account_data_len = check_program_data_account_data.len();
        let check_program_data_lamports =
            bank.get_minimum_balance_for_rent_exemption(check_program_data_account_data_len);
        let check_program_data_account = Account {
            data: check_program_data_account_data,
            executable: false,
            lamports: check_program_data_lamports,
            owner: BPF_LOADER_UPGRADEABLE_ID,
            ..Account::default()
        };

        assert_eq!(bpf_upgradeable_program_config.program_address, program_id);
        assert_eq!(
            bpf_upgradeable_program_config.program_account,
            check_program_account
        );
        assert_eq!(
            bpf_upgradeable_program_config.program_data_address,
            program_data_address
        );
        assert_eq!(
            bpf_upgradeable_program_config.program_data_account,
            check_program_data_account
        );
        assert_eq!(
            bpf_upgradeable_program_config.total_data_size,
            check_program_account_data_len + check_program_data_account_data_len
        );
    }

    #[test]
    fn test_bpf_upgradeable_config_bad_program_account() {
        let bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Store the proper program data account
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

        // Fail if the program account is not owned by the upgradeable loader
        store_account(
            &bank,
            &program_id,
            (
                &UpgradeableLoaderState::Program {
                    programdata_address: program_data_address,
                },
                None,
            ),
            true,
            &Pubkey::new_unique(), // Not the upgradeable loader
        );
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::IncorrectOwner(program_id)
        );

        // Fail if the program account is not executable
        store_account(
            &bank,
            &program_id,
            (
                &UpgradeableLoaderState::Program {
                    programdata_address: program_data_address,
                },
                None,
            ),
            false, // Not executable
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::AccountNotExecutable(program_id)
        );

        // Fail if the program account's state is not `UpgradeableLoaderState::Program`
        store_account(
            &bank,
            &program_id,
            (&vec![0u8; 200], None),
            true,
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::InvalidProgramAccount(program_id)
        );

        // Fail if the program account's state is `UpgradeableLoaderState::Program`,
        // but it points to the wrong data account
        store_account(
            &bank,
            &program_id,
            (
                &UpgradeableLoaderState::Program {
                    programdata_address: Pubkey::new_unique(), // Not the correct data account
                },
                None,
            ),
            true,
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::InvalidProgramAccount(program_id)
        );
    }

    #[test]
    fn test_bpf_upgradeable_config_bad_program_data_account() {
        let bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Store the proper program account
        store_account(
            &bank,
            &program_id,
            (
                &UpgradeableLoaderState::Program {
                    programdata_address: program_data_address,
                },
                None,
            ),
            true,
            &BPF_LOADER_UPGRADEABLE_ID,
        );

        // Fail if the program data account is not owned by the upgradeable loader
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
            &Pubkey::new_unique(), // Not the upgradeable loader
        );
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::IncorrectOwner(program_data_address)
        );

        // Fail if the program data account _is_ executable
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
            true, // Executable
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::AccountIsExecutable(program_data_address)
        );

        // Fail if the program data account does not have the correct state
        store_account(
            &bank,
            &program_data_address,
            (&vec![4u8; 200], None), // Not the correct state
            false,
            &BPF_LOADER_UPGRADEABLE_ID,
        );
        assert_eq!(
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::InvalidProgramDataAccount(program_data_address)
        );
    }

    #[test]
    fn test_bpf_upgradeable_config_features_active() {
        let mut bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Store the program account as non-executable
        let proper_program_account_state = UpgradeableLoaderState::Program {
            programdata_address: program_data_address,
        };
        store_account(
            &bank,
            &program_id,
            (&proper_program_account_state, None),
            false, // Not executable
            &BPF_LOADER_UPGRADEABLE_ID,
        );

        // Store the program data account as executable
        let proper_program_data_account_state = UpgradeableLoaderState::ProgramData {
            slot: 0,
            upgrade_authority_address: Some(Pubkey::new_unique()),
        };
        store_account(
            &bank,
            &program_data_address,
            (&proper_program_data_account_state, Some(&[4u8; 200])),
            true, // Executable
            &BPF_LOADER_UPGRADEABLE_ID,
        );

        // Activate the feature to disable the `executable` check
        bank.store_account(
            &feature_set::deprecate_executable_meta_update_in_bpf_loader::id(),
            &feature::create_account(
                &feature::Feature { activated_at: None },
                bank.get_minimum_balance_for_rent_exemption(feature::Feature::size_of()),
            ),
        );
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Success
        let bpf_upgradeable_program_config =
            BpfUpgradeableConfig::new_checked(&bank, &program_id).unwrap();

        let check_program_account_data = bincode::serialize(&proper_program_account_state).unwrap();
        let check_program_account_data_len = check_program_account_data.len();
        let check_program_lamports =
            bank.get_minimum_balance_for_rent_exemption(check_program_account_data_len);
        let check_program_account = Account {
            data: check_program_account_data,
            executable: false, // Not executable
            lamports: check_program_lamports,
            owner: BPF_LOADER_UPGRADEABLE_ID,
            ..Account::default()
        };

        let mut check_program_data_account_data =
            bincode::serialize(&proper_program_data_account_state).unwrap();
        check_program_data_account_data.extend_from_slice(&[4u8; 200]);
        let check_program_data_account_data_len = check_program_data_account_data.len();
        let check_program_data_lamports =
            bank.get_minimum_balance_for_rent_exemption(check_program_data_account_data_len);
        let check_program_data_account = Account {
            data: check_program_data_account_data,
            executable: true, // Executable
            lamports: check_program_data_lamports,
            owner: BPF_LOADER_UPGRADEABLE_ID,
            ..Account::default()
        };

        assert_eq!(bpf_upgradeable_program_config.program_address, program_id);
        assert_eq!(
            bpf_upgradeable_program_config.program_account,
            check_program_account
        );
        assert_eq!(
            bpf_upgradeable_program_config.program_data_address,
            program_data_address
        );
        assert_eq!(
            bpf_upgradeable_program_config.program_data_account,
            check_program_data_account
        );
        assert_eq!(
            bpf_upgradeable_program_config.total_data_size,
            check_program_account_data_len + check_program_data_account_data_len
        );
    }
}
