#[cfg(test)]
mod tests {
    use {
        crate::bank::{
            builtins::{BuiltinPrograms, BuiltinPrototype},
            *,
        },
        solana_sdk::{
            ed25519_program, feature::Feature, feature_set::FeatureSet,
            genesis_config::create_genesis_config,
        },
    };

    #[test]
    fn test_override_builtins_on_initialization() {
        let check_bank_builtins = |builtins: Arc<BuiltinPrograms>| {
            let bank = Bank::new_with_paths(
                &GenesisConfig::default(),
                Arc::<RuntimeConfig>::default(),
                Vec::new(),
                None,
                Arc::clone(&builtins),
                AccountSecondaryIndexes::default(),
                AccountShrinkThreshold::default(),
                false,
                Some(ACCOUNTS_DB_CONFIG_FOR_TESTING),
                None,
                Some(Pubkey::new_unique()),
                Arc::default(),
            );

            // Assert the bank's builtins contain all provided builtins.
            builtins
                .iter()
                .filter(|b| b.enable_feature_id.is_none())
                .for_each(|b| {
                    assert!(bank.builtin_program_ids.get(&b.program_id).is_some());
                });
        };

        check_bank_builtins(Arc::<BuiltinPrograms>::default());
        check_bank_builtins(Arc::new(BuiltinPrograms::new(vec![])));
        check_bank_builtins(Arc::new(BuiltinPrograms::new(vec![BuiltinPrototype {
            enable_feature_id: None,
            program_id: solana_system_program::id(),
            name: "system_program",
            entrypoint: solana_system_program::system_processor::Entrypoint::vm,
        }])));
        check_bank_builtins(Arc::new(BuiltinPrograms::new(vec![
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_system_program::id(),
                name: "system_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: Pubkey::new_unique(),
                name: "random_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
        ])));
        check_bank_builtins(Arc::new(BuiltinPrograms::new(vec![
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_system_program::id(),
                name: "system_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: solana_stake_program::id(),
                name: "stake_program",
                entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: Pubkey::new_unique(),
                name: "random_program1",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: None,
                program_id: Pubkey::new_unique(),
                name: "stake_program2",
                entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
            },
        ])));
    }

    #[test]
    fn test_override_builtins_on_feature_activation() {
        let check_bank_builtin_feature_activations = |builtins: Arc<BuiltinPrograms>| {
            let mut bank = Bank::new_with_paths(
                &GenesisConfig::default(),
                Arc::<RuntimeConfig>::default(),
                Vec::new(),
                None,
                Arc::clone(&builtins),
                AccountSecondaryIndexes::default(),
                AccountShrinkThreshold::default(),
                false,
                Some(ACCOUNTS_DB_CONFIG_FOR_TESTING),
                None,
                Some(Pubkey::new_unique()),
                Arc::default(),
            );

            let mut feature_set = FeatureSet::default();
            builtins
                .iter()
                .filter_map(|builtin| builtin.enable_feature_id)
                .for_each(|feature_id| {
                    feature_set.inactive.insert(feature_id);
                    bank.store_account(
                        &feature_id,
                        &feature::create_account(&Feature::default(), 42),
                    );
                });
            bank.feature_set = Arc::new(feature_set.clone());

            // Assert the bank's builtins _do not_ contain the additional
            // builtins, since they have not been enabled.
            builtins.iter().for_each(|b| {
                assert!(bank.builtin_program_ids.get(&b.program_id).is_none());
            });

            bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

            // Assert the bank's builtins contain the additional builtins,
            // since they have now been enabled.
            builtins.iter().for_each(|builtin| {
                assert!(bank.builtin_program_ids.get(&builtin.program_id).is_some());
            });
        };

        check_bank_builtin_feature_activations(Arc::new(BuiltinPrograms::new(vec![])));
        check_bank_builtin_feature_activations(Arc::new(BuiltinPrograms::new(vec![
            BuiltinPrototype {
                enable_feature_id: Some(Pubkey::new_unique()),
                program_id: solana_system_program::id(),
                name: "system_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
        ])));
        check_bank_builtin_feature_activations(Arc::new(BuiltinPrograms::new(vec![
            BuiltinPrototype {
                enable_feature_id: Some(Pubkey::new_unique()),
                program_id: solana_system_program::id(),
                name: "system_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: Some(Pubkey::new_unique()),
                program_id: Pubkey::new_unique(),
                name: "random_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
        ])));
        check_bank_builtin_feature_activations(Arc::new(BuiltinPrograms::new(vec![
            BuiltinPrototype {
                enable_feature_id: Some(Pubkey::new_unique()),
                program_id: solana_system_program::id(),
                name: "system_program",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: Some(Pubkey::new_unique()),
                program_id: solana_stake_program::id(),
                name: "stake_program",
                entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: Some(Pubkey::new_unique()),
                program_id: Pubkey::new_unique(),
                name: "random_program1",
                entrypoint: solana_system_program::system_processor::Entrypoint::vm,
            },
            BuiltinPrototype {
                enable_feature_id: Some(Pubkey::new_unique()),
                program_id: Pubkey::new_unique(),
                name: "stake_program2",
                entrypoint: solana_stake_program::stake_instruction::Entrypoint::vm,
            },
        ])));
    }

    #[test]
    fn test_apply_builtin_program_feature_transitions_for_new_epoch() {
        let (genesis_config, _mint_keypair) = create_genesis_config(100_000);

        let mut bank = Bank::new_for_tests(&genesis_config);
        bank.feature_set = Arc::new(FeatureSet::all_enabled());
        bank.finish_init(&genesis_config, false);

        // Overwrite precompile accounts to simulate a cluster which already added precompiles.
        for precompile in get_precompiles() {
            bank.store_account(&precompile.program_id, &AccountSharedData::default());
            // Simulate cluster which added ed25519 precompile with a system program owner
            if precompile.program_id == ed25519_program::id() {
                bank.add_precompiled_account_with_owner(
                    &precompile.program_id,
                    solana_sdk::system_program::id(),
                );
            } else {
                bank.add_precompiled_account(&precompile.program_id);
            }
        }

        // Normally feature transitions are applied to a bank that hasn't been
        // frozen yet.  Freeze the bank early to ensure that no account changes
        // are made.
        bank.freeze();

        // Simulate crossing an epoch boundary for a new bank
        let only_apply_transitions_for_new_features = true;
        bank.apply_builtin_program_feature_transitions(
            only_apply_transitions_for_new_features,
            &HashSet::new(),
        );
    }

    #[test]
    fn test_startup_from_snapshot_after_precompile_transition() {
        let (genesis_config, _mint_keypair) = create_genesis_config(100_000);

        let mut bank = Bank::new_for_tests(&genesis_config);
        bank.feature_set = Arc::new(FeatureSet::all_enabled());
        bank.finish_init(&genesis_config, false);

        // Overwrite precompile accounts to simulate a cluster which already added precompiles.
        for precompile in get_precompiles() {
            bank.store_account(&precompile.program_id, &AccountSharedData::default());
            bank.add_precompiled_account(&precompile.program_id);
        }

        bank.freeze();

        // Simulate starting up from snapshot finishing the initialization for a frozen bank
        bank.finish_init(&genesis_config, false);
    }
}
