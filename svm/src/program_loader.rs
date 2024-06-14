use {
    crate::loader::{Loader, ProgramAccountLoadResult},
    solana_program_runtime::{
        loaded_programs::{
            LoadProgramMetrics, ProgramCacheEntry, ProgramCacheEntryOwner, ProgramCacheEntryType,
            ProgramRuntimeEnvironments,
        },
        timings::ExecuteDetailsTimings,
    },
    solana_sdk::{
        account::ReadableAccount,
        bpf_loader_upgradeable::UpgradeableLoaderState,
        clock::Slot,
        epoch_schedule::EpochSchedule,
        instruction::InstructionError,
        loader_v4::{self, LoaderV4State},
        pubkey::Pubkey,
    },
    std::sync::Arc,
};

/// Loads the program with the given pubkey.
///
/// If the account doesn't exist it returns `None`. If the account does exist, it must be a program
/// account (belong to one of the program loaders). Returns `Some(InvalidAccountData)` if the program
/// account is `Closed`, contains invalid data or any of the programdata accounts are invalid.
pub fn load_program_with_pubkey<L: Loader>(
    loader: &L,
    environments: &ProgramRuntimeEnvironments,
    pubkey: &Pubkey,
    slot: Slot,
    _epoch_schedule: &EpochSchedule,
    reload: bool,
) -> Option<Arc<ProgramCacheEntry>> {
    let mut load_program_metrics = LoadProgramMetrics {
        program_id: pubkey.to_string(),
        ..LoadProgramMetrics::default()
    };

    let loaded_program = match loader.load_program_accounts(pubkey)? {
        ProgramAccountLoadResult::InvalidAccountData(owner) => Ok(
            ProgramCacheEntry::new_tombstone(slot, owner, ProgramCacheEntryType::Closed),
        ),

        ProgramAccountLoadResult::ProgramOfLoaderV1(program_account) => loader
            .load_program_from_bytes(
                &mut load_program_metrics,
                program_account.data(),
                program_account.owner(),
                program_account.data().len(),
                0,
                environments.program_runtime_v1.clone(),
                reload,
            )
            .map_err(|_| (0, ProgramCacheEntryOwner::LoaderV1)),

        ProgramAccountLoadResult::ProgramOfLoaderV2(program_account) => loader
            .load_program_from_bytes(
                &mut load_program_metrics,
                program_account.data(),
                program_account.owner(),
                program_account.data().len(),
                0,
                environments.program_runtime_v1.clone(),
                reload,
            )
            .map_err(|_| (0, ProgramCacheEntryOwner::LoaderV2)),

        ProgramAccountLoadResult::ProgramOfLoaderV3(program_account, programdata_account, slot) => {
            programdata_account
                .data()
                .get(UpgradeableLoaderState::size_of_programdata_metadata()..)
                .ok_or(Box::new(InstructionError::InvalidAccountData).into())
                .and_then(|programdata| {
                    loader.load_program_from_bytes(
                        &mut load_program_metrics,
                        programdata,
                        program_account.owner(),
                        program_account
                            .data()
                            .len()
                            .saturating_add(programdata_account.data().len()),
                        slot,
                        environments.program_runtime_v1.clone(),
                        reload,
                    )
                })
                .map_err(|_| (slot, ProgramCacheEntryOwner::LoaderV3))
        }

        ProgramAccountLoadResult::ProgramOfLoaderV4(program_account, slot) => program_account
            .data()
            .get(LoaderV4State::program_data_offset()..)
            .ok_or(Box::new(InstructionError::InvalidAccountData).into())
            .and_then(|elf_bytes| {
                loader.load_program_from_bytes(
                    &mut load_program_metrics,
                    elf_bytes,
                    &loader_v4::id(),
                    program_account.data().len(),
                    slot,
                    environments.program_runtime_v2.clone(),
                    reload,
                )
            })
            .map_err(|_| (slot, ProgramCacheEntryOwner::LoaderV4)),
    }
    .unwrap_or_else(|(slot, owner)| {
        let env = if let ProgramCacheEntryOwner::LoaderV4 = &owner {
            environments.program_runtime_v2.clone()
        } else {
            environments.program_runtime_v1.clone()
        };
        ProgramCacheEntry::new_tombstone(
            slot,
            owner,
            ProgramCacheEntryType::FailedVerification(env),
        )
    });

    let mut timings = ExecuteDetailsTimings::default();
    load_program_metrics.submit_datapoint(&mut timings);
    loaded_program.update_access_slot(slot);
    Some(Arc::new(loaded_program))
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::transaction_processor::TransactionBatchProcessor,
        solana_program_runtime::loaded_programs::{
            BlockRelation, ForkGraph, ProgramRuntimeEnvironments,
        },
        solana_sdk::{
            account::{AccountSharedData, WritableAccount},
            bpf_loader, bpf_loader_upgradeable,
            loader_v4::{self, LoaderV4Status},
        },
        std::{
            cell::RefCell,
            collections::HashMap,
            env,
            fs::{self, File},
            io::Read,
        },
    };

    struct TestForkGraph {}

    impl ForkGraph for TestForkGraph {
        fn relationship(&self, _a: Slot, _b: Slot) -> BlockRelation {
            BlockRelation::Unknown
        }
    }

    #[derive(Default, Clone)]
    pub struct MockBankCallback {
        pub account_shared_data: RefCell<HashMap<Pubkey, AccountSharedData>>,
    }

    impl Loader for MockBankCallback {
        fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
            if let Some(data) = self.account_shared_data.borrow().get(account) {
                if data.lamports() == 0 {
                    None
                } else {
                    owners.iter().position(|entry| data.owner() == entry)
                }
            } else {
                None
            }
        }

        fn load_account(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
            self.account_shared_data.borrow().get(pubkey).cloned()
        }

        fn add_builtin_account(&self, name: &str, program_id: &Pubkey) {
            let mut account_data = AccountSharedData::default();
            account_data.set_data(name.as_bytes().to_vec());
            self.account_shared_data
                .borrow_mut()
                .insert(*program_id, account_data);
        }
    }

    fn load_test_program() -> Vec<u8> {
        let mut dir = env::current_dir().unwrap();
        dir.push("tests");
        dir.push("example-programs");
        dir.push("hello-solana");
        dir.push("hello_solana_program.so");
        let mut file = File::open(dir.clone()).expect("file not found");
        let metadata = fs::metadata(dir).expect("Unable to read metadata");
        let mut buffer = vec![0; metadata.len() as usize];
        file.read_exact(&mut buffer).expect("Buffer overflow");
        buffer
    }

    #[test]
    fn test_load_program_not_found() {
        let mock_bank = MockBankCallback::default();
        let key = Pubkey::new_unique();
        let batch_processor = TransactionBatchProcessor::<TestForkGraph>::default();

        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(50).unwrap(),
            &key,
            500,
            &batch_processor.epoch_schedule,
            false,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_load_program_invalid_account_data() {
        let key = Pubkey::new_unique();
        let mock_bank = MockBankCallback::default();
        let mut account_data = AccountSharedData::default();
        account_data.set_owner(loader_v4::id());
        let batch_processor = TransactionBatchProcessor::<TestForkGraph>::default();
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key, account_data.clone());

        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(20).unwrap(),
            &key,
            0, // Slot 0
            &batch_processor.epoch_schedule,
            false,
        );

        let loaded_program = ProgramCacheEntry::new_tombstone(
            0, // Slot 0
            ProgramCacheEntryOwner::LoaderV4,
            ProgramCacheEntryType::FailedVerification(
                batch_processor
                    .get_environments_for_epoch(20)
                    .unwrap()
                    .program_runtime_v1,
            ),
        );
        assert_eq!(result.unwrap(), Arc::new(loaded_program));
    }

    #[test]
    fn test_load_program_program_loader_v1_or_v2() {
        let key = Pubkey::new_unique();
        let mock_bank = MockBankCallback::default();
        let mut account_data = AccountSharedData::default();
        account_data.set_owner(bpf_loader::id());
        let batch_processor = TransactionBatchProcessor::<TestForkGraph>::default();
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key, account_data.clone());

        // This should return an error
        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(20).unwrap(),
            &key,
            200,
            &batch_processor.epoch_schedule,
            false,
        );
        let loaded_program = ProgramCacheEntry::new_tombstone(
            0,
            ProgramCacheEntryOwner::LoaderV2,
            ProgramCacheEntryType::FailedVerification(
                batch_processor
                    .get_environments_for_epoch(20)
                    .unwrap()
                    .program_runtime_v1,
            ),
        );
        assert_eq!(result.unwrap(), Arc::new(loaded_program));

        let buffer = load_test_program();
        account_data.set_data(buffer);

        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key, account_data.clone());

        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(20).unwrap(),
            &key,
            200,
            &batch_processor.epoch_schedule,
            false,
        );

        let environments = ProgramRuntimeEnvironments::default();
        let expected = mock_bank.load_program_from_bytes(
            &mut LoadProgramMetrics::default(),
            account_data.data(),
            account_data.owner(),
            account_data.data().len(),
            0,
            environments.program_runtime_v1.clone(),
            false,
        );

        assert_eq!(result.unwrap(), Arc::new(expected.unwrap()));
    }

    #[test]
    fn test_load_program_program_loader_v3() {
        let key1 = Pubkey::new_unique();
        let key2 = Pubkey::new_unique();
        let mock_bank = MockBankCallback::default();
        let batch_processor = TransactionBatchProcessor::<TestForkGraph>::default();

        let mut account_data = AccountSharedData::default();
        account_data.set_owner(bpf_loader_upgradeable::id());

        let state = UpgradeableLoaderState::Program {
            programdata_address: key2,
        };
        account_data.set_data(bincode::serialize(&state).unwrap());
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key1, account_data.clone());

        let state = UpgradeableLoaderState::ProgramData {
            slot: 0,
            upgrade_authority_address: None,
        };
        let mut account_data2 = AccountSharedData::default();
        account_data2.set_data(bincode::serialize(&state).unwrap());
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key2, account_data2.clone());

        // This should return an error
        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(0).unwrap(),
            &key1,
            0,
            &batch_processor.epoch_schedule,
            false,
        );
        let loaded_program = ProgramCacheEntry::new_tombstone(
            0,
            ProgramCacheEntryOwner::LoaderV3,
            ProgramCacheEntryType::FailedVerification(
                batch_processor
                    .get_environments_for_epoch(0)
                    .unwrap()
                    .program_runtime_v1,
            ),
        );
        assert_eq!(result.unwrap(), Arc::new(loaded_program));

        let mut buffer = load_test_program();
        let mut header = bincode::serialize(&state).unwrap();
        let mut complement = vec![
            0;
            std::cmp::max(
                0,
                UpgradeableLoaderState::size_of_programdata_metadata() - header.len()
            )
        ];
        header.append(&mut complement);
        header.append(&mut buffer);
        account_data.set_data(header);

        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key2, account_data.clone());

        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(20).unwrap(),
            &key1,
            200,
            &batch_processor.epoch_schedule,
            false,
        );

        let data = account_data.data();
        account_data
            .set_data(data[UpgradeableLoaderState::size_of_programdata_metadata()..].to_vec());

        let environments = ProgramRuntimeEnvironments::default();
        let expected = mock_bank.load_program_from_bytes(
            &mut LoadProgramMetrics::default(),
            account_data.data(),
            account_data.owner(),
            account_data.data().len(),
            0,
            environments.program_runtime_v1.clone(),
            false,
        );
        assert_eq!(result.unwrap(), Arc::new(expected.unwrap()));
    }

    #[test]
    fn test_load_program_of_loader_v4() {
        let key = Pubkey::new_unique();
        let mock_bank = MockBankCallback::default();
        let mut account_data = AccountSharedData::default();
        account_data.set_owner(loader_v4::id());
        let batch_processor = TransactionBatchProcessor::<TestForkGraph>::default();

        let loader_data = LoaderV4State {
            slot: 0,
            authority_address: Pubkey::new_unique(),
            status: LoaderV4Status::Deployed,
        };
        let encoded = unsafe {
            std::mem::transmute::<&LoaderV4State, &[u8; LoaderV4State::program_data_offset()]>(
                &loader_data,
            )
        };
        account_data.set_data(encoded.to_vec());
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key, account_data.clone());

        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(0).unwrap(),
            &key,
            0,
            &batch_processor.epoch_schedule,
            false,
        );
        let loaded_program = ProgramCacheEntry::new_tombstone(
            0,
            ProgramCacheEntryOwner::LoaderV4,
            ProgramCacheEntryType::FailedVerification(
                batch_processor
                    .get_environments_for_epoch(0)
                    .unwrap()
                    .program_runtime_v1,
            ),
        );
        assert_eq!(result.unwrap(), Arc::new(loaded_program));

        let mut header = account_data.data().to_vec();
        let mut complement =
            vec![0; std::cmp::max(0, LoaderV4State::program_data_offset() - header.len())];
        header.append(&mut complement);

        let mut buffer = load_test_program();
        header.append(&mut buffer);

        account_data.set_data(header);
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key, account_data.clone());

        let result = load_program_with_pubkey(
            &mock_bank,
            &batch_processor.get_environments_for_epoch(20).unwrap(),
            &key,
            200,
            &batch_processor.epoch_schedule,
            false,
        );

        let data = account_data.data()[LoaderV4State::program_data_offset()..].to_vec();
        account_data.set_data(data);
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key, account_data.clone());

        let environments = ProgramRuntimeEnvironments::default();
        let expected = mock_bank.load_program_from_bytes(
            &mut LoadProgramMetrics::default(),
            account_data.data(),
            account_data.owner(),
            account_data.data().len(),
            0,
            environments.program_runtime_v1.clone(),
            false,
        );
        assert_eq!(result.unwrap(), Arc::new(expected.unwrap()));
    }

    #[test]
    fn test_load_program_environment() {
        let key = Pubkey::new_unique();
        let mock_bank = MockBankCallback::default();
        let mut account_data = AccountSharedData::default();
        account_data.set_owner(bpf_loader::id());
        let batch_processor = TransactionBatchProcessor::<TestForkGraph>::default();

        let upcoming_environments = ProgramRuntimeEnvironments::default();
        let current_environments = {
            let mut program_cache = batch_processor.program_cache.write().unwrap();
            program_cache.upcoming_environments = Some(upcoming_environments.clone());
            program_cache.environments.clone()
        };
        mock_bank
            .account_shared_data
            .borrow_mut()
            .insert(key, account_data.clone());

        for is_upcoming_env in [false, true] {
            let result = load_program_with_pubkey(
                &mock_bank,
                &batch_processor
                    .get_environments_for_epoch(is_upcoming_env as u64)
                    .unwrap(),
                &key,
                200,
                &batch_processor.epoch_schedule,
                false,
            )
            .unwrap();
            assert_ne!(
                is_upcoming_env,
                Arc::ptr_eq(
                    result.program.get_environment().unwrap(),
                    &current_environments.program_runtime_v1,
                )
            );
            assert_eq!(
                is_upcoming_env,
                Arc::ptr_eq(
                    result.program.get_environment().unwrap(),
                    &upcoming_environments.program_runtime_v1,
                )
            );
        }
    }
}
