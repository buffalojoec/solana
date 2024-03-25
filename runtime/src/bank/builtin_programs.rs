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
                core_bpf_migration::CoreBpfMigrationConfig, BuiltinPrototype,
                StatelessBuiltinPrototype, BUILTINS, STATELESS_BUILTINS,
            },
            tests::create_simple_test_bank,
            ApplyFeatureActivationsCaller, Bank,
        },
        solana_accounts_db::accounts_db::CalcAccountsHashDataSource,
        solana_sdk::{
            account::{AccountSharedData, ReadableAccount},
            account_utils::StateMut,
            bpf_loader_upgradeable::{self, get_program_data_address, UpgradeableLoaderState},
            clock::{Slot, DEFAULT_SLOTS_PER_EPOCH},
            feature::{self, Feature},
            feature_set::FeatureSet,
            pubkey::Pubkey,
        },
        std::sync::Arc,
        test_case::test_case,
    };

    const PROGRAM_DATA_OFFSET: usize = UpgradeableLoaderState::size_of_programdata_metadata();

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

    struct TestContext {
        builtin_id: Pubkey,
        source_program_id: Pubkey,
        slot: Slot,
        upgrade_authority_address: Option<Pubkey>,
        elf: Vec<u8>,
    }

    impl TestContext {
        // Initialize some test values and set up the source BPF upgradeable
        // program in the bank.
        fn new(
            bank: &mut Bank,
            builtin_id: &Pubkey,
            feature_id: &Pubkey,
            source_program_id: &Pubkey,
        ) -> Self {
            let slot = 99;
            let upgrade_authority_address = Some(Pubkey::new_unique());
            let elf = vec![4; 2000];

            let source_program_data_address = get_program_data_address(source_program_id);

            let source_program_account = AccountSharedData::new_data(
                100_000_000,
                &UpgradeableLoaderState::Program {
                    programdata_address: source_program_data_address,
                },
                &bpf_loader_upgradeable::id(),
            )
            .unwrap();

            let source_program_data_account = {
                let mut data = bincode::serialize(&UpgradeableLoaderState::ProgramData {
                    slot,
                    upgrade_authority_address,
                })
                .unwrap();
                data.extend_from_slice(&elf);

                let mut account =
                    AccountSharedData::new(100_000_000, data.len(), &bpf_loader_upgradeable::id());
                account.set_data(data);
                account
            };

            bank.store_account_and_update_capitalization(
                source_program_id,
                &source_program_account,
            );
            bank.store_account_and_update_capitalization(
                &source_program_data_address,
                &source_program_data_account,
            );

            let mut feature_set = FeatureSet::all_enabled();
            feature_set.inactive.insert(*feature_id);
            bank.feature_set = Arc::new(feature_set);

            Self {
                builtin_id: *builtin_id,
                source_program_id: *source_program_id,
                slot,
                upgrade_authority_address,
                elf,
            }
        }

        // Evaluate the account state of the builtin post-migration.
        // Ensure the builtin program account is now a BPF upgradeable program
        // as well as the bank's builtins and cache have been updated.
        fn run_program_checks_post_migration(&self, bank: &Bank) {
            let program_account = bank.get_account(&self.builtin_id).unwrap();
            let program_data_address = get_program_data_address(&self.builtin_id);

            // Program account is owned by the upgradeable loader.
            assert_eq!(program_account.owner(), &bpf_loader_upgradeable::id());

            // Program account has the correct state, with a pointer to its program
            // data address.
            let program_account_state: UpgradeableLoaderState = program_account.state().unwrap();
            assert_eq!(
                program_account_state,
                UpgradeableLoaderState::Program {
                    programdata_address: program_data_address
                }
            );

            let program_data_account = bank.get_account(&program_data_address).unwrap();

            // Program data account is owned by the upgradeable loader.
            assert_eq!(program_data_account.owner(), &bpf_loader_upgradeable::id());

            // Program data account has the correct state.
            // It should exactly match the original, including upgrade authority
            // and slot.
            let program_data_account_state_metadata: UpgradeableLoaderState =
                bincode::deserialize(&program_data_account.data()[..PROGRAM_DATA_OFFSET]).unwrap();
            assert_eq!(
                program_data_account_state_metadata,
                UpgradeableLoaderState::ProgramData {
                    slot: self.slot,
                    upgrade_authority_address: self.upgrade_authority_address
                },
            );
            assert_eq!(
                &program_data_account.data()[PROGRAM_DATA_OFFSET..],
                &self.elf,
            );

            // The bank's builtins should no longer contain the builtin
            // program ID.
            assert!(!bank.builtin_program_ids.contains(&self.builtin_id));

            // The cache should have unloaded both programs.
            let program_cache = bank.transaction_processor.program_cache.read().unwrap();
            assert!(!program_cache
                .get_flattened_entries(true, true)
                .iter()
                .any(|(id, _)| id == &self.builtin_id || id == &self.source_program_id));
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
    fn test(prototype: TestPrototype) {
        let mut bank = create_simple_test_bank(0);

        let (builtin_id, config) = prototype.deconstruct();
        let feature_id = &config.feature_id;
        let source_program_id = &config.source_program_id;

        let test_context = TestContext::new(&mut bank, builtin_id, feature_id, source_program_id);

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
        bank.apply_feature_activations(ApplyFeatureActivationsCaller::NewFromParent, false);

        // Run the post-migration program checks.
        assert!(bank.feature_set.is_active(feature_id));
        test_context.run_program_checks_post_migration(&bank);

        // Warp to another epoch and check again.
        let collector_id = bank.collector_id;
        let new_bank = Bank::warp_from_parent(
            Arc::new(bank),
            &collector_id,
            DEFAULT_SLOTS_PER_EPOCH * 2,
            CalcAccountsHashDataSource::IndexForTests,
        );
        test_context.run_program_checks_post_migration(&new_bank);
    }
}
