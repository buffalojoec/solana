pub(crate) mod core_bpf_migration;
pub mod prototypes;

pub use prototypes::{BuiltinPrototype, StatelessBuiltinPrototype};
use solana_sdk::{bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable, feature_set};

pub static BUILTINS: &[BuiltinPrototype] = &[
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::system_program::CONFIG),
        enable_feature_id: None,
        program_id: solana_system_program::id(),
        name: "system_program",
        entrypoint: solana_system_program::system_processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::vote_program::CONFIG),
        enable_feature_id: None,
        program_id: solana_vote_program::id(),
        name: "vote_program",
        entrypoint: solana_vote_program::vote_processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::stake_program::CONFIG),
        enable_feature_id: None,
        program_id: solana_stake_program::id(),
        name: "stake_program",
        entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::config_program::CONFIG),
        enable_feature_id: None,
        program_id: solana_config_program::id(),
        name: "config_program",
        entrypoint: solana_config_program::config_processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::bpf_loader_deprecated_program::CONFIG),
        enable_feature_id: None,
        program_id: bpf_loader_deprecated::id(),
        name: "solana_bpf_loader_deprecated_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::bpf_loader_program::CONFIG),
        enable_feature_id: None,
        program_id: bpf_loader::id(),
        name: "solana_bpf_loader_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::bpf_loader_upgradeable_program::CONFIG),
        enable_feature_id: None,
        program_id: bpf_loader_upgradeable::id(),
        name: "solana_bpf_loader_upgradeable_program",
        entrypoint: solana_bpf_loader_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::compute_budget_program::CONFIG),
        enable_feature_id: None,
        program_id: solana_sdk::compute_budget::id(),
        name: "compute_budget_program",
        entrypoint: solana_compute_budget_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::address_lookup_table_program::CONFIG),
        enable_feature_id: None,
        program_id: solana_sdk::address_lookup_table::program::id(),
        name: "address_lookup_table_program",
        entrypoint: solana_address_lookup_table_program::processor::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::zk_token_proof_program::CONFIG),
        enable_feature_id: Some(feature_set::zk_token_sdk_enabled::id()),
        program_id: solana_zk_token_sdk::zk_token_proof_program::id(),
        name: "zk_token_proof_program",
        entrypoint: solana_zk_token_proof_program::Entrypoint::vm,
    },
    BuiltinPrototype {
        #[cfg(not(test))]
        core_bpf_migration_config: None,
        #[cfg(test)]
        core_bpf_migration_config: Some(test_only::loader_v4::CONFIG),
        enable_feature_id: Some(feature_set::enable_program_runtime_v2_and_loader_v4::id()),
        program_id: solana_sdk::loader_v4::id(),
        name: "loader_v4",
        entrypoint: solana_loader_v4_program::Entrypoint::vm,
    },
];

pub static STATELESS_BUILTINS: &[StatelessBuiltinPrototype] = &[StatelessBuiltinPrototype {
    #[cfg(not(test))]
    core_bpf_migration_config: None,
    #[cfg(test)]
    core_bpf_migration_config: Some(test_only::feature_gate_program::CONFIG),
    program_id: solana_sdk::feature::id(),
    name: "feature_gate_program",
}];

#[cfg(test)]
mod test_only {
    use super::core_bpf_migration::{CoreBpfMigrationConfig, CoreBpfMigrationTargetType};
    pub mod system_program {
        pub mod feature {
            solana_sdk::declare_id!("AnjsdWg7LXFbjDdy78wncCJs9PyTdWpKkFmHAwQU1mQ6");
        }
        pub mod source_program {
            solana_sdk::declare_id!("EDEhzg1Jk79Wrk4mwpRa7txjgRxcE6igXwd6egFDVhuz");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_system_program",
        };
    }

    pub mod vote_program {
        pub mod feature {
            solana_sdk::declare_id!("5wDLHMasPmtrcpfRZX67RVkBXBbSTQ9S4C8EJomD3yAk");
        }
        pub mod source_program {
            solana_sdk::declare_id!("6T9s4PTcHnpq2AVAqoCbJd4FuHsdD99MjSUEbS7qb1tT");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_vote_program",
        };
    }

    pub mod stake_program {
        pub mod feature {
            solana_sdk::declare_id!("5gp5YKtNEirX45igBvp39bN6nEwhkNMRS7m2c63D1xPM");
        }
        pub mod source_program {
            solana_sdk::declare_id!("2a3XnUr4Xfxd8hBST8wd4D3Qbiu339XKessYsDwabCED");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_stake_program",
        };
    }

    pub mod config_program {
        pub mod feature {
            solana_sdk::declare_id!("8Jve2vaMFhEgQ5JeB5HxekLx7hyjfRN2jLFawACqekNf");
        }
        pub mod source_program {
            solana_sdk::declare_id!("73ALcNtVqyM3q7XsvB2xkVECvggu4CcLX5J2XKmpjdBU");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_config_program",
        };
    }

    pub mod bpf_loader_deprecated_program {
        pub mod feature {
            solana_sdk::declare_id!("8gpakCv5Pk5PZGv9RUjzdkk2GVQPGx12cNRUDMQ3bP86");
        }
        pub mod source_program {
            solana_sdk::declare_id!("DveUYB5m9G3ce4zpV3fxg9pCNkvH1wDsyd8XberZ47JL");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_bpf_loader_deprecated_program",
        };
    }

    pub mod bpf_loader_program {
        pub mod feature {
            solana_sdk::declare_id!("8yEdUm4SaP1yNq2MczEVdrM48SucvZCTDSqjcAKfYfL6");
        }
        pub mod source_program {
            solana_sdk::declare_id!("2EWMYGJPuGLW4TexLLEMeXP2BkB1PXEKBFb698yw6LhT");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_bpf_loader_program",
        };
    }

    pub mod bpf_loader_upgradeable_program {
        pub mod feature {
            solana_sdk::declare_id!("oPQbVjgoQ7SaQmzZiiHW4xqHbh4BJqqrFhxEJZiMiwY");
        }
        pub mod source_program {
            solana_sdk::declare_id!("6bTmA9iefD57GDoQ9wUjG8SeYkSpRw3EkKzxZCbhkavq");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_bpf_loader_upgradeable_program",
        };
    }

    pub mod compute_budget_program {
        pub mod feature {
            solana_sdk::declare_id!("D39vUspVfhjPVD7EtMJZrA5j1TSMp4LXfb43nxumGdHT");
        }
        pub mod source_program {
            solana_sdk::declare_id!("KfX1oLpFC5CwmFeSgXrNcXaouKjFkPuSJ4UsKb3zKMX");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_compute_budget_program",
        };
    }

    pub mod address_lookup_table_program {
        pub mod feature {
            solana_sdk::declare_id!("5G9xu4TnRShZpEhWyjAW2FnRNCwF85g5XKzSbQy4XpCq");
        }
        pub mod source_program {
            solana_sdk::declare_id!("DQshE9LTac8eXjZTi8ApeuZJYH67UxTMUxaEGstC6mqJ");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_address_lookup_table_program",
        };
    }

    pub mod zk_token_proof_program {
        pub mod feature {
            solana_sdk::declare_id!("GfeFwUzKP9NmaP5u4VfnFgEvQoeQc2wPgnBFrUZhpib5");
        }
        pub mod source_program {
            solana_sdk::declare_id!("Ffe9gL8vXraBkiv3HqbLvBqY7i9V4qtZxjH83jYYDe1V");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_zk_token_proof_program",
        };
    }

    pub mod loader_v4 {
        pub mod feature {
            solana_sdk::declare_id!("Cz5JthYp27KR3rwTCtVJhbRgwHCurbwcYX46D8setL22");
        }
        pub mod source_program {
            solana_sdk::declare_id!("EH45pKy1kzjifB93wEJi91js3S4HETdsteywR7ZCNPn5");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Builtin,
            datapoint_name: "migrate_builtin_to_core_bpf_loader_v4_program",
        };
    }

    pub mod feature_gate_program {
        pub mod feature {
            solana_sdk::declare_id!("8ZYAiJVVEon55wqGt2wZi5oVjxwK4cBUUfwMwHFvkqpB");
        }
        pub mod source_program {
            solana_sdk::declare_id!("CWioXdq2ctv8Z4XdhmzJzgpU5i97ZHZZJVSJUmndV3mk");
        }
        pub const CONFIG: super::CoreBpfMigrationConfig = super::CoreBpfMigrationConfig {
            source_program_id: source_program::id(),
            feature_id: feature::id(),
            migration_target: super::CoreBpfMigrationTargetType::Stateless,
            datapoint_name: "migrate_stateless_to_core_bpf_feature_gate_program",
        };
    }
}
