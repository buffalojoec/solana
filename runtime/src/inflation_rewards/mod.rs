//! Information about stake and voter rewards based on stake state.
use {
    self::points::{
        CalculatedStakePoints, CalculationEnvironment, DelegatedVoteState,
        InflationPointCalculationEvent, SkippedReason, calculate_stake_points_and_credits,
    },
    crate::{
        alpenglow_epoch_type::AlpenglowEpochType,
        bank::commission::{commission_split, commission_split_preserve_lamports},
        stake_delegation::effective_stake,
    },
    solana_instruction::error::InstructionError,
    solana_stake_interface::{error::StakeError, state::Stake},
};

pub mod points;

#[derive(Debug, PartialEq, Eq)]
struct CalculatedStakeRewards {
    staker_rewards: u64,
    voter_rewards: u64,
    new_credits_observed: u64,
}

/// Redeems rewards for the given epoch, stake state and vote state.
/// Returns a tuple of:
/// * Stakers reward
/// * Voters reward
/// * Updated stake information
#[allow(clippy::too_many_arguments)]
pub(crate) fn redeem_rewards<'a>(
    mut stake: Stake,
    voter_commission_bps: u16,
    vote_state: DelegatedVoteState,
    calculation_environment: CalculationEnvironment<'a>,
    inflation_point_calc_tracer: Option<impl Fn(&InflationPointCalculationEvent)>,
    ag_epoch_type: &AlpenglowEpochType,
    current_lamports: u64,
    minimum_lamports: u64,
) -> Result<(u64, u64, Stake), InstructionError> {
    if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
        let CalculationEnvironment {
            rewarded_epoch,
            stake_history,
            new_rate_activation_epoch,
            commission_rate_in_basis_points,
            use_fixed_point_stake_math,
            ..
        } = calculation_environment;
        let effective_stake_at_rewarded_epoch = effective_stake(
            &stake,
            rewarded_epoch,
            stake_history,
            new_rate_activation_epoch,
            use_fixed_point_stake_math,
        );
        inflation_point_calc_tracer(
            &InflationPointCalculationEvent::EffectiveStakeAtRewardedEpoch(
                effective_stake_at_rewarded_epoch,
            ),
        );
        inflation_point_calc_tracer(&InflationPointCalculationEvent::PriorTotalLamports(
            current_lamports,
        ));
        // Choose which trace to emit based on the `commission_rate_in_basis_points` feature.
        if commission_rate_in_basis_points {
            inflation_point_calc_tracer(&InflationPointCalculationEvent::CommissionBps(
                voter_commission_bps,
            ));
        } else {
            inflation_point_calc_tracer(&InflationPointCalculationEvent::Commission(
                (voter_commission_bps / 100) as u8,
            ));
        }
    }

    if let Some((stakers_reward, voters_reward)) = redeem_stake_rewards(
        &mut stake,
        voter_commission_bps,
        vote_state,
        calculation_environment,
        inflation_point_calc_tracer,
        ag_epoch_type,
        current_lamports,
        minimum_lamports,
    ) {
        Ok((stakers_reward, voters_reward, stake))
    } else {
        Err(StakeError::NoCreditsToRedeem.into())
    }
}

fn redeem_stake_rewards<'a>(
    stake: &mut Stake,
    voter_commission_bps: u16,
    vote_state: DelegatedVoteState,
    calculation_environment: CalculationEnvironment<'a>,
    inflation_point_calc_tracer: Option<impl Fn(&InflationPointCalculationEvent)>,
    ag_epoch_type: &AlpenglowEpochType,
    current_lamports: u64,
    minimum_lamports: u64,
) -> Option<(u64, u64)> {
    if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
        inflation_point_calc_tracer(&InflationPointCalculationEvent::CreditsObserved(
            stake.credits_observed,
            None,
        ));
    }

    let adjust_delegations_for_rent = calculation_environment.adjust_delegations_for_rent;
    let maybe_rewards = calculate_stake_rewards(
        stake,
        voter_commission_bps,
        vote_state,
        calculation_environment,
        inflation_point_calc_tracer.as_ref(),
        ag_epoch_type,
    )
    .map(|calculated_stake_rewards| {
        if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer {
            inflation_point_calc_tracer(&InflationPointCalculationEvent::CreditsObserved(
                stake.credits_observed,
                Some(calculated_stake_rewards.new_credits_observed),
            ));
        }
        stake.credits_observed = calculated_stake_rewards.new_credits_observed;
        (
            calculated_stake_rewards.staker_rewards,
            calculated_stake_rewards.voter_rewards,
        )
    });

    let staker_rewards = maybe_rewards.map(|x| x.0).unwrap_or(0);
    if adjust_delegations_for_rent {
        let new_delegation_with_rewards = stake.delegation.stake.saturating_add(staker_rewards);
        let needs_adjustment = delegation_may_need_adjustment(
            stake.delegation.stake,
            new_delegation_with_rewards,
            current_lamports.saturating_add(staker_rewards),
            minimum_lamports,
        );
        // If `maybe_rewards.is_some()`, need to drive forward credits, even
        // if rewards are zero
        if needs_adjustment || maybe_rewards.is_some() {
            stake.delegation.stake = new_delegation_with_rewards;
            let voter_rewards = maybe_rewards.map(|x| x.1).unwrap_or(0);
            Some((staker_rewards, voter_rewards))
        } else {
            None
        }
    } else {
        stake.delegation.stake += staker_rewards;
        maybe_rewards
    }
}

/// Returns `true` if stake delegation needs to be adjusted during distribution
/// based on Rent sysvar parameters at epoch boundary
///
/// The actual adjustment happens at distribution, to account for any lamports
/// credited to the account during partitioned epoch rewards, before the
/// distribution has occurred.
pub(crate) fn delegation_may_need_adjustment(
    current_delegation: u64,
    new_delegation_with_rewards: u64,
    lamports_with_rewards: u64,
    minimum_lamports: u64,
) -> bool {
    let new_delegation = std::cmp::min(
        new_delegation_with_rewards,
        lamports_with_rewards.saturating_sub(minimum_lamports),
    );

    new_delegation != current_delegation
}

/// for a given stake and vote_state, calculate what distributions and what updates should be made
/// returns a tuple in the case of a payout of:
///   * staker_rewards to be distributed
///   * voter_rewards to be distributed
///   * new value for credits_observed in the stake
///
/// returns None if there's no payout or if any deserved payout is < 1 lamport
fn calculate_stake_rewards<'a>(
    stake: &Stake,
    voter_commission_bps: u16,
    vote_state: DelegatedVoteState,
    calculation_environment: CalculationEnvironment<'a>,
    inflation_point_calc_tracer: Option<impl Fn(&InflationPointCalculationEvent)>,
    ag_epoch_type: &AlpenglowEpochType,
) -> Option<CalculatedStakeRewards> {
    let CalculationEnvironment {
        stake_history,
        new_rate_activation_epoch,
        point_value,
        rewarded_epoch,
        use_fixed_point_stake_math,
        ..
    } = calculation_environment;

    // ensure to run to trigger (optional) inflation_point_calc_tracer
    let CalculatedStakePoints {
        tower_points,
        ag_points,
        new_credits_observed,
        mut force_credits_update_with_skipped_reward,
    } = calculate_stake_points_and_credits(
        stake,
        vote_state,
        stake_history,
        inflation_point_calc_tracer.as_ref(),
        new_rate_activation_epoch,
        ag_epoch_type,
        use_fixed_point_stake_math,
    );

    // Drive credits_observed forward unconditionally when rewards are disabled
    // or when this is the stake's activation epoch
    if point_value.rewards == 0 {
        if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
            inflation_point_calc_tracer(&SkippedReason::DisabledInflation.into());
        }
        force_credits_update_with_skipped_reward = true;
    } else if stake.delegation.activation_epoch == rewarded_epoch {
        // not assert!()-ed; but points should be zero
        if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
            inflation_point_calc_tracer(&SkippedReason::JustActivated.into());
        }
        force_credits_update_with_skipped_reward = true;
    }

    // Once alpenglow is active we no longer allow for epochs where rewards are not redeemed.
    let is_tower_epoch = matches!(ag_epoch_type, AlpenglowEpochType::Tower);
    let advance_credits_for_skipped_reward =
        !is_tower_epoch && new_credits_observed != stake.credits_observed;
    let skipped_reward = || {
        Some(CalculatedStakeRewards {
            staker_rewards: 0,
            voter_rewards: 0,
            new_credits_observed,
        })
    };

    let skip_reward = |reason: SkippedReason| {
        if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
            inflation_point_calc_tracer(&reason.into());
        }
        if advance_credits_for_skipped_reward {
            skipped_reward()
        } else {
            None
        }
    };

    if force_credits_update_with_skipped_reward {
        return skipped_reward();
    }

    let rewards = match ag_epoch_type {
        AlpenglowEpochType::Alpenglow { .. } => {
            if ag_points == 0 {
                return skip_reward(SkippedReason::ZeroPoints);
            }
            // In alpenglow, `points` represents the actual reward that this `stake` earned.
            ag_points
        }
        AlpenglowEpochType::Tower => {
            if tower_points == 0 {
                if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
                    inflation_point_calc_tracer(&SkippedReason::ZeroPoints.into());
                }
                return None;
            }
            if point_value.points == 0 {
                if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
                    inflation_point_calc_tracer(&SkippedReason::ZeroPointValue.into());
                }
                return None;
            }
            // In tower, `points` still needs to be scaled by `point_value` to calculate this
            // `vote_state` earned.
            // The final unwrap is safe, as points_value.points is guaranteed to be non zero above.
            tower_points
                .checked_mul(u128::from(point_value.rewards))
                .expect("Rewards intermediate calculation should fit within u128")
                .checked_div(point_value.points)
                .unwrap()
        }
        AlpenglowEpochType::MigrationEpoch {
            num_tower_slots,
            num_ag_slots,
            ..
        } => {
            if tower_points == 0 && ag_points == 0 {
                return skip_reward(SkippedReason::ZeroPoints);
            }
            if ag_points == 0 && point_value.points == 0 {
                return skip_reward(SkippedReason::ZeroPointValue);
            }
            let total_slots = (num_tower_slots + num_ag_slots) as u128;
            let tower_points = tower_points
                .checked_mul(u128::from(point_value.rewards))
                .expect("Rewards intermediate calculation should fit within u128")
                .checked_div(point_value.points)
                .unwrap()
                .checked_mul(*num_tower_slots as u128)
                .unwrap()
                .checked_div(total_slots)
                .unwrap();
            tower_points + ag_points
        }
    };

    let rewards = u64::try_from(rewards).expect("Rewards should fit within u64");

    // don't bother trying to split if fractional lamports got truncated
    if rewards == 0 {
        return skip_reward(SkippedReason::ZeroReward);
    }
    let (voter_rewards, staker_rewards, is_split) = if is_tower_epoch {
        commission_split(voter_commission_bps, rewards)
    } else {
        commission_split_preserve_lamports(voter_commission_bps, rewards)
    };
    if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
        inflation_point_calc_tracer(&InflationPointCalculationEvent::SplitRewards(
            rewards,
            voter_rewards,
            staker_rewards,
            point_value.clone(),
        ));
    }

    if (voter_rewards == 0 || staker_rewards == 0) && is_split && is_tower_epoch {
        // In Tower, don't collect if we lose a whole lamport somewhere.
        // is_split means there should be tokens on both sides; don't move
        // credits_observed if one side didn't get paid.
        if let Some(inflation_point_calc_tracer) = inflation_point_calc_tracer.as_ref() {
            inflation_point_calc_tracer(&SkippedReason::TooEarlyUnfairSplit.into());
        }
        return None;
    }

    Some(CalculatedStakeRewards {
        staker_rewards,
        voter_rewards,
        new_credits_observed,
    })
}

#[cfg(test)]
mod tests {
    use {
        self::points::{PointValue, null_tracer},
        super::*,
        crate::alpenglow_epoch_type::RewardEpochDelegatedStakes,
        agave_votor_messages::migration::AG_MIGRATION_EPOCH_CREDIT,
        solana_clock::Epoch,
        solana_native_token::LAMPORTS_PER_SOL,
        solana_pubkey::Pubkey,
        solana_rent::Rent,
        solana_stake_interface::{
            stake_history::StakeHistory,
            state::{Delegation, StakeStateV2},
        },
        solana_vote_program::vote_state::{VoteStateV4, handler::VoteStateHandler},
        test_case::{test_case, test_matrix},
    };

    fn new_stake(
        stake: u64,
        voter_pubkey: &Pubkey,
        vote_state: &VoteStateV4,
        activation_epoch: Epoch,
    ) -> Stake {
        Stake {
            delegation: Delegation::new(voter_pubkey, stake, activation_epoch),
            credits_observed: vote_state.credits(),
        }
    }

    /// Returns an instance of `AlpenglowEpochType`, total stake, and first AG epoch.
    fn get_ag_epoch_type() -> (AlpenglowEpochType, u64, Epoch) {
        let total_stake = 1_000;
        let migration_epoch = 0;
        let first_ag_epoch = migration_epoch + 1;
        (
            AlpenglowEpochType::Alpenglow {
                migration_epoch,
                reward_epoch_delegated_stakes: RewardEpochDelegatedStakes {
                    epoch: first_ag_epoch,
                    delegated_stakes: [(Pubkey::default(), total_stake)].into_iter().collect(),
                },
            },
            total_stake,
            first_ag_epoch,
        )
    }

    fn make_ag_epoch_type_for_test(
        ag_enabled: bool,
        vote_state: &VoteStateV4,
        ag_total_stake_multiplier: u64,
    ) -> AlpenglowEpochType {
        if ag_enabled {
            AlpenglowEpochType::Alpenglow {
                migration_epoch: 0,
                reward_epoch_delegated_stakes: RewardEpochDelegatedStakes {
                    epoch: vote_state
                        .epoch_credits
                        .last()
                        .map(|(epoch, _final_epoch_credits, _initial_epoch_credits)| *epoch)
                        .unwrap_or(1),
                    delegated_stakes: [(Pubkey::default(), ag_total_stake_multiplier)]
                        .into_iter()
                        .collect(),
                },
            }
        } else {
            AlpenglowEpochType::Tower
        }
    }

    #[test_matrix([true, false], [true, false])]
    fn test_stake_state_redeem_rewards(adjust_delegations_for_rent: bool, ag_enabled: bool) {
        let mut vote_state = VoteStateHandler::new_v4(VoteStateV4::default());
        // assume stake.stake() is right
        // bootstrap means fully-vested stake at epoch 0
        let stake_lamports = 1;
        let mut stake = new_stake(
            stake_lamports,
            &Pubkey::default(),
            vote_state.as_ref_v4(),
            u64::MAX,
        );
        let stake_history = &StakeHistory::default();
        let new_rate_activation_epoch = None;
        let commission_rate_in_basis_points = true;

        // epoch credits work differently in AG, so we need a multiplier to account for that.
        let ag_total_stake_multiplier = if ag_enabled {
            let (_, ag_total_stake_multiplier, _) = get_ag_epoch_type();
            ag_total_stake_multiplier
        } else {
            1
        };

        let inc_credits = |handler: &mut VoteStateHandler, epoch: Epoch, credits: u64| {
            if ag_enabled {
                let (_, _, first_ag_epoch) = get_ag_epoch_type();
                handler
                    .increment_credits(epoch + first_ag_epoch, credits * ag_total_stake_multiplier);
            } else {
                handler.increment_credits(epoch, credits);
            }
        };
        let mut rent = Rent::default();
        let minimum_balance = rent.minimum_balance(StakeStateV2::size_of());

        // Adjust rent down, no impact
        if adjust_delegations_for_rent {
            rent.lamports_per_byte /= 2;
        }
        let new_minimum_balance = rent.minimum_balance(StakeStateV2::size_of());

        // this one can't collect now, credits_observed == vote_state.credits()
        assert_eq!(
            None,
            redeem_stake_rewards(
                &mut stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 0,
                    point_value: &PointValue {
                        rewards: 1_000_000_000,
                        points: 1
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier,
                ),
                stake_lamports + minimum_balance,
                new_minimum_balance,
            )
        );

        // put 2 credits in at epoch 0
        inc_credits(&mut vote_state, 0, 2);

        // this one should be able to collect exactly 2
        assert_eq!(
            Some((stake_lamports * 2, 0)),
            redeem_stake_rewards(
                &mut stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 0,
                    point_value: &PointValue {
                        rewards: 1,
                        points: 1
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier,
                ),
                stake_lamports + minimum_balance,
                new_minimum_balance,
            )
        );

        assert_eq!(
            stake.delegation.stake,
            stake_lamports + (stake_lamports * 2)
        );
        assert_eq!(stake.credits_observed, 2 * ag_total_stake_multiplier);
    }

    #[test_matrix([true, false])]
    fn test_stake_state_calculate_rewards(ag_enabled: bool) {
        let mut vote_state = VoteStateHandler::new_v4(VoteStateV4::default());
        // assume stake.stake() is right
        // bootstrap means fully-vested stake at epoch 0
        let mut stake = new_stake(1, &Pubkey::default(), vote_state.as_ref_v4(), u64::MAX);

        let stake_history = &StakeHistory::default();
        let new_rate_activation_epoch = None;
        let commission_rate_in_basis_points = true;
        let adjust_delegations_for_rent = true;

        // epoch credits work differently in AG, so we need a multiplier to account for that.
        let ag_total_stake_multiplier = if ag_enabled {
            let (_, ag_total_stake_multiplier, _) = get_ag_epoch_type();
            ag_total_stake_multiplier
        } else {
            1
        };

        let inc_credits = |handler: &mut VoteStateHandler, epoch: Epoch, credits: u64| {
            if ag_enabled {
                let (_, _, first_ag_epoch) = get_ag_epoch_type();
                handler
                    .increment_credits(epoch + first_ag_epoch, credits * ag_total_stake_multiplier);
            } else {
                handler.increment_credits(epoch, credits);
            }
        };
        // this one can't collect now, credits_observed == vote_state.credits()
        assert_eq!(
            None,
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 0,
                    point_value: &PointValue {
                        rewards: 1_000_000_000,
                        points: 1
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        // put 2 credits in at epoch 0
        inc_credits(&mut vote_state, 0, 2);

        // this one should be able to collect exactly 2
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: stake.delegation.stake * 2,
                voter_rewards: 0,
                new_credits_observed: 2 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 0,
                    point_value: &PointValue {
                        rewards: 2,
                        points: 2 // all his
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        stake.credits_observed = ag_total_stake_multiplier;
        // this one should be able to collect exactly 1 (already observed one)
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: stake.delegation.stake,
                voter_rewards: 0,
                new_credits_observed: 2 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 0,
                    point_value: &PointValue {
                        rewards: 1,
                        points: 1
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        // put 1 credit in epoch 1
        inc_credits(&mut vote_state, 1, 1);

        stake.credits_observed = 2 * ag_total_stake_multiplier;
        // this one should be able to collect the one just added
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: stake.delegation.stake,
                voter_rewards: 0,
                new_credits_observed: 3 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 1,
                    point_value: &PointValue {
                        rewards: 2,
                        points: 2
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        // put 1 credit in epoch 2
        inc_credits(&mut vote_state, 2, 1);
        // Tower redeems all unobserved epoch credits. Full AG only considers
        // the latest epoch credit entry.
        let expected_staker_rewards = if ag_enabled {
            stake.delegation.stake
        } else {
            stake.delegation.stake * 2
        };
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: expected_staker_rewards,
                voter_rewards: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 2,
                        points: 2
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        stake.credits_observed = 0;
        // Tower collects everything from t=0. Full AG only considers the
        // latest epoch credit entry.
        let expected_staker_rewards = if ag_enabled {
            stake.delegation.stake
        } else {
            stake.delegation.stake * 2 // epoch 0
                + stake.delegation.stake // epoch 1
                + stake.delegation.stake // epoch 2
        };
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: expected_staker_rewards,
                voter_rewards: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 4,
                        points: 4
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        let small_redemption_result = || {
            ag_enabled.then_some(CalculatedStakeRewards {
                staker_rewards: 0,
                voter_rewards: 1,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            })
        };

        // same as above, but is a small enough reward that both sides round to
        // zero after the Tower commission split. Tower defers; AG assigns the
        // remainder to the voter and advances credits.
        vote_state.set_inflation_rewards_commission_bps(100);
        assert_eq!(
            small_redemption_result(),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 4,
                        points: 4
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );
        vote_state.set_inflation_rewards_commission_bps(9900);
        assert_eq!(
            small_redemption_result(),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 4,
                        points: 4
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        // now one with inflation disabled. no one gets paid, but we still need
        // to advance the stake state's credits_observed field to prevent back-
        // paying rewards when inflation is turned on.
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: 0,
                voter_rewards: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 0,
                        points: 4
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        // credits_observed remains at previous level when vote_state credits are
        // not advancing and inflation is disabled
        stake.credits_observed = 4 * ag_total_stake_multiplier;
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: 0,
                voter_rewards: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 0,
                        points: 4
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        assert_eq!(
            CalculatedStakePoints {
                tower_points: 0,
                ag_points: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
                force_credits_update_with_skipped_reward: false,
            },
            calculate_stake_points_and_credits(
                &stake,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                &StakeHistory::default(),
                null_tracer(),
                None,
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
                true,
            )
        );

        // credits_observed is auto-rewound when vote_state credits are assumed to have been
        // recreated
        stake.credits_observed = 1000 * ag_total_stake_multiplier;
        // this is new behavior 1; return the post-recreation rewound credits from the vote account
        assert_eq!(
            CalculatedStakePoints {
                tower_points: 0,
                ag_points: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
                force_credits_update_with_skipped_reward: true,
            },
            calculate_stake_points_and_credits(
                &stake,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                &StakeHistory::default(),
                null_tracer(),
                None,
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
                true,
            )
        );
        // this is new behavior 2; don't hint when credits both from stake and vote are identical
        stake.credits_observed = 4 * ag_total_stake_multiplier;
        assert_eq!(
            CalculatedStakePoints {
                tower_points: 0,
                ag_points: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
                force_credits_update_with_skipped_reward: false,
            },
            calculate_stake_points_and_credits(
                &stake,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                &StakeHistory::default(),
                null_tracer(),
                None,
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
                true,
            )
        );

        // get rewards and credits observed when not the activation epoch
        vote_state.set_inflation_rewards_commission_bps(0);
        stake.credits_observed = 3 * ag_total_stake_multiplier;
        stake.delegation.activation_epoch = 1;
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: stake.delegation.stake, // epoch 2
                voter_rewards: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 1,
                        points: 1
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );

        // credits_observed is moved forward for the stake's activation epoch,
        // and no rewards are perceived
        stake.delegation.activation_epoch = 2;
        stake.credits_observed = 3 * ag_total_stake_multiplier;
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: 0,
                voter_rewards: 0,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 2,
                    point_value: &PointValue {
                        rewards: 1,
                        points: 1
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &make_ag_epoch_type_for_test(
                    ag_enabled,
                    vote_state.as_ref_v4(),
                    ag_total_stake_multiplier
                ),
            )
        );
    }

    #[test_case(u64::MAX, 1_000, u64::MAX => panics "Rewards intermediate calculation should fit within u128")]
    #[test_case(1, u64::MAX, u64::MAX => panics "Rewards should fit within u64")]
    fn calculate_rewards_tests(stake: u64, rewards: u64, credits: u64) {
        let mut vote_state = VoteStateHandler::new_v4(VoteStateV4::default());

        let stake = new_stake(stake, &Pubkey::default(), vote_state.as_ref_v4(), u64::MAX);

        vote_state.increment_credits(0, credits);

        let stake_history = &StakeHistory::default();
        let new_rate_activation_epoch = None;
        let commission_rate_in_basis_points = true;
        let adjust_delegations_for_rent = true;

        calculate_stake_rewards(
            &stake,
            vote_state.as_ref_v4().inflation_rewards_commission_bps,
            DelegatedVoteState::from(vote_state.as_ref_v4()),
            CalculationEnvironment {
                rewarded_epoch: 0,
                point_value: &PointValue { rewards, points: 1 },
                stake_history,
                new_rate_activation_epoch,
                commission_rate_in_basis_points,
                adjust_delegations_for_rent,
                use_fixed_point_stake_math: true,
            },
            null_tracer(),
            &AlpenglowEpochType::Tower,
        );
    }

    #[test_matrix([true, false])]
    fn test_stake_state_calculate_points_with_typical_values(ag_enabled: bool) {
        let vote_state = VoteStateHandler::new_v4(VoteStateV4::default());

        // bootstrap means fully-vested stake at epoch 0 with
        //  10_000_000 SOL is a big but not unreasaonable stake
        let stake = new_stake(
            10_000_000 * LAMPORTS_PER_SOL,
            &Pubkey::default(),
            vote_state.as_ref_v4(),
            u64::MAX,
        );
        let stake_history = &StakeHistory::default();
        let new_rate_activation_epoch = None;
        let commission_rate_in_basis_points = true;
        let adjust_delegations_for_rent = true;

        let ag_stake_state = if ag_enabled {
            let (state, _, _) = get_ag_epoch_type();
            state
        } else {
            AlpenglowEpochType::Tower
        };

        // this one can't collect now, credits_observed == vote_state.credits()
        assert_eq!(
            None,
            calculate_stake_rewards(
                &stake,
                vote_state.as_ref_v4().inflation_rewards_commission_bps,
                DelegatedVoteState::from(vote_state.as_ref_v4()),
                CalculationEnvironment {
                    rewarded_epoch: 0,
                    point_value: &PointValue {
                        rewards: 1_000_000_000,
                        points: 1
                    },
                    stake_history,
                    new_rate_activation_epoch,
                    commission_rate_in_basis_points,
                    adjust_delegations_for_rent,
                    use_fixed_point_stake_math: true,
                },
                null_tracer(),
                &ag_stake_state,
            )
        );
    }

    #[test]
    fn test_migration_epoch_dust_split_advances_credits() {
        let (_, ag_total_stake_multiplier, _) = get_ag_epoch_type();
        let mut vote_state = VoteStateV4 {
            inflation_rewards_commission_bps: 100,
            epoch_credits: vec![
                AG_MIGRATION_EPOCH_CREDIT,
                (0, 4 * ag_total_stake_multiplier, 0),
            ],
            ..VoteStateV4::default()
        };
        let stake = Stake {
            delegation: Delegation::new(&Pubkey::default(), 1, u64::MAX),
            credits_observed: 0,
        };
        let point_value = PointValue {
            rewards: 4,
            points: 1,
        };
        let stake_history = StakeHistory::default();
        let calculation_environment = || CalculationEnvironment {
            rewarded_epoch: 0,
            point_value: &point_value,
            stake_history: &stake_history,
            new_rate_activation_epoch: None,
            commission_rate_in_basis_points: true,
            adjust_delegations_for_rent: true,
            use_fixed_point_stake_math: true,
        };
        let migration_epoch_type = AlpenglowEpochType::MigrationEpoch {
            num_tower_slots: 0,
            num_ag_slots: 1,
            migration_epoch: 0,
            reward_epoch_delegated_stakes: RewardEpochDelegatedStakes {
                epoch: 0,
                delegated_stakes: [(Pubkey::default(), ag_total_stake_multiplier)]
                    .into_iter()
                    .collect(),
            },
        };

        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: 3,
                voter_rewards: 1,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.inflation_rewards_commission_bps,
                DelegatedVoteState::from(&vote_state),
                calculation_environment(),
                null_tracer(),
                &migration_epoch_type,
            )
        );

        vote_state.inflation_rewards_commission_bps = 9_900;
        assert_eq!(
            Some(CalculatedStakeRewards {
                staker_rewards: 0,
                voter_rewards: 4,
                new_credits_observed: 4 * ag_total_stake_multiplier,
            }),
            calculate_stake_rewards(
                &stake,
                vote_state.inflation_rewards_commission_bps,
                DelegatedVoteState::from(&vote_state),
                calculation_environment(),
                null_tracer(),
                &migration_epoch_type,
            )
        );
    }
}
