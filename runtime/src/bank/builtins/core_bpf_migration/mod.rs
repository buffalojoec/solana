#![allow(dead_code)] // Removed in later commit
pub(crate) mod error;
mod source_bpf_upgradeable;
mod target_builtin;

pub(crate) enum CoreBpfMigrationTargetType {
    Builtin,
    Stateless,
}
