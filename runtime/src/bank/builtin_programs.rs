#[cfg(test)]
mod tests {
    use {
        crate::bank::*,
        solana_sdk::{
            ed25519_program, feature_set::FeatureSet, genesis_config::create_genesis_config,
        },
    };

    #[test]
    fn test_apply_builtin_program_feature_transitions_for_new_epoch() {
        let (genesis_config, _mint_keypair) = create_genesis_config(100_000);

        let mut bank = Bank::new_for_tests(&genesis_config);
        bank.feature_set = Arc::new(FeatureSet::all_enabled());
        bank.finish_init(&genesis_config, None, false);

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
        bank.finish_init(&genesis_config, None, false);

        // Overwrite precompile accounts to simulate a cluster which already added precompiles.
        for precompile in get_precompiles() {
            bank.store_account(&precompile.program_id, &AccountSharedData::default());
            bank.add_precompiled_account(&precompile.program_id);
        }

        bank.freeze();

        // Simulate starting up from snapshot finishing the initialization for a frozen bank
        bank.finish_init(&genesis_config, None, false);
    }
}

#[cfg(test)]
mod tests_core_bpf_migration {
    use {
        crate::bank::{
            builtins::{
                core_bpf_migration::{tests::TestContext, CoreBpfMigrationConfig},
                BuiltinPrototype, StatelessBuiltinPrototype, BUILTINS, STATELESS_BUILTINS,
            },
            tests::{create_genesis_config, create_simple_test_bank},
            ApplyFeatureActivationsCaller, Bank,
        },
        solana_accounts_db::accounts_db::CalcAccountsHashDataSource,
        solana_sdk::{
            account::{AccountSharedData, ReadableAccount},
            clock::DEFAULT_SLOTS_PER_EPOCH,
            feature::{self, Feature},
            feature_set::FeatureSet,
            native_loader,
            pubkey::Pubkey,
        },
        std::sync::Arc,
        test_case::test_case,
    };

    enum TestPrototype<'a> {
        Builtin(&'a BuiltinPrototype),
        Stateless(&'a StatelessBuiltinPrototype),
    }
    impl<'a> TestPrototype<'a> {
        fn deconstruct(&'a self) -> (&'a Pubkey, &'a CoreBpfMigrationConfig) {
            match self {
                Self::Builtin(prototype) => (
                    &prototype.program_id,
                    prototype.core_bpf_migration_config.as_ref().unwrap(),
                ),
                Self::Stateless(prototype) => (
                    &prototype.program_id,
                    prototype.core_bpf_migration_config.as_ref().unwrap(),
                ),
            }
        }
    }

    #[test_case(TestPrototype::Builtin(&BUILTINS[0]); "system")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[1]); "vote")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[2]); "stake")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[3]); "config")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[4]); "bpf_loader_deprecated")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[5]); "bpf_loader")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[6]); "bpf_loader_upgradeable")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[7]); "compute_budget")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[8]); "address_lookup_table")]
    #[test_case(TestPrototype::Stateless(&STATELESS_BUILTINS[0]); "feature_gate")]
    fn test_core_bpf_migration(prototype: TestPrototype) {
        let mut bank = create_simple_test_bank(0);

        let (builtin_id, config) = prototype.deconstruct();
        let feature_id = &config.feature_id;
        let source_program_id = &config.source_program_id;

        let mut feature_set = FeatureSet::all_enabled();
        feature_set.inactive.insert(*feature_id);
        bank.feature_set = Arc::new(feature_set);

        let test_context = TestContext::new(&bank, builtin_id, source_program_id);

        // Simulate crossing an epoch boundary for a new bank.
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Verify the feature was not activated and the program was not
        // migrated.
        assert!(!bank.feature_set.is_active(feature_id));
        assert!(bank.get_account(source_program_id).is_some());

        // Activate the feature.
        bank.store_account_and_update_capitalization(
            feature_id,
            &feature::create_account(&Feature::default(), 42),
        );

        // Simulate crossing an epoch boundary for a new bank.
        let migration_slot = bank.slot();
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Run the post-migration program checks.
        assert!(bank.feature_set.is_active(feature_id));
        test_context.run_program_checks_post_migration(&bank, migration_slot);

        // Warp to another epoch and check again.
        let collector_id = bank.collector_id;
        let mut new_bank = Bank::warp_from_parent(
            Arc::new(bank),
            &collector_id,
            DEFAULT_SLOTS_PER_EPOCH * 2,
            CalcAccountsHashDataSource::IndexForTests,
        );
        test_context.run_program_checks_post_migration(&new_bank, migration_slot);

        // Simulate crossing an epoch boundary again.
        new_bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);
        test_context.run_program_checks_post_migration(&new_bank, migration_slot);
    }

    // Simulate a failure to migrate the program.
    // Here we want to see that the bank handles the failure gracefully and
    // advances to the next epoch without issue.
    #[test]
    fn test_core_bpf_migration_failure() {
        let mut bank = create_simple_test_bank(0);

        let test_prototype = TestPrototype::Builtin(&BUILTINS[0]); // System program
        let (builtin_id, config) = test_prototype.deconstruct();
        let feature_id = &config.feature_id;
        let source_program_id = &config.source_program_id;

        let mut feature_set = FeatureSet::all_enabled();
        feature_set.inactive.insert(*feature_id);
        bank.feature_set = Arc::new(feature_set);

        // Unused but sets everything up.
        let _test_context = TestContext::new(&bank, builtin_id, source_program_id);

        // Intentionally nuke the source program account to force the migration
        // to fail.
        bank.store_account_and_update_capitalization(
            source_program_id,
            &AccountSharedData::default(),
        );

        // Activate the feature.
        bank.store_account_and_update_capitalization(
            feature_id,
            &feature::create_account(&Feature::default(), 42),
        );

        // Simulate crossing an epoch boundary for a new bank.
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Assert the feature was activated but the bank still has the builtin.
        assert!(bank.feature_set.is_active(feature_id));
        assert!(bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert!(bank.get_account(builtin_id).unwrap().owner() == &native_loader::id());

        // Warp to another epoch and check again.
        let collector_id = bank.collector_id;
        let mut new_bank = Bank::warp_from_parent(
            Arc::new(bank),
            &collector_id,
            DEFAULT_SLOTS_PER_EPOCH * 2,
            CalcAccountsHashDataSource::IndexForTests,
        );
        assert!(new_bank.feature_set.is_active(feature_id));
        assert!(new_bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert!(new_bank.get_account(builtin_id).unwrap().owner() == &native_loader::id());

        // Simulate crossing an epoch boundary again.
        new_bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);
        assert!(new_bank.feature_set.is_active(feature_id));
        assert!(new_bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert!(new_bank.get_account(builtin_id).unwrap().owner() == &native_loader::id());
    }

    // Simulate creating a bank from a snapshot after a migration feature was
    // activated, but the migration failed.
    // Here we want to see that the bank recognizes the failed migration and
    // adds the original builtin to the new bank.
    #[test]
    fn test_core_bpf_migration_init_after_failed_migration() {
        let (genesis_config, _mint_keypair) = create_genesis_config(0);
        let mut bank = Bank::new_for_tests(&genesis_config);

        let test_prototype = TestPrototype::Builtin(&BUILTINS[0]); // System program
        let (builtin_id, config) = test_prototype.deconstruct();
        let feature_id = &config.feature_id;

        // Set up the feature set and activate the migration feature.
        let mut feature_set = FeatureSet::all_enabled();
        feature_set.inactive.insert(*feature_id);
        bank.feature_set = Arc::new(feature_set);

        // This time don't set anything up for the migration
        // (ie. source program accounts).
        // Create the feature account as "activated" rather than "pending".
        bank.store_account_and_update_capitalization(
            feature_id,
            &feature::create_account(
                &Feature {
                    activated_at: Some(0),
                },
                42,
            ),
        );

        // Run `finish_init` to simulate starting up from a snapshot.
        // Clear all builtins to simulate a fresh bank init.
        bank.transaction_processor
            .program_cache
            .write()
            .unwrap()
            .remove_programs(
                bank.transaction_processor
                    .builtin_program_ids
                    .read()
                    .unwrap()
                    .clone()
                    .into_iter(),
            );
        bank.transaction_processor
            .builtin_program_ids
            .write()
            .unwrap()
            .clear();
        bank.finish_init(&genesis_config, None, false);

        // Assert the feature is active and the bank still added the builtin.
        assert!(bank.feature_set.is_active(feature_id));
        assert!(bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert!(bank.get_account(builtin_id).unwrap().owner() == &native_loader::id());

        // Simulate crossing an epoch boundary for a new bank.
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Assert the feature is active but the bank still has the builtin.
        assert!(bank.feature_set.is_active(feature_id));
        assert!(bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert!(bank.get_account(builtin_id).unwrap().owner() == &native_loader::id());
    }

    // Simulate creating a bank from a snapshot after a migration feature was
    // activated and the migration was successful.
    // Here we want to see that the bank recognizes the migration and
    // _does not_ add the original builtin to the new bank.
    #[test]
    fn test_core_bpf_migration_init_after_successful_migration() {
        let (genesis_config, _mint_keypair) = create_genesis_config(0);
        let mut bank = Bank::new_for_tests(&genesis_config);

        let test_prototype = TestPrototype::Builtin(&BUILTINS[0]); // System program
        let (builtin_id, config) = test_prototype.deconstruct();
        let feature_id = &config.feature_id;
        let source_program_id = &config.source_program_id;

        let mut feature_set = FeatureSet::all_enabled();
        feature_set.inactive.insert(*feature_id);
        bank.feature_set = Arc::new(feature_set);

        // Set everything up for migration.
        let test_context = TestContext::new(&bank, builtin_id, source_program_id);

        // Activate the feature to perform the migration.
        let migration_slot = bank.slot();
        bank.store_account_and_update_capitalization(
            feature_id,
            &feature::create_account(&Feature::default(), 42),
        );
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Run `finish_init` to simulate starting up from a snapshot.
        // Clear all builtins to simulate a fresh bank init.
        bank.transaction_processor
            .program_cache
            .write()
            .unwrap()
            .remove_programs(
                bank.transaction_processor
                    .builtin_program_ids
                    .read()
                    .unwrap()
                    .clone()
                    .into_iter(),
            );
        bank.transaction_processor
            .builtin_program_ids
            .write()
            .unwrap()
            .clear();
        bank.finish_init(&genesis_config, None, false);

        // Assert the feature is active, the bank _did not_ add the builtin,
        // and the program remains BPF.
        assert!(bank.feature_set.is_active(feature_id));
        test_context.run_program_checks_post_migration(&bank, migration_slot);
    }
}
