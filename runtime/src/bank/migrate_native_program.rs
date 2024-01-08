use {
    super::Bank,
    solana_sdk::{
        account::{Account, AccountSharedData},
        bpf_loader::ID as BPF_LOADER_ID,
        bpf_loader_upgradeable::{UpgradeableLoaderState, ID as BPF_LOADER_UPGRADEABLE_ID},
        native_loader::ID as NATIVE_LOADER_ID,
        pubkey::Pubkey,
    },
    std::sync::atomic::Ordering::Relaxed,
    thiserror::Error,
};

/// Helper for deriving the program data address from a program id
fn get_program_data_address(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[program_id.as_ref()], &BPF_LOADER_UPGRADEABLE_ID).0
}

/// Errors returned by `replace_account` methods
#[derive(Debug, Error)]
pub enum MigrateNativeProgramError {
    /// Account not executable
    #[error("Account not executable: {0:?}")]
    AccountNotExecutable(Pubkey),
    /// Account is executable
    #[error("Account is executable: {0:?}")]
    AccountIsExecutable(Pubkey),
    /// Account not found
    #[error("Account not found: {0:?}")]
    AccountNotFound(Pubkey),
    /// Account exists
    #[error("Account exists: {0:?}")]
    AccountExists(Pubkey),
    /// Incorrect account owner
    #[error("Incorrect account owner for {0:?}")]
    IncorrectOwner(Pubkey),
    /// program has a data account
    #[error("Data account exists for program {0:?}")]
    ProgramHasDataAccount(Pubkey),
    /// Program has no data account
    #[error("Data account does not exist for program {0:?}")]
    ProgramHasNoDataAccount(Pubkey),
    /// Invalid program data account
    #[error("Invalid program data account: {0:?}")]
    InvalidProgramDataAccount(Pubkey),
    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] Box<bincode::ErrorKind>),
}

// Code is currently off of the hot path until a migration is due
//
/// Enum representing the native programs that can be migrated to BPF
/// programs
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(crate) enum MigrateNativeProgram {
    AddressLookupTable,
    BpfLoader,
    BpfLoaderUpgradeable,
    ComputeBudget,
    Config,
    Ed25519,
    FeatureGate,
    NativeLoader,
    Secp256k1,
    System,
    Stake,
    Vote,
    // ZkTokenProof,
}
impl MigrateNativeProgram {
    pub(crate) fn id(&self) -> Pubkey {
        match self {
            Self::AddressLookupTable => solana_sdk::address_lookup_table::program::id(),
            Self::BpfLoader => solana_sdk::bpf_loader::id(),
            Self::BpfLoaderUpgradeable => solana_sdk::bpf_loader_upgradeable::id(),
            Self::ComputeBudget => solana_sdk::compute_budget::id(),
            Self::Config => solana_sdk::config::program::id(),
            Self::Ed25519 => solana_sdk::ed25519_program::id(),
            Self::FeatureGate => solana_sdk::feature::id(),
            Self::NativeLoader => solana_sdk::native_loader::id(),
            Self::Secp256k1 => solana_sdk::secp256k1_program::id(),
            Self::System => solana_sdk::system_program::id(),
            Self::Stake => solana_sdk::stake::program::id(),
            Self::Vote => solana_sdk::vote::program::id(),
            // Self::ZkTokenProof => solana_zk_token_proof_program::id(),
        }
    }

    fn program_account_should_exist(&self) -> bool {
        match self {
            Self::FeatureGate | Self::NativeLoader => false,
            _ => true,
        }
    }
}

struct MigrateNativeProgramConfigNativeProgram {
    program_address: Pubkey,
    program_account: Account,
    program_data_address: Pubkey,
    total_data_size: usize,
}
impl MigrateNativeProgramConfigNativeProgram {
    fn new_checked(
        bank: &Bank,
        native_program: MigrateNativeProgram,
    ) -> Result<Self, MigrateNativeProgramError> {
        let program_address = native_program.id();
        let program_account: Account = if native_program.program_account_should_exist() {
            let program_account: Account = bank
                .get_account_with_fixed_root(&program_address)
                .ok_or(MigrateNativeProgramError::AccountNotFound(program_address))?
                .into();

            // The program account should be owned by the native loader and be
            // executable
            if program_account.owner != NATIVE_LOADER_ID {
                return Err(MigrateNativeProgramError::IncorrectOwner(program_address));
            }
            if !program_account.executable {
                return Err(MigrateNativeProgramError::AccountNotExecutable(
                    program_address,
                ));
            }

            program_account
        } else {
            // The program account should _not_ exist
            if bank.get_account_with_fixed_root(&program_address).is_some() {
                return Err(MigrateNativeProgramError::AccountExists(program_address));
            }

            AccountSharedData::default().into()
        };

        // The program data account should _not_ exist
        let program_data_address = get_program_data_address(&program_address);
        if bank
            .get_account_with_fixed_root(&program_data_address)
            .is_some()
        {
            return Err(MigrateNativeProgramError::ProgramHasDataAccount(
                program_address,
            ));
        }

        let total_data_size = program_account.data.len();

        Ok(Self {
            program_address,
            program_account,
            program_data_address,
            total_data_size,
        })
    }
}

struct MigrateNativeProgramConfigUpgradeableProgram {
    program_address: Pubkey,
    program_account: Account,
    program_data_address: Pubkey,
    program_data_account: Account,
    total_data_size: usize,
}
impl MigrateNativeProgramConfigUpgradeableProgram {
    fn new_checked(bank: &Bank, address: &Pubkey) -> Result<Self, MigrateNativeProgramError> {
        let program_address = *address;
        let program_account: Account = bank
            .get_account_with_fixed_root(&program_address)
            .ok_or(MigrateNativeProgramError::AccountNotFound(program_address))?
            .into();

        // The source program account should be owned by the upgradeable loader
        // and be executable
        if program_account.owner != BPF_LOADER_UPGRADEABLE_ID {
            return Err(MigrateNativeProgramError::IncorrectOwner(program_address));
        }
        if !program_account.executable {
            return Err(MigrateNativeProgramError::AccountNotExecutable(
                program_address,
            ));
        }

        // The source account should also have a pointer to its data account
        let program_data_address = get_program_data_address(&program_address);
        let deserialized_program_data: UpgradeableLoaderState =
            bincode::deserialize(&program_account.data)?;
        if let UpgradeableLoaderState::Program {
            programdata_address,
        } = deserialized_program_data
        {
            if programdata_address != program_data_address {
                return Err(MigrateNativeProgramError::InvalidProgramDataAccount(
                    program_data_address,
                ));
            }
        } else {
            return Err(MigrateNativeProgramError::InvalidProgramDataAccount(
                program_data_address,
            ));
        }

        let program_data_account: Account = bank
            .get_account_with_fixed_root(&program_data_address)
            .ok_or(MigrateNativeProgramError::ProgramHasNoDataAccount(
                program_address,
            ))?
            .into();

        // The source program data account should be owned by the upgradeable
        // loader and _not_ be executable
        if program_data_account.owner != BPF_LOADER_UPGRADEABLE_ID {
            return Err(MigrateNativeProgramError::IncorrectOwner(
                program_data_address,
            ));
        }
        if program_data_account.executable {
            return Err(MigrateNativeProgramError::AccountIsExecutable(
                program_data_address,
            ));
        }

        let total_data_size = program_account.data.len() + program_data_account.data.len();

        Ok(Self {
            program_address,
            program_account,
            program_data_address,
            program_data_account,
            total_data_size,
        })
    }
}

struct MigrateNativeProgramConfigNonUpgradeableProgram {
    program_address: Pubkey,
    program_account: Account,
    total_data_size: usize,
}
impl MigrateNativeProgramConfigNonUpgradeableProgram {
    fn new_checked(bank: &Bank, address: &Pubkey) -> Result<Self, MigrateNativeProgramError> {
        let program_address = *address;
        let program_account: Account = bank
            .get_account_with_fixed_root(&program_address)
            .ok_or(MigrateNativeProgramError::AccountNotFound(program_address))?
            .into();

        // The program account should be owned by the non-upgradeable loader and
        // be executable
        if program_account.owner != BPF_LOADER_ID {
            return Err(MigrateNativeProgramError::IncorrectOwner(program_address));
        }
        if !program_account.executable {
            return Err(MigrateNativeProgramError::AccountNotExecutable(
                program_address,
            ));
        }

        // The program data account should _not_ exist
        let program_data_address = get_program_data_address(&program_address);
        if bank
            .get_account_with_fixed_root(&program_data_address)
            .is_some()
        {
            return Err(MigrateNativeProgramError::ProgramHasDataAccount(
                program_address,
            ));
        }

        let total_data_size = program_account.data.len();

        Ok(Self {
            program_address,
            program_account,
            total_data_size,
        })
    }
}

/// Migrate a native program to an upgradeable BPF program using a BPF version
/// of the program deployed at some arbitrary address.
#[allow(dead_code)]
pub(crate) fn migrate_native_program_to_bpf_upgradeable(
    bank: &Bank,
    source_program_address: &Pubkey,
    target_program: MigrateNativeProgram,
    datapoint_name: &'static str,
) -> Result<(), MigrateNativeProgramError> {
    datapoint_info!(datapoint_name, ("slot", bank.slot, i64));

    let source =
        MigrateNativeProgramConfigUpgradeableProgram::new_checked(bank, source_program_address)?;

    let target = MigrateNativeProgramConfigNativeProgram::new_checked(bank, target_program)?;

    // Burn lamports from the target program account
    bank.capitalization
        .fetch_sub(target.program_account.lamports, Relaxed);

    // Replace the native program account's data to point to the new data
    // account and clear the source program account
    let target_program_account_data = AccountSharedData::from(Account {
        data: bincode::serialize(&UpgradeableLoaderState::Program {
            programdata_address: target.program_data_address,
        })?,
        owner: BPF_LOADER_UPGRADEABLE_ID,
        executable: true,
        ..source.program_account
    });
    bank.store_account(&target.program_address, &target_program_account_data);
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

    Ok(())
}

/// Migrate a native program to a non-upgradeable BPF program using a BPF
/// version of the program deployed at some arbitrary address.
#[allow(dead_code)]
pub(crate) fn migrate_native_program_to_bpf_non_upgradeable(
    bank: &Bank,
    source_program_address: &Pubkey,
    target_program: MigrateNativeProgram,
    datapoint_name: &'static str,
) -> Result<(), MigrateNativeProgramError> {
    datapoint_info!(datapoint_name, ("slot", bank.slot, i64));

    let source =
        MigrateNativeProgramConfigNonUpgradeableProgram::new_checked(bank, source_program_address)?;

    let target = MigrateNativeProgramConfigNativeProgram::new_checked(bank, target_program)?;

    // Burn lamports from the target program account
    bank.capitalization
        .fetch_sub(target.program_account.lamports, Relaxed);

    // Copy the non-upgradeable BPF program's account into the native program's
    // address, then clear the non-upgradeable BPF program account.
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
