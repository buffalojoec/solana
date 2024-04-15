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
            test_utils::goto_end_of_slot,
            tests::{
                create_genesis_config, new_bank_from_parent_with_bank_forks,
                new_from_parent_with_fork_next_slot,
            },
            Bank, LAMPORTS_PER_SOL,
        },
        solana_sdk::{
            account::{AccountSharedData, ReadableAccount},
            bpf_loader_upgradeable::{self, get_program_data_address, UpgradeableLoaderState},
            feature::{self, Feature},
            feature_set::FeatureSet,
            instruction::Instruction,
            message::Message,
            native_loader,
            pubkey::Pubkey,
            signature::Signer,
            transaction::Transaction,
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

    // This test can't be used to the `compute_budget` program, unless a valid
    // `compute_budget` program is provided as the replacement (source).
    // See program_runtime::compute_budget_processor::process_compute_budget_instructions`.`
    // It also can't test the `bpf_loader_upgradeable` program, as it's used in
    // a particular way on transaction instruction processing.
    // See `solana_svm::account_loader::load_transaction_accounts`.
    #[test_case(TestPrototype::Builtin(&BUILTINS[0]); "system")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[1]); "vote")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[2]); "stake")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[3]); "config")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[4]); "bpf_loader_deprecated")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[5]); "bpf_loader")]
    #[test_case(TestPrototype::Builtin(&BUILTINS[8]); "address_lookup_table")]
    #[test_case(TestPrototype::Stateless(&STATELESS_BUILTINS[0]); "feature_gate")]
    fn test_core_bpf_migration(prototype: TestPrototype) {
        let (genesis_config, mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let mut root_bank = Bank::new_for_tests(&genesis_config);

        let (builtin_id, config) = prototype.deconstruct();
        let feature_id = &config.feature_id;
        let source_program_id = &config.source_program_id;

        // Add the feature to the bank's inactive feature set.
        let mut feature_set = FeatureSet::all_enabled();
        feature_set.inactive.insert(*feature_id);
        root_bank.feature_set = Arc::new(feature_set);

        // Initialize the accounts for the source program.
        let test_context = TestContext::new(&root_bank, builtin_id, source_program_id);

        let (bank, bank_forks) = root_bank.wrap_with_bank_forks_for_tests();

        // Advance one slot so that the source program becomes effective in the
        // program cache.
        goto_end_of_slot(bank.clone());
        let bank = new_from_parent_with_fork_next_slot(bank, bank_forks.as_ref());

        // Successfully invoke the source program.
        bank.process_transaction(&Transaction::new(
            &vec![&mint_keypair],
            Message::new(
                &[Instruction::new_with_bytes(
                    *source_program_id,
                    &[],
                    Vec::new(),
                )],
                Some(&mint_keypair.pubkey()),
            ),
            bank.last_blockhash(),
        ))
        .unwrap();

        // Advance to the next epoch without activating the feature.
        let bank = new_bank_from_parent_with_bank_forks(&bank_forks, bank, &Pubkey::default(), 33);

        // Assert the feature was not activated and the program was not
        // migrated.
        assert!(!bank.feature_set.is_active(feature_id));
        assert!(bank.get_account(source_program_id).is_some());

        // Store the account to activate the feature.
        bank.store_account_and_update_capitalization(
            feature_id,
            &feature::create_account(&Feature::default(), 42),
        );

        // Advance the bank to cross the epoch boundary and activate the
        // feature.
        goto_end_of_slot(bank.clone());
        let bank = new_bank_from_parent_with_bank_forks(&bank_forks, bank, &Pubkey::default(), 96);

        // Run the post-migration program checks.
        assert!(bank.feature_set.is_active(feature_id));
        test_context.run_program_checks_post_migration(&bank, 96);

        // Advance one slot so that the new target program becomes effective in
        // the program cache.
        goto_end_of_slot(bank.clone());
        let bank = new_from_parent_with_fork_next_slot(bank, bank_forks.as_ref());

        // Successfully invoke the new target program.
        bank.process_transaction(&Transaction::new(
            &vec![&mint_keypair],
            Message::new(
                &[Instruction::new_with_bytes(*builtin_id, &[], Vec::new())],
                Some(&mint_keypair.pubkey()),
            ),
            bank.last_blockhash(),
        ))
        .unwrap();

        // Simulate crossing another epoch boundary for a new bank.
        goto_end_of_slot(bank.clone());
        let bank = new_bank_from_parent_with_bank_forks(&bank_forks, bank, &Pubkey::default(), 224);

        // Run the post-migration program checks again.
        assert!(bank.feature_set.is_active(feature_id));
        test_context.run_program_checks_post_migration(&bank, 96);

        // Again, successfully invoke the new target program.
        bank.process_transaction(&Transaction::new(
            &vec![&mint_keypair],
            Message::new(
                &[Instruction::new_with_bytes(*builtin_id, &[], Vec::new())],
                Some(&mint_keypair.pubkey()),
            ),
            bank.last_blockhash(),
        ))
        .unwrap();
    }

    // Simulate a failure to migrate the program.
    // Here we want to see that the bank handles the failure gracefully and
    // advances to the next epoch without issue.
    #[test]
    fn test_core_bpf_migration_failure() {
        let (genesis_config, _mint_keypair) = create_genesis_config(0);
        let mut root_bank = Bank::new_for_tests(&genesis_config);

        let test_prototype = TestPrototype::Builtin(&BUILTINS[0]); // System program
        let (builtin_id, config) = test_prototype.deconstruct();
        let feature_id = &config.feature_id;
        let source_program_id = &config.source_program_id;

        // Add the feature to the bank's inactive feature set.
        let mut feature_set = FeatureSet::all_enabled();
        feature_set.inactive.insert(*feature_id);
        root_bank.feature_set = Arc::new(feature_set);

        // Initialize the accounts for the source program.
        let _test_context = TestContext::new(&root_bank, builtin_id, source_program_id);

        let (bank, bank_forks) = root_bank.wrap_with_bank_forks_for_tests();

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

        // Advance the bank to cross the epoch boundary and activate the
        // feature.
        goto_end_of_slot(bank.clone());
        let bank = new_bank_from_parent_with_bank_forks(&bank_forks, bank, &Pubkey::default(), 33);

        // Assert the feature _was_ activated but the program was not migrated.
        assert!(bank.feature_set.is_active(feature_id));
        assert!(bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert_eq!(
            bank.get_account(builtin_id).unwrap().owner(),
            &native_loader::id()
        );

        // Simulate crossing an epoch boundary again.
        goto_end_of_slot(bank.clone());
        let bank = new_bank_from_parent_with_bank_forks(&bank_forks, bank, &Pubkey::default(), 96);
        assert!(bank.feature_set.is_active(feature_id));
        assert!(bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert_eq!(
            bank.get_account(builtin_id).unwrap().owner(),
            &native_loader::id()
        );
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
        let (bank, bank_forks) = bank.wrap_with_bank_forks_for_tests();
        goto_end_of_slot(bank.clone());
        let bank = new_bank_from_parent_with_bank_forks(&bank_forks, bank, &Pubkey::default(), 33);

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
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let mut bank = Bank::new_for_tests(&genesis_config);

        let test_prototype = TestPrototype::Builtin(&BUILTINS[0]); // System program
        let (builtin_id, config) = test_prototype.deconstruct();
        let feature_id = &config.feature_id;
        let source_program_id = &config.source_program_id;

        // Set up the feature as _active_.
        let mut feature_set = FeatureSet::all_enabled();
        feature_set.inactive.insert(*feature_id);
        bank.feature_set = Arc::new(feature_set);
        bank.store_account_and_update_capitalization(
            feature_id,
            &feature::create_account(&Feature::default(), 42),
        );
        bank.activate_feature(feature_id);

        assert!(bank.feature_set.is_active(feature_id));

        // Set up the accounts as post-migration.
        let _test_context = TestContext::new(&bank, builtin_id, source_program_id);
        {
            let source_program_data_address = get_program_data_address(source_program_id);
            let source_program_data_account =
                bank.get_account(&source_program_data_address).unwrap();
            let source_program_state = UpgradeableLoaderState::Program {
                programdata_address: source_program_data_address,
            };
            let lamports = bank.get_minimum_balance_for_rent_exemption(
                bincode::serialized_size(&source_program_state).unwrap() as usize,
            );
            bank.store_account_and_update_capitalization(
                source_program_id,
                &AccountSharedData::new_data(
                    lamports,
                    &source_program_state,
                    &bpf_loader_upgradeable::id(),
                )
                .unwrap(),
            );
            bank.store_account_and_update_capitalization(
                &source_program_data_address,
                &source_program_data_account,
            );
        }

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
        assert!(bank
            .transaction_processor
            .builtin_program_ids
            .read()
            .unwrap()
            .contains(builtin_id));
        assert!(bank.get_account(builtin_id).unwrap().owner() == &native_loader::id());
    }
}
