#![allow(dead_code)] // TODO: Removed in future commit
use {
    super::error::MigrateBuiltinError,
    crate::bank::Bank,
    solana_sdk::{
        account::Account,
        bpf_loader::ID as BPF_LOADER_ID,
        bpf_loader_upgradeable::{get_program_data_address, UpgradeableLoaderState},
        feature_set::deprecate_executable_meta_update_in_bpf_loader,
        pubkey::Pubkey,
    },
};

/// Struct for holding the configuration of a BPF (non-upgradeable) program
/// intending to replace a built-in program.
///
/// This struct is used to validate the BPF (non-upgradeable) program's account
/// before the migration is performed.
#[derive(Debug)]
pub(crate) struct BpfConfig {
    pub(crate) program_address: Pubkey,
    pub(crate) program_account: Account,
    pub(crate) total_data_size: usize,
}
impl BpfConfig {
    /// Run checks on the program account
    fn check_program_account(&self, bank: &Bank) -> Result<(), MigrateBuiltinError> {
        // The program account should be owned by the non-upgradeable loader and
        // be executable
        if self.program_account.owner != BPF_LOADER_ID {
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

        // The program data account should have the correct state
        let programdata_data_offset = UpgradeableLoaderState::size_of_programdata_metadata();
        if self.program_account.data.len() < programdata_data_offset {
            return Err(MigrateBuiltinError::InvalidProgramAccount(
                self.program_address,
            ));
        }
        // Length checked in previous block
        match bincode::deserialize::<UpgradeableLoaderState>(
            &self.program_account.data[..programdata_data_offset],
        ) {
            Ok(UpgradeableLoaderState::ProgramData { .. }) => Ok(()),
            _ => Err(MigrateBuiltinError::InvalidProgramAccount(
                self.program_address,
            )),
        }
    }

    /// Creates a new migration config for the given BPF (non-upgradeable)
    /// program, validating the BPF program's account
    pub(crate) fn new_checked(bank: &Bank, address: &Pubkey) -> Result<Self, MigrateBuiltinError> {
        let program_address = *address;
        let program_account: Account = bank
            .get_account_with_fixed_root(&program_address)
            .ok_or(MigrateBuiltinError::AccountNotFound(program_address))?
            .into();

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

        let config = Self {
            program_address,
            program_account,
            total_data_size,
        };

        config.check_program_account(bank)?;

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
    fn test_bpf_config() {
        let bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();

        // Fail if the program account does not exist
        assert_eq!(
            BpfConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::AccountNotFound(program_id)
        );

        // Store the proper program account
        let proper_program_account_state = UpgradeableLoaderState::ProgramData {
            slot: 0,
            upgrade_authority_address: Some(Pubkey::new_unique()),
        };
        store_account(
            &bank,
            &program_id,
            (&proper_program_account_state, Some(&[4u8; 200])),
            true,
            &BPF_LOADER_ID,
        );

        // Success
        let bpf_program_config = BpfConfig::new_checked(&bank, &program_id).unwrap();

        let mut check_program_account_data =
            bincode::serialize(&proper_program_account_state).unwrap();
        check_program_account_data.extend_from_slice(&[4u8; 200]);
        let check_program_account_data_len = check_program_account_data.len();
        let check_program_lamports =
            bank.get_minimum_balance_for_rent_exemption(check_program_account_data_len);
        let check_program_account = Account {
            data: check_program_account_data,
            executable: true,
            lamports: check_program_lamports,
            owner: BPF_LOADER_ID,
            ..Account::default()
        };

        assert_eq!(bpf_program_config.program_address, program_id);
        assert_eq!(bpf_program_config.program_account, check_program_account);
        assert_eq!(
            bpf_program_config.total_data_size,
            check_program_account_data_len
        );
    }

    #[test]
    fn tst_bpf_config_bad_program_account() {
        let bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();

        // Fail if the program account is not owned by the non-upgradeable loader
        store_account(
            &bank,
            &program_id,
            (
                &UpgradeableLoaderState::ProgramData {
                    slot: 0,
                    upgrade_authority_address: Some(Pubkey::new_unique()),
                },
                Some(&[4u8; 200]),
            ),
            true,
            &Pubkey::new_unique(), // Not the non-upgradeable loader
        );
        assert_eq!(
            BpfConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::IncorrectOwner(program_id)
        );

        // Fail if the program account is not executable
        store_account(
            &bank,
            &program_id,
            (
                &UpgradeableLoaderState::ProgramData {
                    slot: 0,
                    upgrade_authority_address: Some(Pubkey::new_unique()),
                },
                Some(&[4u8; 200]),
            ),
            false, // Not executable
            &BPF_LOADER_ID,
        );
        assert_eq!(
            BpfConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::AccountNotExecutable(program_id)
        );
    }

    #[test]
    fn test_bpf_config_program_data_account_exists() {
        let bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();
        let (program_data_address, _) = get_program_data_address(&program_id);

        // Store the proper program account
        store_account(
            &bank,
            &program_id,
            (
                &UpgradeableLoaderState::ProgramData {
                    slot: 0,
                    upgrade_authority_address: Some(Pubkey::new_unique()),
                },
                Some(&[4u8; 200]),
            ),
            true,
            &BPF_LOADER_ID,
        );

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
            BpfConfig::new_checked(&bank, &program_id).unwrap_err(),
            MigrateBuiltinError::ProgramHasDataAccount(program_id)
        );
    }

    #[test]
    fn test_bpf_config_features_active() {
        let mut bank = create_simple_test_bank(0);

        let program_id = Pubkey::new_unique();

        // Store the program account as non-executable
        let proper_program_account_state = UpgradeableLoaderState::ProgramData {
            slot: 0,
            upgrade_authority_address: Some(Pubkey::new_unique()),
        };
        store_account(
            &bank,
            &program_id,
            (&proper_program_account_state, Some(&[4u8; 200])),
            false, // Not executable
            &BPF_LOADER_ID,
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
        let bpf_program_config = BpfConfig::new_checked(&bank, &program_id).unwrap();

        let mut check_program_account_data =
            bincode::serialize(&proper_program_account_state).unwrap();
        check_program_account_data.extend_from_slice(&[4u8; 200]);
        let check_program_account_data_len = check_program_account_data.len();
        let check_program_lamports =
            bank.get_minimum_balance_for_rent_exemption(check_program_account_data_len);
        let check_program_account = Account {
            data: check_program_account_data,
            executable: false, // Not executable
            lamports: check_program_lamports,
            owner: BPF_LOADER_ID,
            ..Account::default()
        };

        assert_eq!(bpf_program_config.program_address, program_id);
        assert_eq!(bpf_program_config.program_account, check_program_account);
        assert_eq!(
            bpf_program_config.total_data_size,
            check_program_account_data_len
        );
    }
}
