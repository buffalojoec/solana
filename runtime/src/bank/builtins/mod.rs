pub(crate) mod core_bpf_migration;
pub mod prototypes;

pub use prototypes::{BuiltinPrototype, StatelessBuiltinPrototype};
use solana_sdk::{bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable, feature_set};

/// Runtime builtin programs.
#[derive(Debug)]
pub struct BuiltinPrograms(Vec<BuiltinPrototype>);

impl BuiltinPrograms {
    pub fn new(builtins: Vec<BuiltinPrototype>) -> Self {
        Self(builtins)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, BuiltinPrototype> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl Default for BuiltinPrograms {
    fn default() -> Self {
        Self(vec![
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_system_program::id(),
                name: "system_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_vote_program::id(),
                name: "vote_program",
                entrypoint: solana_vote_program::vote_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_stake_program::id(),
                name: "stake_program",
                entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_config_program::id(),
                name: "config_program",
                entrypoint: solana_config_program::config_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: bpf_loader_deprecated::id(),
                name: "solana_bpf_loader_deprecated_program",
                entrypoint: solana_bpf_loader_program::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: bpf_loader::id(),
                name: "solana_bpf_loader_program",
                entrypoint: solana_bpf_loader_program::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: bpf_loader_upgradeable::id(),
                name: "solana_bpf_loader_upgradeable_program",
                entrypoint: solana_bpf_loader_program::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_sdk::compute_budget::id(),
                name: "compute_budget_program",
                entrypoint: solana_compute_budget_program::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_sdk::address_lookup_table::program::id(),
                name: "address_lookup_table_program",
                entrypoint: solana_address_lookup_table_program::processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: Some(feature_set::zk_token_sdk_enabled::id()),
                program_id: solana_zk_token_sdk::zk_token_proof_program::id(),
                name: "zk_token_proof_program",
                entrypoint: solana_zk_token_proof_program::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: Some(feature_set::enable_program_runtime_v2_and_loader_v4::id()),
                program_id: solana_sdk::loader_v4::id(),
                name: "loader_v4",
                entrypoint: solana_loader_v4_program::Entrypoint::vm,
            },
        ])
    }
}

#[cfg(RUSTC_WITH_SPECIALIZATION)]
impl solana_frozen_abi::abi_example::AbiExample for BuiltinPrograms {
    fn example() -> Self {
        Self(Vec::<BuiltinPrototype>::example())
    }
}
