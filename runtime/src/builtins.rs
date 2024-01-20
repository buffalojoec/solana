use {
    crate::bank::Bank,
    solana_program_runtime::invoke_context::BuiltinFunctionWithContext,
    solana_sdk::{
        bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable, feature_set, pubkey::Pubkey,
    },
};

/// Transitions of built-in programs at epoch bondaries when features are activated.
pub struct BuiltinPrototype {
    /// The feature ID used to activate the built-in program
    pub enable_feature_id: Option<Pubkey>,
    /// The feature ID used to _deactivate_ the built-in program
    pub disable_feature_id: Option<Pubkey>,
    pub program_id: Pubkey,
    pub name: &'static str,
    pub entrypoint: BuiltinFunctionWithContext,
}

impl std::fmt::Debug for BuiltinPrototype {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut builder = f.debug_struct("BuiltinPrototype");
        builder.field("program_id", &self.program_id);
        builder.field("name", &self.name);
        builder.field("enable_feature_id", &self.enable_feature_id);
        builder.field("disable_feature_id", &self.disable_feature_id);
        builder.finish()
    }
}

#[cfg(RUSTC_WITH_SPECIALIZATION)]
impl solana_frozen_abi::abi_example::AbiExample for BuiltinPrototype {
    fn example() -> Self {
        // BuiltinPrototype isn't serializable by definition.
        solana_program_runtime::declare_process_instruction!(MockBuiltin, 0, |_invoke_context| {
            // Do nothing
            Ok(())
        });
        Self {
            enable_feature_id: None,
            disable_feature_id: None,
            program_id: Pubkey::default(),
            name: "",
            entrypoint: MockBuiltin::vm,
        }
    }
}

pub static BUILTINS: &[BuiltinPrototype] = &[
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: solana_system_program::id(),
        name: "system_program",
        entrypoint: solana_system_program::system_processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: solana_vote_program::id(),
        name: "vote_program",
        entrypoint: solana_vote_program::vote_processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: solana_stake_program::id(),
        name: "stake_program",
        entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: solana_config_program::id(),
        name: "config_program",
        entrypoint: solana_config_program::config_processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: bpf_loader_deprecated::id(),
        name: "solana_bpf_loader_deprecated_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: bpf_loader::id(),
        name: "solana_bpf_loader_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: bpf_loader_upgradeable::id(),
        name: "solana_bpf_loader_upgradeable_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: solana_sdk::compute_budget::id(),
        name: "compute_budget_program",
        entrypoint: solana_compute_budget_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: None,
        disable_feature_id: None,
        program_id: solana_sdk::address_lookup_table::program::id(),
        name: "address_lookup_table_program",
        entrypoint: solana_address_lookup_table_program::processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: Some(feature_set::zk_token_sdk_enabled::id()),
        disable_feature_id: None,
        program_id: solana_zk_token_sdk::zk_token_proof_program::id(),
        name: "zk_token_proof_program",
        entrypoint: solana_zk_token_proof_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        enable_feature_id: Some(feature_set::enable_program_runtime_v2_and_loader_v4::id()),
        disable_feature_id: None,
        program_id: solana_sdk::loader_v4::id(),
        name: "loader_v4",
        entrypoint: solana_loader_v4_program::Entrypoint::vm,
    },
];

/// Enum used to identify built-ins for the purpose of setting up a migration
/// to BPF.
#[allow(dead_code)] // Code is off the hot path until a migration is due
pub(crate) enum Builtin {
    AddressLookupTable,
    BpfLoader,
    BpfLoaderDeprecated,
    BpfLoaderUpgradeable,
    ComputeBudget,
    Config,
    FeatureGate,
    LoaderV4,
    NativeLoader,
    Stake,
    System,
    Vote,
    ZkTokenProof,
}
impl Builtin {
    pub(crate) fn program_id(&self) -> Pubkey {
        match self {
            Builtin::AddressLookupTable => solana_sdk::address_lookup_table::program::id(),
            Builtin::BpfLoader => bpf_loader::id(),
            Builtin::BpfLoaderDeprecated => bpf_loader_deprecated::id(),
            Builtin::BpfLoaderUpgradeable => bpf_loader_upgradeable::id(),
            Builtin::ComputeBudget => solana_sdk::compute_budget::id(),
            Builtin::Config => solana_config_program::id(),
            Builtin::FeatureGate => solana_sdk::feature::id(),
            Builtin::LoaderV4 => solana_sdk::loader_v4::id(),
            Builtin::NativeLoader => solana_sdk::native_loader::id(),
            Builtin::Stake => solana_stake_program::id(),
            Builtin::System => solana_system_program::id(),
            Builtin::Vote => solana_vote_program::id(),
            Builtin::ZkTokenProof => solana_zk_token_sdk::zk_token_proof_program::id(),
        }
    }

    pub(crate) fn program_should_exist(&self, bank: &Bank) -> bool {
        let program_id = self.program_id();
        if let Some(prototype) = BUILTINS.iter().find(|p| p.program_id == program_id) {
            // If the activation feature is active, the program account should
            // exist
            if let Some(enable_feature_id) = prototype.enable_feature_id {
                return bank.feature_set.is_active(&enable_feature_id);
            }
            // If the _deactivation_ feature is active, the program account
            // should not exist
            if let Some(disable_feature_id) = prototype.disable_feature_id {
                return !bank.feature_set.is_active(&disable_feature_id);
            }
            return true;
        }
        // If the program is not listed as a built-in, then the program account
        // should not exist
        false
    }
}
