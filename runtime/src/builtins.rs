use {
    crate::bank::Bank,
    solana_program_runtime::invoke_context::BuiltinFunctionWithContext,
    solana_sdk::{
        bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable, feature_set, pubkey::Pubkey,
    },
};

/// Transitions of built-in programs at epoch bondaries when features are activated.
pub struct BuiltinPrototype {
    pub feature_id: Option<Pubkey>,
    pub program_id: Pubkey,
    pub name: &'static str,
    pub entrypoint: BuiltinFunctionWithContext,
}

impl std::fmt::Debug for BuiltinPrototype {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut builder = f.debug_struct("BuiltinPrototype");
        builder.field("program_id", &self.program_id);
        builder.field("name", &self.name);
        builder.field("feature_id", &self.feature_id);
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
            feature_id: None,
            program_id: Pubkey::default(),
            name: "",
            entrypoint: MockBuiltin::vm,
        }
    }
}

/// Note: This function is set up to be intentionally redundant so that
/// built-in programs may be filtered out from this list on feature activation.
///
/// When migrating a built-in (native) program to a BPF program, the built-in
/// program should be filtered out of this list using a feature gate, like so:
///
/// ```rust, ignore
/// pub fn get_builtins(bank: &Bank) -> Vec<BuiltinPrototype> {
///     let mut builtins = Vec::new();
/// 
///     /* ... */
///     
///     if !bank.feature_set.is_active(
///         &solana_sdk::feature_set::migrate_address_lookup_table_to_bpf::id(),
///     ) {
///         builtins.push(BuiltinPrototype {
///             feature_id: None,
///             program_id: solana_sdk::address_lookup_table::program::id(),
///             name: "address_lookup_table_program",
///             entrypoint: solana_address_lookup_table_program::processor::Entrypoint::vm,
///         });
///     }
/// 
///     /* ... */
/// 
///     builtins
/// }
/// ```
///
/// Upon post-activation cleanup, the built-in program can be removed from the
/// list altogether.
pub fn get_builtins(_bank: &Bank) -> Vec<BuiltinPrototype> {
    let mut builtins = Vec::new();

    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: solana_system_program::id(),
        name: "system_program",
        entrypoint: solana_system_program::system_processor::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: solana_vote_program::id(),
        name: "vote_program",
        entrypoint: solana_vote_program::vote_processor::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: solana_stake_program::id(),
        name: "stake_program",
        entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: solana_config_program::id(),
        name: "config_program",
        entrypoint: solana_config_program::config_processor::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: bpf_loader_deprecated::id(),
        name: "solana_bpf_loader_deprecated_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: bpf_loader::id(),
        name: "solana_bpf_loader_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: bpf_loader_upgradeable::id(),
        name: "solana_bpf_loader_upgradeable_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: solana_sdk::compute_budget::id(),
        name: "compute_budget_program",
        entrypoint: solana_compute_budget_program::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: None,
        program_id: solana_sdk::address_lookup_table::program::id(),
        name: "address_lookup_table_program",
        entrypoint: solana_address_lookup_table_program::processor::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: Some(feature_set::zk_token_sdk_enabled::id()),
        program_id: solana_zk_token_sdk::zk_token_proof_program::id(),
        name: "zk_token_proof_program",
        entrypoint: solana_zk_token_proof_program::Entrypoint::vm,
    });
    builtins.push(BuiltinPrototype {
        feature_id: Some(feature_set::enable_program_runtime_v2_and_loader_v4::id()),
        program_id: solana_sdk::loader_v4::id(),
        name: "loader_v4",
        entrypoint: solana_loader_v4_program::Entrypoint::vm,
    });

    builtins
}
