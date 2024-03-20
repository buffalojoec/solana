use {
    super::{error::CoreBpfMigrationError, CoreBpfMigrationTarget},
    crate::bank::Bank,
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        bpf_loader_upgradeable::get_program_data_address,
        native_loader::ID as NATIVE_LOADER_ID,
        pubkey::Pubkey,
    },
};

/// Used to validate a built-in program's account before migrating to Core BPF.
#[derive(Debug)]
pub(crate) struct TargetProgramBuiltin {
    pub program_address: Pubkey,
    pub program_account: AccountSharedData,
    pub program_data_address: Pubkey,
    pub total_data_size: usize,
}

impl TargetProgramBuiltin {
    /// Create a new migration configuration for a built-in program.
    pub(crate) fn new_checked(
        bank: &Bank,
        program_id: &Pubkey,
        migration_target: &CoreBpfMigrationTarget,
    ) -> Result<Self, CoreBpfMigrationError> {
        let program_address = *program_id;
        let program_account = match migration_target {
            CoreBpfMigrationTarget::Builtin => {
                // The program account should exist.
                let program_account = bank
                    .get_account_with_fixed_root(&program_address)
                    .ok_or(CoreBpfMigrationError::AccountNotFound(program_address))?;

                // The program account should be owned by the native loader.
                if program_account.owner() != &NATIVE_LOADER_ID {
                    return Err(CoreBpfMigrationError::IncorrectOwner(program_address));
                }

                program_account
            }
            CoreBpfMigrationTarget::Stateless => {
                // The program account should _not_ exist.
                if bank.get_account_with_fixed_root(&program_address).is_some() {
                    return Err(CoreBpfMigrationError::AccountExists(program_address));
                }

                AccountSharedData::default()
            }
        };

        let program_data_address = get_program_data_address(&program_address);

        // The program data account should not exist.
        if bank
            .get_account_with_fixed_root(&program_data_address)
            .is_some()
        {
            return Err(CoreBpfMigrationError::ProgramHasDataAccount(
                program_address,
            ));
        }

        // The total data size is the size of the program account's data.
        let total_data_size = program_account.data().len();

        Ok(Self {
            program_address,
            program_account,
            program_data_address,
            total_data_size,
        })
    }
}
