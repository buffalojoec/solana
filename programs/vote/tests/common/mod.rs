#![allow(dead_code)]

use {
    solana_account::{Account, AccountSharedData, state_traits::StateMutWincode},
    solana_clock::Clock,
    solana_epoch_schedule::EpochSchedule,
    solana_instruction::{AccountMeta, Instruction},
    solana_instruction_error::InstructionError,
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_sdk_ids::sysvar,
    solana_slot_hashes::SlotHashes,
    solana_svm::conformance::{
        callback::ConformanceCallback,
        instr::{context::InstrContext, harness::execute_instr_with_callback},
        programs::keyed_account_for_builtin_pubkey,
    },
    solana_svm_feature_set::SVMFeatureSet,
    solana_sysvar_id::SysvarId,
    solana_vote_program::{
        id,
        vote_processor::DEFAULT_COMPUTE_UNITS,
        vote_state::{
            VoteInit, VoteStateV3, VoteStateV4, VoteStateVersions,
            create_v4_account_with_authorized, handler::VoteStateHandler,
        },
    },
    solana_vote_interface::state::BLS_PUBLIC_KEY_COMPRESSED_SIZE,
    std::str::FromStr,
};

pub fn create_sysvar_account<T>(value: &T) -> AccountSharedData
where
    T: wincode::SchemaWrite<solana_account::WincodeConfig, Src = T>
        + for<'de> wincode::SchemaRead<'de, solana_account::WincodeConfig, Dst = T>
        + SysvarId,
{
    let serialized_len = wincode::config::serialized_size(value, solana_account::WINCODE_CONFIG)
        .unwrap() as usize;
    let canonical_data_len = match T::id() {
        sysvar::clock::ID => solana_clock::SIZE,
        sysvar::epoch_schedule::ID => solana_epoch_schedule::SIZE,
        sysvar::rent::ID => solana_rent::SIZE,
        sysvar::slot_hashes::ID => solana_slot_hashes::SIZE,
        id => panic!("unsupported sysvar: {id}"),
    };
    let required_data_len = canonical_data_len.max(serialized_len);
    AccountSharedData::new_data_with_space(1, value, required_data_len, &sysvar::id()).unwrap()
}

pub fn vote_state_size_of() -> usize {
    VoteStateV4::size_of()
}

pub fn deserialize_vote_state_for_test(
    account_data: &[u8],
    vote_pubkey: &Pubkey,
) -> VoteStateHandler {
    VoteStateHandler::new_v4(VoteStateV4::deserialize(account_data, vote_pubkey).unwrap())
}

pub struct VoteAccountTestFixtureWithAuthorities {
    pub vote_account: AccountSharedData,
    pub vote_pubkey: Pubkey,
    pub voter_base_key: Pubkey,
    pub voter_owner: Pubkey,
    pub voter_seed: String,
    pub withdrawer_base_key: Pubkey,
    pub withdrawer_owner: Pubkey,
    pub withdrawer_seed: String,
}

pub fn create_default_account() -> AccountSharedData {
    AccountSharedData::empty(0, &Pubkey::new_unique())
}

#[derive(Clone, Copy, Default)]
pub struct VoteProgramFeatures {
    pub block_revenue_sharing: bool,
    pub vote_account_initialize_v2: bool,
}

impl VoteProgramFeatures {
    pub fn all_enabled() -> Self {
        Self {
            block_revenue_sharing: true,
            vote_account_initialize_v2: true,
        }
    }
}

pub fn process_instruction(
    features: VoteProgramFeatures,
    instruction_data: &[u8],
    transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
    instruction_accounts: Vec<AccountMeta>,
    expected_result: Result<(), InstructionError>,
) -> Vec<AccountSharedData> {
    process_instruction_with_cu_check(
        features,
        instruction_data,
        transaction_accounts,
        instruction_accounts,
        expected_result,
        DEFAULT_COMPUTE_UNITS,
    )
}

/// Runs a single vote-program instruction through the SVM conformance harness.
///
/// The returned accounts are in the same order as `transaction_accounts`, so
/// callers can index them positionally. The compute units consumed are pinned
/// to `expected_cus` by clamping the harness budget and asserting that no
/// budget remains after execution.
pub fn process_instruction_with_cu_check(
    features: VoteProgramFeatures,
    instruction_data: &[u8],
    transaction_accounts: Vec<(Pubkey, AccountSharedData)>,
    instruction_accounts: Vec<AccountMeta>,
    expected_result: Result<(), InstructionError>,
    expected_cus: u64,
) -> Vec<AccountSharedData> {
    let VoteProgramFeatures {
        block_revenue_sharing,
        vote_account_initialize_v2,
    } = features;
    let feature_set = SVMFeatureSet {
        block_revenue_sharing,
        vote_account_initialize_v2,
        ..SVMFeatureSet::all_enabled()
    };

    let instruction = Instruction {
        program_id: id(),
        accounts: instruction_accounts,
        data: instruction_data.to_vec(),
    };

    let mut accounts: Vec<(Pubkey, Account)> = transaction_accounts
        .iter()
        .map(|(key, account)| (*key, account.clone().into()))
        .collect();
    add_required_accounts(&mut accounts, &instruction.program_id);

    let context = InstrContext {
        feature_set,
        accounts,
        instruction,
        cu_avail: expected_cus,
    };

    let effects = execute_instr_with_callback(&context, &ConformanceCallback::default());

    assert_eq!(effects.result, expected_result.err());
    assert_eq!(
        expected_cus.saturating_sub(effects.cu_avail),
        expected_cus,
        "Expected {expected_cus} CU consumed, got {}",
        expected_cus.saturating_sub(effects.cu_avail),
    );

    // Return the resulting accounts in the same positional order as the input
    // `transaction_accounts`, converting back to `AccountSharedData`. An account
    // the harness dropped (e.g. closed to zero lamports) comes back as default.
    transaction_accounts
        .iter()
        .map(|(key, _)| {
            effects
                .resulting_accounts
                .iter()
                .find(|(result_key, _)| result_key == key)
                .map(|(_, account)| AccountSharedData::from(account.clone()))
                .unwrap_or_default()
        })
        .collect()
}

pub fn process_instruction_as_one_arg(
    features: VoteProgramFeatures,
    instruction: &Instruction,
    expected_result: Result<(), InstructionError>,
) -> Vec<AccountSharedData> {
    process_instruction_as_one_arg_with_cu_check(
        features,
        instruction,
        expected_result,
        DEFAULT_COMPUTE_UNITS,
    )
}

pub fn process_instruction_as_one_arg_with_cu_check(
    features: VoteProgramFeatures,
    instruction: &Instruction,
    expected_result: Result<(), InstructionError>,
    expected_cus: u64,
) -> Vec<AccountSharedData> {
    let mut pubkeys: std::collections::HashSet<Pubkey> = instruction
        .accounts
        .iter()
        .map(|meta| meta.pubkey)
        .collect();
    pubkeys.insert(sysvar::clock::id());
    pubkeys.insert(sysvar::epoch_schedule::id());
    pubkeys.insert(sysvar::rent::id());
    pubkeys.insert(sysvar::slot_hashes::id());
    let transaction_accounts: Vec<_> = pubkeys
        .iter()
        .map(|pubkey| {
            (
                *pubkey,
                if sysvar::clock::check_id(pubkey) {
                    create_sysvar_account(&Clock::default())
                } else if sysvar::epoch_schedule::check_id(pubkey) {
                    create_sysvar_account(&EpochSchedule::without_warmup())
                } else if sysvar::slot_hashes::check_id(pubkey) {
                    create_sysvar_account(&SlotHashes::default())
                } else if sysvar::rent::check_id(pubkey) {
                    create_sysvar_account(&Rent::free())
                } else if *pubkey == invalid_vote_state_pubkey() {
                    AccountSharedData::from(Account {
                        owner: invalid_vote_state_pubkey(),
                        ..Account::default()
                    })
                } else {
                    AccountSharedData::from(Account {
                        owner: id(),
                        ..Account::default()
                    })
                },
            )
        })
        .collect();
    process_instruction_with_cu_check(
        features,
        &instruction.data,
        transaction_accounts,
        instruction.accounts.clone(),
        expected_result,
        expected_cus,
    )
}

pub fn invalid_vote_state_pubkey() -> Pubkey {
    Pubkey::from_str("BadVote111111111111111111111111111111111111").unwrap()
}

pub fn create_default_rent_account() -> AccountSharedData {
    create_sysvar_account(&Rent::free())
}

pub fn create_default_clock_account() -> AccountSharedData {
    create_sysvar_account(&Clock::default())
}

pub fn create_test_account() -> (Pubkey, AccountSharedData) {
    let rent = Rent::default();
    let vote_pubkey = solana_pubkey::new_rand();
    let node_pubkey = solana_pubkey::new_rand();

    let balance = rent.minimum_balance(VoteStateV4::size_of());
    let account = create_v4_account_with_authorized(
        &node_pubkey,
        &vote_pubkey,
        [0u8; BLS_PUBLIC_KEY_COMPRESSED_SIZE],
        &vote_pubkey,
        0,
        &vote_pubkey,
        0,
        &node_pubkey,
        balance,
    );

    (vote_pubkey, account)
}

/// Create a V4 vote account with `bls_pubkey_compressed: None`, mirroring
/// what an account looks like after `InitializeAccount` (the v1
/// instruction) but before any BLS pubkey has been registered.
pub fn create_test_account_no_bls_key() -> (Pubkey, AccountSharedData) {
    let rent = Rent::default();
    let vote_pubkey = solana_pubkey::new_rand();
    let node_pubkey = solana_pubkey::new_rand();
    let balance = rent.minimum_balance(VoteStateV4::size_of());

    let vote_state = VoteStateV4::new_with_defaults(
        &vote_pubkey,
        &VoteInit {
            node_pubkey,
            authorized_voter: vote_pubkey,
            authorized_withdrawer: vote_pubkey,
            commission: 0,
        },
        &Clock::default(),
    );
    let account = AccountSharedData::new_data_with_space(
        balance,
        &VoteStateVersions::V4(Box::new(vote_state)),
        VoteStateV4::size_of(),
        &id(),
    )
    .unwrap();

    (vote_pubkey, account)
}

/// Create a vote account whose stored representation is `VoteStateV3`
/// (rather than V4). Useful for verifying instructions that require a V4
/// storage layout reject pre-V4 accounts.
pub fn create_test_account_v3() -> (Pubkey, AccountSharedData) {
    let rent = Rent::default();
    let vote_pubkey = solana_pubkey::new_rand();
    let node_pubkey = solana_pubkey::new_rand();
    let balance = rent.minimum_balance(VoteStateV3::size_of());

    let vote_state = VoteStateV3::new(
        &VoteInit {
            node_pubkey,
            authorized_voter: vote_pubkey,
            authorized_withdrawer: vote_pubkey,
            commission: 0,
        },
        &Clock::default(),
    );
    let account = AccountSharedData::new_data_with_space(
        balance,
        &VoteStateVersions::V3(Box::new(vote_state)),
        VoteStateV3::size_of(),
        &id(),
    )
    .unwrap();

    (vote_pubkey, account)
}

pub fn create_test_account_with_authorized() -> (Pubkey, Pubkey, Pubkey, AccountSharedData) {
    let vote_pubkey = solana_pubkey::new_rand();
    let authorized_voter = solana_pubkey::new_rand();
    let authorized_withdrawer = solana_pubkey::new_rand();
    let account =
        create_test_account_with_provided_authorized(&authorized_voter, &authorized_withdrawer);

    (
        vote_pubkey,
        authorized_voter,
        authorized_withdrawer,
        account,
    )
}

pub fn create_test_account_with_provided_authorized(
    authorized_voter: &Pubkey,
    authorized_withdrawer: &Pubkey,
) -> AccountSharedData {
    let node_pubkey = solana_pubkey::new_rand();

    create_v4_account_with_authorized(
        &node_pubkey,
        authorized_voter,
        [0u8; BLS_PUBLIC_KEY_COMPRESSED_SIZE],
        authorized_withdrawer,
        0,
        authorized_withdrawer,
        0,
        &node_pubkey,
        100,
    )
}

pub fn create_test_account_with_authorized_from_seed() -> VoteAccountTestFixtureWithAuthorities {
    let vote_pubkey = Pubkey::new_unique();
    let voter_base_key = Pubkey::new_unique();
    let voter_owner = Pubkey::new_unique();
    let voter_seed = String::from("VOTER_SEED");
    let withdrawer_base_key = Pubkey::new_unique();
    let withdrawer_owner = Pubkey::new_unique();
    let withdrawer_seed = String::from("WITHDRAWER_SEED");
    let authorized_voter =
        Pubkey::create_with_seed(&voter_base_key, voter_seed.as_str(), &voter_owner).unwrap();
    let authorized_withdrawer = Pubkey::create_with_seed(
        &withdrawer_base_key,
        withdrawer_seed.as_str(),
        &withdrawer_owner,
    )
    .unwrap();

    let vote_state = VoteStateV4::new_with_defaults(
        &vote_pubkey,
        &VoteInit {
            node_pubkey: Pubkey::new_unique(),
            authorized_voter,
            authorized_withdrawer,
            commission: 0,
        },
        &Clock::default(),
    );
    let vote_account = AccountSharedData::new_data_with_space(
        100,
        &VoteStateVersions::V4(Box::new(vote_state)),
        VoteStateV4::size_of(),
        &id(),
    )
    .unwrap();

    VoteAccountTestFixtureWithAuthorities {
        vote_account,
        vote_pubkey,
        voter_base_key,
        voter_owner,
        voter_seed,
        withdrawer_base_key,
        withdrawer_owner,
        withdrawer_seed,
    }
}

pub fn create_test_account_with_epoch_credits(
    credits_to_append: &[u64],
) -> (Pubkey, AccountSharedData) {
    let vote_pubkey = solana_pubkey::new_rand();
    let node_pubkey = solana_pubkey::new_rand();

    let vote_init = VoteInit {
        node_pubkey,
        authorized_voter: vote_pubkey,
        authorized_withdrawer: vote_pubkey,
        commission: 0,
    };
    let clock = Clock::default();

    let space = vote_state_size_of();
    let lamports = Rent::default().minimum_balance(space);

    let v4 = VoteStateV4::new_with_defaults(&vote_pubkey, &vote_init, &clock);
    let mut vote_state = VoteStateHandler::new_v4(v4);

    let epoch_credits = vote_state.epoch_credits_mut();
    epoch_credits.clear();

    let mut current_epoch_credits: u64 = 0;
    let mut previous_epoch_credits = 0;
    for (epoch, credits) in credits_to_append.iter().enumerate() {
        current_epoch_credits = current_epoch_credits.saturating_add(*credits);
        epoch_credits.push((
            u64::try_from(epoch).unwrap(),
            current_epoch_credits,
            previous_epoch_credits,
        ));
        previous_epoch_credits = current_epoch_credits;
    }

    let account = AccountSharedData::new_with_data(
        lamports,
        vote_state.serialize_padded(VoteStateV4::size_of()).unwrap(),
        &id(),
    );

    (vote_pubkey, account)
}

/// Ensure the sysvar accounts the harness requires and the builtin program
/// account are present, without overriding any the test supplied explicitly.
fn add_required_accounts(accounts: &mut Vec<(Pubkey, Account)>, program_id: &Pubkey) {
    let required = [
        Some((
            sysvar::clock::id(),
            Account::from(create_sysvar_account(&Clock::default())),
        )),
        Some((
            sysvar::rent::id(),
            Account::from(create_sysvar_account(&Rent::free())),
        )),
        keyed_account_for_builtin_pubkey(program_id),
    ];
    for (address, account) in required.into_iter().flatten() {
        if !accounts.iter().any(|(key, _)| *key == address) {
            accounts.push((address, account));
        }
    }
}
