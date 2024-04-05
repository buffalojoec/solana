use {
    crate::transaction_processing_callback::TransactionProcessingCallback,
    solana_program_runtime::loaded_programs::{
        LoadProgramMetrics, LoadedProgram, ProgramRuntimeEnvironment, ProgramRuntimeEnvironments,
        DELAY_VISIBILITY_SLOT_OFFSET,
    },
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount},
        account_utils::StateMut,
        bpf_loader_upgradeable::{self, UpgradeableLoaderState},
        clock::Slot,
        loader_v4::{self, LoaderV4Status},
        pubkey::Pubkey,
    },
};

#[derive(Debug)]
pub(crate) enum ProgramAccountLoadResult {
    AccountNotFound,
    InvalidAccountData(ProgramRuntimeEnvironment),
    ProgramOfLoaderV1orV2(AccountSharedData),
    ProgramOfLoaderV3(AccountSharedData, AccountSharedData, Slot),
    ProgramOfLoaderV4(AccountSharedData, Slot),
}

pub(crate) fn load_program_from_bytes(
    load_program_metrics: &mut LoadProgramMetrics,
    programdata: &[u8],
    loader_key: &Pubkey,
    account_size: usize,
    deployment_slot: Slot,
    program_runtime_environment: ProgramRuntimeEnvironment,
    reloading: bool,
) -> std::result::Result<LoadedProgram, Box<dyn std::error::Error>> {
    if reloading {
        // Safety: this is safe because the program is being reloaded in the cache.
        unsafe {
            LoadedProgram::reload(
                loader_key,
                program_runtime_environment.clone(),
                deployment_slot,
                deployment_slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET),
                programdata,
                account_size,
                load_program_metrics,
            )
        }
    } else {
        LoadedProgram::new(
            loader_key,
            program_runtime_environment.clone(),
            deployment_slot,
            deployment_slot.saturating_add(DELAY_VISIBILITY_SLOT_OFFSET),
            programdata,
            account_size,
            load_program_metrics,
        )
    }
}

pub(crate) fn load_program_accounts<CB: TransactionProcessingCallback>(
    callbacks: &CB,
    pubkey: &Pubkey,
    environments: &ProgramRuntimeEnvironments,
) -> ProgramAccountLoadResult {
    let program_account = match callbacks.get_account_shared_data(pubkey) {
        None => return ProgramAccountLoadResult::AccountNotFound,
        Some(account) => account,
    };

    debug_assert!(solana_bpf_loader_program::check_loader_id(
        program_account.owner()
    ));

    if loader_v4::check_id(program_account.owner()) {
        return solana_loader_v4_program::get_state(program_account.data())
            .ok()
            .and_then(|state| {
                (!matches!(state.status, LoaderV4Status::Retracted)).then_some(state.slot)
            })
            .map(|slot| ProgramAccountLoadResult::ProgramOfLoaderV4(program_account, slot))
            .unwrap_or(ProgramAccountLoadResult::InvalidAccountData(
                environments.program_runtime_v2.clone(),
            ));
    }

    if !bpf_loader_upgradeable::check_id(program_account.owner()) {
        return ProgramAccountLoadResult::ProgramOfLoaderV1orV2(program_account);
    }

    if let Ok(UpgradeableLoaderState::Program {
        programdata_address,
    }) = program_account.state()
    {
        let programdata_account = match callbacks.get_account_shared_data(&programdata_address) {
            None => return ProgramAccountLoadResult::AccountNotFound,
            Some(account) => account,
        };

        if let Ok(UpgradeableLoaderState::ProgramData {
            slot,
            upgrade_authority_address: _,
        }) = programdata_account.state()
        {
            return ProgramAccountLoadResult::ProgramOfLoaderV3(
                program_account,
                programdata_account,
                slot,
            );
        }
    }
    ProgramAccountLoadResult::InvalidAccountData(environments.program_runtime_v1.clone())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        solana_program_runtime::{
            loaded_programs::{
                BlockRelation, ForkGraph, ProgramRuntimeEnvironment, ProgramRuntimeEnvironments,
            },
            solana_rbpf::program::BuiltinProgram,
        },
        solana_sdk::{
            account::WritableAccount,
            bpf_loader,
            feature_set::FeatureSet,
            hash::Hash,
            loader_v4::{self, LoaderV4State, LoaderV4Status},
            rent_collector::RentCollector,
        },
        std::{
            collections::HashMap,
            env,
            fs::{self, File},
            io::Read,
            sync::Arc,
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
        rent_collector: RentCollector,
        feature_set: Arc<FeatureSet>,
        pub account_shared_data: HashMap<Pubkey, AccountSharedData>,
    }

    impl TransactionProcessingCallback for MockBankCallback {
        fn account_matches_owners(&self, account: &Pubkey, owners: &[Pubkey]) -> Option<usize> {
            if let Some(data) = self.account_shared_data.get(account) {
                if data.lamports() == 0 {
                    None
                } else {
                    owners.iter().position(|entry| data.owner() == entry)
                }
            } else {
                None
            }
        }

        fn get_account_shared_data(&self, pubkey: &Pubkey) -> Option<AccountSharedData> {
            self.account_shared_data.get(pubkey).cloned()
        }

        fn get_last_blockhash_and_lamports_per_signature(&self) -> (Hash, u64) {
            (Hash::new_unique(), 2)
        }

        fn get_rent_collector(&self) -> &RentCollector {
            &self.rent_collector
        }

        fn get_feature_set(&self) -> Arc<FeatureSet> {
            self.feature_set.clone()
        }
    }

    #[test]
    fn test_load_program_accounts_account_not_found() {
        let mut mock_bank = MockBankCallback::default();
        let key = Pubkey::new_unique();
        let environment = ProgramRuntimeEnvironments::default();

        let result = load_program_accounts(&mock_bank, &key, &environment);

        assert!(matches!(result, ProgramAccountLoadResult::AccountNotFound));

        let mut account_data = AccountSharedData::default();
        account_data.set_owner(bpf_loader_upgradeable::id());
        let state = UpgradeableLoaderState::Program {
            programdata_address: Pubkey::new_unique(),
        };
        account_data.set_data(bincode::serialize(&state).unwrap());
        mock_bank
            .account_shared_data
            .insert(key, account_data.clone());

        let result = load_program_accounts(&mock_bank, &key, &environment);
        assert!(matches!(result, ProgramAccountLoadResult::AccountNotFound));

        account_data.set_data(Vec::new());
        mock_bank.account_shared_data.insert(key, account_data);

        let result = load_program_accounts(&mock_bank, &key, &environment);

        assert!(matches!(
            result,
            ProgramAccountLoadResult::InvalidAccountData(_)
        ));
    }

    #[test]
    fn test_load_program_accounts_loader_v4() {
        let key = Pubkey::new_unique();
        let mut mock_bank = MockBankCallback::default();
        let mut account_data = AccountSharedData::default();
        account_data.set_owner(loader_v4::id());
        let environment = ProgramRuntimeEnvironments::default();
        mock_bank
            .account_shared_data
            .insert(key, account_data.clone());

        let result = load_program_accounts(&mock_bank, &key, &environment);
        assert!(matches!(
            result,
            ProgramAccountLoadResult::InvalidAccountData(_)
        ));

        account_data.set_data(vec![0; 64]);
        mock_bank
            .account_shared_data
            .insert(key, account_data.clone());
        let result = load_program_accounts(&mock_bank, &key, &environment);
        assert!(matches!(
            result,
            ProgramAccountLoadResult::InvalidAccountData(_)
        ));

        let loader_data = LoaderV4State {
            slot: 25,
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
            .insert(key, account_data.clone());

        let result = load_program_accounts(&mock_bank, &key, &environment);

        match result {
            ProgramAccountLoadResult::ProgramOfLoaderV4(data, slot) => {
                assert_eq!(data, account_data);
                assert_eq!(slot, 25);
            }

            _ => panic!("Invalid result"),
        }
    }

    #[test]
    fn test_load_program_accounts_loader_v1_or_v2() {
        let key = Pubkey::new_unique();
        let mut mock_bank = MockBankCallback::default();
        let mut account_data = AccountSharedData::default();
        account_data.set_owner(bpf_loader::id());
        let environment = ProgramRuntimeEnvironments::default();
        mock_bank
            .account_shared_data
            .insert(key, account_data.clone());

        let result = load_program_accounts(&mock_bank, &key, &environment);
        match result {
            ProgramAccountLoadResult::ProgramOfLoaderV1orV2(data) => {
                assert_eq!(data, account_data);
            }
            _ => panic!("Invalid result"),
        }
    }

    #[test]
    fn test_load_program_accounts_success() {
        let key1 = Pubkey::new_unique();
        let key2 = Pubkey::new_unique();
        let mut mock_bank = MockBankCallback::default();
        let environment = ProgramRuntimeEnvironments::default();

        let mut account_data = AccountSharedData::default();
        account_data.set_owner(bpf_loader_upgradeable::id());

        let state = UpgradeableLoaderState::Program {
            programdata_address: key2,
        };
        account_data.set_data(bincode::serialize(&state).unwrap());
        mock_bank
            .account_shared_data
            .insert(key1, account_data.clone());

        let state = UpgradeableLoaderState::ProgramData {
            slot: 25,
            upgrade_authority_address: None,
        };
        let mut account_data2 = AccountSharedData::default();
        account_data2.set_data(bincode::serialize(&state).unwrap());
        mock_bank
            .account_shared_data
            .insert(key2, account_data2.clone());

        let result = load_program_accounts(&mock_bank, &key1, &environment);

        match result {
            ProgramAccountLoadResult::ProgramOfLoaderV3(data1, data2, slot) => {
                assert_eq!(data1, account_data);
                assert_eq!(data2, account_data2);
                assert_eq!(slot, 25);
            }

            _ => panic!("Invalid result"),
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
    fn test_load_program_from_bytes() {
        let buffer = load_test_program();

        let mut metrics = LoadProgramMetrics::default();
        let loader = bpf_loader_upgradeable::id();
        let size = buffer.len();
        let slot = 2;
        let environment = ProgramRuntimeEnvironment::new(BuiltinProgram::new_mock());

        let result = load_program_from_bytes(
            &mut metrics,
            &buffer,
            &loader,
            size,
            slot,
            environment.clone(),
            false,
        );

        assert!(result.is_ok());

        let result = load_program_from_bytes(
            &mut metrics,
            &buffer,
            &loader,
            size,
            slot,
            environment,
            true,
        );

        assert!(result.is_ok());
    }
}
