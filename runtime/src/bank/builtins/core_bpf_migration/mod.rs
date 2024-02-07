use solana_sdk::pubkey::Pubkey;

/// Configurations for migrating a built-in program to Core BPF.
pub struct CoreBpfMigrationConfig {
    pub source_program_id: Pubkey,
    pub feature_id: Pubkey,
}

impl std::fmt::Debug for CoreBpfMigrationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut builder = f.debug_struct("CoreBpfMigrationConfig");
        builder.field("source_program_id", &self.source_program_id);
        builder.field("feature_id", &self.feature_id);
        builder.finish()
    }
}
