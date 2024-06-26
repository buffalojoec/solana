mod calculation;
mod compare;
mod distribution;
mod epoch_rewards_hasher;
mod sysvar;

use {
    super::Bank,
    crate::{stake_account::StakeAccount, stake_history::StakeHistory},
    solana_accounts_db::{
        partitioned_rewards::PartitionedEpochRewardsConfig, stake_rewards::StakeReward,
    },
    solana_sdk::{
        account::AccountSharedData, clock::Slot, feature_set, pubkey::Pubkey,
        reward_info::RewardInfo, stake::state::Delegation,
    },
    solana_vote::vote_account::VoteAccounts,
    std::sync::Arc,
};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub(super) enum RewardInterval {
    /// the slot within the epoch is INSIDE the reward distribution interval
    InsideInterval,
    /// the slot within the epoch is OUTSIDE the reward distribution interval
    OutsideInterval,
}

#[derive(AbiExample, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StartBlockHeightAndRewards {
    /// the block height of the slot at which rewards distribution began
    pub(crate) start_block_height: u64,
    /// calculated epoch rewards pending distribution, outer Vec is by partition (one partition per block)
    pub(crate) stake_rewards_by_partition: Arc<Vec<StakeRewards>>,
}

/// Represent whether bank is in the reward phase or not.
#[derive(AbiExample, AbiEnumVisitor, Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub(crate) enum EpochRewardStatus {
    /// this bank is in the reward phase.
    /// Contents are the start point for epoch reward calculation,
    /// i.e. parent_slot and parent_block height for the starting
    /// block of the current epoch.
    Active(StartBlockHeightAndRewards),
    /// this bank is outside of the rewarding phase.
    #[default]
    Inactive,
}

#[derive(Debug, Default)]
pub(super) struct VoteRewardsAccounts {
    /// reward info for each vote account pubkey.
    /// This type is used by `update_reward_history()`
    pub(super) rewards: Vec<(Pubkey, RewardInfo)>,
    /// corresponds to pubkey in `rewards`
    /// Some if account is to be stored.
    /// None if to be skipped.
    pub(super) accounts_to_store: Vec<Option<AccountSharedData>>,
}

/// hold reward calc info to avoid recalculation across functions
pub(super) struct EpochRewardCalculateParamInfo<'a> {
    pub(super) stake_history: StakeHistory,
    pub(super) stake_delegations: Vec<(&'a Pubkey, &'a StakeAccount<Delegation>)>,
    pub(super) cached_vote_accounts: &'a VoteAccounts,
}

/// Hold all results from calculating the rewards for partitioned distribution.
/// This struct exists so we can have a function which does all the calculation with no
/// side effects.
pub(super) struct PartitionedRewardsCalculation {
    pub(super) vote_account_rewards: VoteRewardsAccounts,
    pub(super) stake_rewards_by_partition: StakeRewardCalculationPartitioned,
    pub(super) old_vote_balance_and_staked: u64,
    pub(super) validator_rewards: u64,
    pub(super) validator_rate: f64,
    pub(super) foundation_rate: f64,
    pub(super) prev_epoch_duration_in_years: f64,
    pub(super) capitalization: u64,
}

/// result of calculating the stake rewards at beginning of new epoch
pub(super) struct StakeRewardCalculationPartitioned {
    /// each individual stake account to reward, grouped by partition
    pub(super) stake_rewards_by_partition: Vec<StakeRewards>,
    /// total lamports across all `stake_rewards`
    pub(super) total_stake_rewards_lamports: u64,
}

pub(super) struct CalculateRewardsAndDistributeVoteRewardsResult {
    /// total rewards for the epoch (including both vote rewards and stake rewards)
    pub(super) total_rewards: u64,
    /// distributed vote rewards
    pub(super) distributed_rewards: u64,
    /// stake rewards that still need to be distributed, grouped by partition
    pub(super) stake_rewards_by_partition: Vec<StakeRewards>,
}

pub(crate) type StakeRewards = Vec<StakeReward>;

impl Bank {
    pub(super) fn is_partitioned_rewards_feature_enabled(&self) -> bool {
        self.feature_set
            .is_active(&feature_set::enable_partitioned_epoch_reward::id())
    }

    pub(crate) fn set_epoch_reward_status_active(
        &mut self,
        stake_rewards_by_partition: Vec<StakeRewards>,
    ) {
        self.epoch_reward_status = EpochRewardStatus::Active(StartBlockHeightAndRewards {
            start_block_height: self.block_height,
            stake_rewards_by_partition: Arc::new(stake_rewards_by_partition),
        });
    }

    pub(super) fn partitioned_epoch_rewards_config(&self) -> &PartitionedEpochRewardsConfig {
        &self
            .rc
            .accounts
            .accounts_db
            .partitioned_epoch_rewards_config
    }

    /// # stake accounts to store in one block during partitioned reward interval
    pub(super) fn partitioned_rewards_stake_account_stores_per_block(&self) -> u64 {
        self.partitioned_epoch_rewards_config()
            .stake_account_stores_per_block
    }

    /// reward calculation happens synchronously during the first block of the epoch boundary.
    /// So, # blocks for reward calculation is 1.
    pub(super) fn get_reward_calculation_num_blocks(&self) -> Slot {
        self.partitioned_epoch_rewards_config()
            .reward_calculation_num_blocks
    }

    /// Calculate the number of blocks required to distribute rewards to all stake accounts.
    pub(super) fn get_reward_distribution_num_blocks(&self, rewards: &StakeRewards) -> u64 {
        let total_stake_accounts = rewards.len();
        if self.epoch_schedule.warmup && self.epoch < self.first_normal_epoch() {
            1
        } else {
            const MAX_FACTOR_OF_REWARD_BLOCKS_IN_EPOCH: u64 = 10;
            let num_chunks = solana_accounts_db::accounts_hash::AccountsHasher::div_ceil(
                total_stake_accounts,
                self.partitioned_rewards_stake_account_stores_per_block() as usize,
            ) as u64;

            // Limit the reward credit interval to 10% of the total number of slots in a epoch
            num_chunks.clamp(
                1,
                (self.epoch_schedule.slots_per_epoch / MAX_FACTOR_OF_REWARD_BLOCKS_IN_EPOCH).max(1),
            )
        }
    }

    /// Return `RewardInterval` enum for current bank
    pub(super) fn get_reward_interval(&self) -> RewardInterval {
        if matches!(self.epoch_reward_status, EpochRewardStatus::Active(_)) {
            RewardInterval::InsideInterval
        } else {
            RewardInterval::OutsideInterval
        }
    }

    /// true if it is ok to run partitioned rewards code.
    /// This means the feature is activated or certain testing situations.
    pub(super) fn is_partitioned_rewards_code_enabled(&self) -> bool {
        self.is_partitioned_rewards_feature_enabled()
            || self
                .partitioned_epoch_rewards_config()
                .test_enable_partitioned_rewards
    }

    /// For testing only
    pub fn force_reward_interval_end_for_tests(&mut self) {
        self.epoch_reward_status = EpochRewardStatus::Inactive;
    }

    pub(super) fn force_partition_rewards_in_first_block_of_epoch(&self) -> bool {
        self.partitioned_epoch_rewards_config()
            .test_enable_partitioned_rewards
            && self.get_reward_calculation_num_blocks() == 0
            && self.partitioned_rewards_stake_account_stores_per_block() == u64::MAX
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            bank::tests::{create_genesis_config, new_bank_from_parent_with_bank_forks},
            genesis_utils::{
                create_genesis_config_with_vote_accounts, GenesisConfigInfo, ValidatorVoteKeypairs,
            },
        },
        assert_matches::assert_matches,
        solana_accounts_db::{
            accounts_db::{
                AccountShrinkThreshold, AccountsDbConfig, ACCOUNTS_DB_CONFIG_FOR_TESTING,
            },
            accounts_index::AccountSecondaryIndexes,
            partitioned_rewards::TestPartitionedEpochRewards,
        },
        solana_program_runtime::runtime_config::RuntimeConfig,
        solana_sdk::{
            epoch_schedule::EpochSchedule,
            native_token::LAMPORTS_PER_SOL,
            signature::Signer,
            system_transaction,
            vote::state::{VoteStateVersions, MAX_LOCKOUT_HISTORY},
        },
        solana_vote_program::{vote_state, vote_transaction},
    };

    impl Bank {
        /// Return the total number of blocks in reward interval (including both calculation and crediting).
        pub(in crate::bank) fn get_reward_total_num_blocks(&self, rewards: &StakeRewards) -> u64 {
            self.get_reward_calculation_num_blocks()
                + self.get_reward_distribution_num_blocks(rewards)
        }
    }

    #[test]
    fn test_force_reward_interval_end() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let mut bank = Bank::new_for_tests(&genesis_config);

        let expected_num = 100;

        let stake_rewards = (0..expected_num)
            .map(|_| StakeReward::new_random())
            .collect::<Vec<_>>();

        bank.set_epoch_reward_status_active(vec![stake_rewards]);
        assert!(bank.get_reward_interval() == RewardInterval::InsideInterval);

        bank.force_reward_interval_end_for_tests();
        assert!(bank.get_reward_interval() == RewardInterval::OutsideInterval);
    }

    #[test]
    fn test_is_partitioned_reward_feature_enable() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);

        let mut bank = Bank::new_for_tests(&genesis_config);
        assert!(!bank.is_partitioned_rewards_feature_enabled());
        bank.activate_feature(&feature_set::enable_partitioned_epoch_reward::id());
        assert!(bank.is_partitioned_rewards_feature_enabled());
    }

    /// Test get_reward_distribution_num_blocks, get_reward_calculation_num_blocks, get_reward_total_num_blocks during small epoch
    /// The num_credit_blocks should be cap to 10% of the total number of blocks in the epoch.
    #[test]
    fn test_get_reward_distribution_num_blocks_cap() {
        let (mut genesis_config, _mint_keypair) =
            create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        genesis_config.epoch_schedule = EpochSchedule::custom(32, 32, false);

        // Config stake reward distribution to be 10 per block
        let mut accounts_db_config: AccountsDbConfig = ACCOUNTS_DB_CONFIG_FOR_TESTING.clone();
        accounts_db_config.test_partitioned_epoch_rewards =
            TestPartitionedEpochRewards::PartitionedEpochRewardsConfigRewardBlocks {
                reward_calculation_num_blocks: 1,
                stake_account_stores_per_block: 10,
            };

        let bank = Bank::new_with_paths(
            &genesis_config,
            Arc::new(RuntimeConfig::default()),
            Vec::new(),
            None,
            None,
            AccountSecondaryIndexes::default(),
            AccountShrinkThreshold::default(),
            false,
            Some(accounts_db_config),
            None,
            Some(Pubkey::new_unique()),
            Arc::default(),
        );

        let stake_account_stores_per_block =
            bank.partitioned_rewards_stake_account_stores_per_block();
        assert_eq!(stake_account_stores_per_block, 10);

        let check_num_reward_distribution_blocks =
            |num_stakes: u64,
             expected_num_reward_distribution_blocks: u64,
             expected_num_reward_computation_blocks: u64| {
                // Given the short epoch, i.e. 32 slots, we should cap the number of reward distribution blocks to 32/10 = 3.
                let stake_rewards = (0..num_stakes)
                    .map(|_| StakeReward::new_random())
                    .collect::<Vec<_>>();

                assert_eq!(
                    bank.get_reward_distribution_num_blocks(&stake_rewards),
                    expected_num_reward_distribution_blocks
                );
                assert_eq!(
                    bank.get_reward_calculation_num_blocks(),
                    expected_num_reward_computation_blocks
                );
                assert_eq!(
                    bank.get_reward_total_num_blocks(&stake_rewards),
                    bank.get_reward_distribution_num_blocks(&stake_rewards)
                        + bank.get_reward_calculation_num_blocks(),
                );
            };

        for test_record in [
            // num_stakes, expected_num_reward_distribution_blocks, expected_num_reward_computation_blocks
            (0, 1, 1),
            (1, 1, 1),
            (stake_account_stores_per_block, 1, 1),
            (2 * stake_account_stores_per_block - 1, 2, 1),
            (2 * stake_account_stores_per_block, 2, 1),
            (3 * stake_account_stores_per_block - 1, 3, 1),
            (3 * stake_account_stores_per_block, 3, 1),
            (4 * stake_account_stores_per_block, 3, 1), // cap at 3
            (5 * stake_account_stores_per_block, 3, 1), //cap at 3
        ] {
            check_num_reward_distribution_blocks(test_record.0, test_record.1, test_record.2);
        }
    }

    /// Test get_reward_distribution_num_blocks, get_reward_calculation_num_blocks, get_reward_total_num_blocks during normal epoch gives the expected result
    #[test]
    fn test_get_reward_distribution_num_blocks_normal() {
        solana_logger::setup();
        let (mut genesis_config, _mint_keypair) =
            create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        genesis_config.epoch_schedule = EpochSchedule::custom(432000, 432000, false);

        let bank = Bank::new_for_tests(&genesis_config);

        // Given 8k rewards, it will take 2 blocks to credit all the rewards
        let expected_num = 8192;
        let stake_rewards = (0..expected_num)
            .map(|_| StakeReward::new_random())
            .collect::<Vec<_>>();

        assert_eq!(bank.get_reward_distribution_num_blocks(&stake_rewards), 2);
        assert_eq!(bank.get_reward_calculation_num_blocks(), 1);
        assert_eq!(
            bank.get_reward_total_num_blocks(&stake_rewards),
            bank.get_reward_distribution_num_blocks(&stake_rewards)
                + bank.get_reward_calculation_num_blocks(),
        );
    }

    /// Test get_reward_distribution_num_blocks, get_reward_calculation_num_blocks, get_reward_total_num_blocks during warm up epoch gives the expected result.
    /// The num_credit_blocks should be 1 during warm up epoch.
    #[test]
    fn test_get_reward_distribution_num_blocks_warmup() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);

        let bank = Bank::new_for_tests(&genesis_config);
        let rewards = vec![];
        assert_eq!(bank.get_reward_distribution_num_blocks(&rewards), 1);
        assert_eq!(bank.get_reward_calculation_num_blocks(), 1);
        assert_eq!(
            bank.get_reward_total_num_blocks(&rewards),
            bank.get_reward_distribution_num_blocks(&rewards)
                + bank.get_reward_calculation_num_blocks(),
        );
    }

    #[test]
    fn test_rewards_computation_and_partitioned_distribution_one_block() {
        solana_logger::setup();

        // setup the expected number of stake delegations
        let expected_num_delegations = 100;

        let validator_keypairs = (0..expected_num_delegations)
            .map(|_| ValidatorVoteKeypairs::new_rand())
            .collect::<Vec<_>>();

        let GenesisConfigInfo { genesis_config, .. } = create_genesis_config_with_vote_accounts(
            1_000_000_000,
            &validator_keypairs,
            vec![2_000_000_000; expected_num_delegations],
        );

        let bank0 = Bank::new_for_tests(&genesis_config);
        let num_slots_in_epoch = bank0.get_slots_in_epoch(bank0.epoch());
        assert_eq!(num_slots_in_epoch, 32);

        let mut previous_bank = Arc::new(Bank::new_from_parent(
            Arc::new(bank0),
            &Pubkey::default(),
            1,
        ));

        // simulate block progress
        for slot in 2..=num_slots_in_epoch + 2 {
            let pre_cap = previous_bank.capitalization();
            let curr_bank = Bank::new_from_parent(previous_bank, &Pubkey::default(), slot);
            let post_cap = curr_bank.capitalization();

            // Fill banks with banks with votes landing in the next slot
            // Create enough banks such that vote account will root
            for validator_vote_keypairs in validator_keypairs.iter() {
                let vote_id = validator_vote_keypairs.vote_keypair.pubkey();
                let mut vote_account = curr_bank.get_account(&vote_id).unwrap();
                // generate some rewards
                let mut vote_state = Some(vote_state::from(&vote_account).unwrap());
                for i in 0..MAX_LOCKOUT_HISTORY + 42 {
                    if let Some(v) = vote_state.as_mut() {
                        vote_state::process_slot_vote_unchecked(v, i as u64)
                    }
                    let versioned =
                        VoteStateVersions::Current(Box::new(vote_state.take().unwrap()));
                    vote_state::to(&versioned, &mut vote_account).unwrap();
                    match versioned {
                        VoteStateVersions::Current(v) => {
                            vote_state = Some(*v);
                        }
                        _ => panic!("Has to be of type Current"),
                    };
                }
                curr_bank.store_account_and_update_capitalization(&vote_id, &vote_account);
            }

            if slot == num_slots_in_epoch {
                // This is the first block of epoch 1. Reward computation should happen in this block.
                // assert reward compute status activated at epoch boundary
                assert_matches!(
                    curr_bank.get_reward_interval(),
                    RewardInterval::InsideInterval
                );

                // cap should increase because of new epoch rewards
                assert!(post_cap > pre_cap);
            } else if slot == num_slots_in_epoch + 1 || slot == num_slots_in_epoch + 2 {
                // 1. when curr_slot == num_slots_in_epoch + 1, the 2nd block of epoch 1, reward distribution should happen in this block.
                // however, all stake rewards are paid at the this block therefore reward_status should have transitioned to inactive. And since
                // rewards are transferred from epoch_rewards sysvar to stake accounts. The cap should stay the same.
                // 2. when curr_slot == num_slots_in_epoch+2, the 3rd block of epoch 1. reward distribution should have already completed. Therefore,
                // reward_status should stay inactive and cap should stay the same.
                assert_matches!(
                    curr_bank.get_reward_interval(),
                    RewardInterval::OutsideInterval
                );

                assert_eq!(post_cap, pre_cap);
            } else {
                // slot is not in rewards, cap should not change
                assert_eq!(post_cap, pre_cap);
            }
            previous_bank = Arc::new(curr_bank);
        }
    }

    /// Test rewards computation and partitioned rewards distribution at the epoch boundary (two reward distribution blocks)
    #[test]
    fn test_rewards_computation_and_partitioned_distribution_two_blocks() {
        solana_logger::setup();

        // Set up the expected number of stake delegations 100
        let expected_num_delegations = 100;

        let validator_keypairs = (0..expected_num_delegations)
            .map(|_| ValidatorVoteKeypairs::new_rand())
            .collect::<Vec<_>>();

        let GenesisConfigInfo {
            mut genesis_config, ..
        } = create_genesis_config_with_vote_accounts(
            1_000_000_000,
            &validator_keypairs,
            vec![2_000_000_000; expected_num_delegations],
        );
        genesis_config.epoch_schedule = EpochSchedule::custom(32, 32, false);

        // Config stake reward distribution to be 50 per block
        // We will need two blocks for reward distribution. And we can assert that the expected bank
        // capital changes before/during/after reward distribution.
        let mut accounts_db_config: AccountsDbConfig = ACCOUNTS_DB_CONFIG_FOR_TESTING.clone();
        accounts_db_config.test_partitioned_epoch_rewards =
            TestPartitionedEpochRewards::PartitionedEpochRewardsConfigRewardBlocks {
                reward_calculation_num_blocks: 1,
                stake_account_stores_per_block: 50,
            };

        let bank0 = Bank::new_with_paths(
            &genesis_config,
            Arc::new(RuntimeConfig::default()),
            Vec::new(),
            None,
            None,
            AccountSecondaryIndexes::default(),
            AccountShrinkThreshold::default(),
            false,
            Some(accounts_db_config),
            None,
            None,
            Arc::default(),
        );

        let num_slots_in_epoch = bank0.get_slots_in_epoch(bank0.epoch());
        assert_eq!(num_slots_in_epoch, 32);

        let mut previous_bank = Arc::new(Bank::new_from_parent(
            Arc::new(bank0),
            &Pubkey::default(),
            1,
        ));

        // simulate block progress
        for slot in 2..=num_slots_in_epoch + 3 {
            let pre_cap = previous_bank.capitalization();
            let curr_bank = Bank::new_from_parent(previous_bank, &Pubkey::default(), slot);
            let post_cap = curr_bank.capitalization();

            // Fill banks with banks with votes landing in the next slot
            // Create enough banks such that vote account will root
            for validator_vote_keypairs in validator_keypairs.iter() {
                let vote_id = validator_vote_keypairs.vote_keypair.pubkey();
                let mut vote_account = curr_bank.get_account(&vote_id).unwrap();
                // generate some rewards
                let mut vote_state = Some(vote_state::from(&vote_account).unwrap());
                for i in 0..MAX_LOCKOUT_HISTORY + 42 {
                    if let Some(v) = vote_state.as_mut() {
                        vote_state::process_slot_vote_unchecked(v, i as u64)
                    }
                    let versioned =
                        VoteStateVersions::Current(Box::new(vote_state.take().unwrap()));
                    vote_state::to(&versioned, &mut vote_account).unwrap();
                    match versioned {
                        VoteStateVersions::Current(v) => {
                            vote_state = Some(*v);
                        }
                        _ => panic!("Has to be of type Current"),
                    };
                }
                curr_bank.store_account_and_update_capitalization(&vote_id, &vote_account);
            }

            if slot == num_slots_in_epoch {
                // This is the first block of epoch 1. Reward computation should happen in this block.
                // assert reward compute status activated at epoch boundary
                assert_matches!(
                    curr_bank.get_reward_interval(),
                    RewardInterval::InsideInterval
                );

                // cap should increase because of new epoch rewards
                assert!(post_cap > pre_cap);
            } else if slot == num_slots_in_epoch + 1 {
                // When curr_slot == num_slots_in_epoch + 1, the 2nd block of epoch 1, reward distribution should happen in this block.
                // however, since rewards are transferred from epoch_rewards sysvar to stake accounts. The cap should stay the same.
                assert_matches!(
                    curr_bank.get_reward_interval(),
                    RewardInterval::InsideInterval
                );

                assert_eq!(post_cap, pre_cap);
            } else if slot == num_slots_in_epoch + 2 || slot == num_slots_in_epoch + 3 {
                // 1. when curr_slot == num_slots_in_epoch + 2, the 3nd block of epoch 1, reward distribution should happen in this block.
                // however, all stake rewards are paid at the this block therefore reward_status should have transitioned to inactive. And since
                // rewards are transferred from epoch_rewards sysvar to stake accounts. The cap should stay the same.
                // 2. when curr_slot == num_slots_in_epoch+2, the 3rd block of epoch 1. reward distribution should have already completed. Therefore,
                // reward_status should stay inactive and cap should stay the same.
                assert_matches!(
                    curr_bank.get_reward_interval(),
                    RewardInterval::OutsideInterval
                );

                assert_eq!(post_cap, pre_cap);
            } else {
                // slot is not in rewards, cap should not change
                assert_eq!(post_cap, pre_cap);
            }
            previous_bank = Arc::new(curr_bank);
        }
    }

    /// Test that program execution that involves stake accounts should fail during reward period.
    /// Any programs, which result in stake account changes, will throw `ProgramExecutionTemporarilyRestricted` error when
    /// in reward period.
    #[test]
    fn test_program_execution_restricted_for_stake_account_in_reward_period() {
        use solana_sdk::transaction::TransactionError::ProgramExecutionTemporarilyRestricted;

        let validator_vote_keypairs = ValidatorVoteKeypairs::new_rand();
        let validator_keypairs = vec![&validator_vote_keypairs];
        let GenesisConfigInfo { genesis_config, .. } = create_genesis_config_with_vote_accounts(
            1_000_000_000,
            &validator_keypairs,
            vec![1_000_000_000; 1],
        );

        let node_key = &validator_keypairs[0].node_keypair;
        let stake_key = &validator_keypairs[0].stake_keypair;

        let (mut previous_bank, bank_forks) = Bank::new_with_bank_forks_for_tests(&genesis_config);
        let num_slots_in_epoch = previous_bank.get_slots_in_epoch(previous_bank.epoch());
        assert_eq!(num_slots_in_epoch, 32);

        for slot in 1..=num_slots_in_epoch + 2 {
            let bank = new_bank_from_parent_with_bank_forks(
                bank_forks.as_ref(),
                previous_bank.clone(),
                &Pubkey::default(),
                slot,
            );

            // Fill bank_forks with banks with votes landing in the next slot
            // So that rewards will be paid out at the epoch boundary, i.e. slot = 32
            let vote = vote_transaction::new_vote_transaction(
                vec![slot - 1],
                previous_bank.hash(),
                previous_bank.last_blockhash(),
                &validator_vote_keypairs.node_keypair,
                &validator_vote_keypairs.vote_keypair,
                &validator_vote_keypairs.vote_keypair,
                None,
            );
            bank.process_transaction(&vote).unwrap();

            // Insert a transfer transaction from node account to stake account
            let tx = system_transaction::transfer(
                node_key,
                &stake_key.pubkey(),
                1,
                bank.last_blockhash(),
            );
            let r = bank.process_transaction(&tx);

            if slot == num_slots_in_epoch {
                // When the bank is at the beginning of the new epoch, i.e. slot 32,
                // ProgramExecutionTemporarilyRestricted should be thrown for the transfer transaction.
                assert_eq!(
                    r,
                    Err(ProgramExecutionTemporarilyRestricted { account_index: 1 })
                );
            } else {
                // When the bank is outside of reward interval, the transfer transaction should not be affected and will succeed.
                assert!(r.is_ok());
            }

            // Push a dummy blockhash, so that the latest_blockhash() for the transfer transaction in each
            // iteration are different. Otherwise, all those transactions will be the same, and will not be
            // executed by the bank except the first one.
            bank.register_unique_recent_blockhash_for_test();
            previous_bank = bank;
        }
    }
}
