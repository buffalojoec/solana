use {solana_sdk::pubkey::Pubkey, thiserror::Error};

/// Errors returned by a Core BPF migration.
#[derive(Debug, Error, PartialEq)]
pub enum CoreBpfMigrationError {
    /// Account not found
    #[error("Account not found: {0:?}")]
    AccountNotFound(Pubkey),
    /// Account exists
    #[error("Account exists: {0:?}")]
    AccountExists(Pubkey),
    /// Incorrect account owner
    #[error("Incorrect account owner for {0:?}")]
    IncorrectOwner(Pubkey),
    /// Program has a data account
    #[error("Data account exists for program {0:?}")]
    ProgramHasDataAccount(Pubkey),
    /// Program has no data account
    #[error("Data account does not exist for program {0:?}")]
    ProgramHasNoDataAccount(Pubkey),
    /// Invalid program account
    #[error("Invalid program account: {0:?}")]
    InvalidProgramAccount(Pubkey),
    /// Invalid program data account
    #[error("Invalid program data account: {0:?}")]
    InvalidProgramDataAccount(Pubkey),
    /// Failed to serialize new program account
    #[error("Failed to serialize new program account")]
    FailedToSerialize,
    // Since `core_bpf_migration` does not return `ProgramError` or
    // `InstructionError`, we have to duplicate `ArithmeticOverflow` here.
    /// Arithmetic overflow
    #[error("Arithmetic overflow")]
    ArithmeticOverflow,
}
