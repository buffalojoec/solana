//! Splitting a reward amount between a voter's commission and its stakers.

/// The outcome of splitting a reward between a voter's commission and its
/// stakers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommissionSplit {
    /// The voter's commission portion.
    pub voter: u64,
    /// The stakers' portion.
    pub staker: u64,
    /// Whether the reward was actually split between both parties. False when
    /// the commission is 0% or 100%, where one party receives everything.
    pub was_split: bool,
}

/// returns the commission split between voter and stakers
///
///  if commission calculation is 100% one way or other,
///   indicate with false for was_split
pub(crate) fn commission_split(commission_bps: u16, on: u64) -> CommissionSplit {
    const MAX_BPS: u16 = 10_000;
    const MAX_BPS_U128: u128 = MAX_BPS as u128;
    match commission_bps.min(MAX_BPS) {
        0 => CommissionSplit {
            voter: 0,
            staker: on,
            was_split: false,
        },
        MAX_BPS => CommissionSplit {
            voter: on,
            staker: 0,
            was_split: false,
        },
        split => {
            let on = u128::from(on);
            // Calculate mine and theirs independently and symmetrically instead of
            // using the remainder of the other to treat them strictly equally.
            // In Tower, this is also to cancel the rewarding if either of the parties
            // should receive only fractional lamports, resulting in not being rewarded at all.
            // Thus, note that we intentionally discard any residual fractional lamports.
            let mine = on
                .checked_mul(u128::from(split))
                .expect("multiplication of a u64 and u16 should not overflow")
                / MAX_BPS_U128;
            let theirs = on
                .checked_mul(u128::from(
                    MAX_BPS
                        .checked_sub(split)
                        .expect("commission cannot be greater than MAX_BPS"),
                ))
                .expect("multiplication of a u64 and u16 should not overflow")
                / MAX_BPS_U128;

            CommissionSplit {
                voter: mine as u64,
                staker: theirs as u64,
                was_split: true,
            }
        }
    }
}

/// returns the commission split between voter and stakers, assigning any
/// fractional-lamport remainder to the voter so no lamports are lost.
///
/// This is used only for non-Tower epochs, where small unfair splits no longer defer redemption.
pub(crate) fn commission_split_preserve_lamports(commission_bps: u16, on: u64) -> CommissionSplit {
    const MAX_BPS: u16 = 10_000;
    const MAX_BPS_U128: u128 = MAX_BPS as u128;
    match commission_bps.min(MAX_BPS) {
        0 => CommissionSplit {
            voter: 0,
            staker: on,
            was_split: false,
        },
        MAX_BPS => CommissionSplit {
            voter: on,
            staker: 0,
            was_split: false,
        },
        split => {
            let staker_bps = MAX_BPS
                .checked_sub(split)
                .expect("commission cannot be greater than MAX_BPS");
            let staker_rewards = u128::from(on)
                .checked_mul(u128::from(staker_bps))
                .expect("multiplication of a u64 and u16 should not overflow")
                / MAX_BPS_U128;
            let staker_rewards = staker_rewards as u64;
            let voter_rewards = on
                .checked_sub(staker_rewards)
                .expect("staker rewards cannot exceed total rewards");

            CommissionSplit {
                voter: voter_rewards,
                staker: staker_rewards,
                was_split: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use {super::*, proptest::prelude::*};

    /// Terse constructor for the expected split in assertions.
    fn split(voter_rewards: u64, staker_rewards: u64, was_split: bool) -> CommissionSplit {
        CommissionSplit {
            voter: voter_rewards,
            staker: staker_rewards,
            was_split,
        }
    }

    #[test]
    fn test_commission_split_bps() {
        // 0% commission
        assert_eq!(commission_split(0, 1), split(0, 1, false));
        assert_eq!(commission_split(0, 10), split(0, 10, false));
        assert_eq!(commission_split(0, 100), split(0, 100, false));
        assert_eq!(commission_split(0, 1_000), split(0, 1_000, false));
        assert_eq!(commission_split(0, u64::MAX), split(0, u64::MAX, false));

        // 100% commission (10,000 bps)
        assert_eq!(commission_split(10_000, 1), split(1, 0, false));
        assert_eq!(commission_split(10_000, 10), split(10, 0, false));
        assert_eq!(commission_split(10_000, 100), split(100, 0, false));
        assert_eq!(commission_split(10_000, 1_000), split(1_000, 0, false));
        assert_eq!(
            commission_split(10_000, u64::MAX),
            split(u64::MAX, 0, false)
        );

        // Values > 10,000 bps are capped at 100%
        assert_eq!(commission_split(u16::MAX, 1), split(1, 0, false));
        assert_eq!(commission_split(u16::MAX, 10), split(10, 0, false));
        assert_eq!(commission_split(u16::MAX, 100), split(100, 0, false));
        assert_eq!(commission_split(u16::MAX, 1_000), split(1_000, 0, false));
        assert_eq!(
            commission_split(u16::MAX, u64::MAX),
            split(u64::MAX, 0, false)
        );

        // 99% commission (9,900 bps)
        assert_eq!(commission_split(9_900, 1), split(0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_900, 10), split(9, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_900, 100), split(99, 1, true));
        assert_eq!(commission_split(9_900, 1_000), split(990, 10, true));
        assert_eq!(
            commission_split(9_900, u64::MAX),
            split(
                (u64::MAX as u128 * 9_900 / 10_000) as u64,
                (u64::MAX as u128 * 100 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 99.99% commission (9,999 bps)
        assert_eq!(commission_split(9_999, 1), split(0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_999, 10), split(9, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_999, 100), split(99, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_999, 1_000), split(999, 0, true)); // 1-lamport truncation
        assert_eq!(
            commission_split(9_999, u64::MAX),
            split(
                (u64::MAX as u128 * 9_999 / 10_000) as u64,
                (u64::MAX as u128 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 1% commission (100 bps)
        assert_eq!(commission_split(100, 1), split(0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(100, 10), split(0, 9, true)); // 1-lamport truncation
        assert_eq!(commission_split(100, 100), split(1, 99, true));
        assert_eq!(commission_split(100, 1_000), split(10, 990, true));
        assert_eq!(
            commission_split(100, u64::MAX),
            split(
                (u64::MAX as u128 * 100 / 10_000) as u64,
                (u64::MAX as u128 * 9_900 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 50% commission (5,000 bps)
        assert_eq!(commission_split(5_000, 1), split(0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(5_000, 10), split(5, 5, true));
        assert_eq!(commission_split(5_000, 100), split(50, 50, true));
        assert_eq!(commission_split(5_000, 1_000), split(500, 500, true));
        assert_eq!(
            commission_split(5_000, u64::MAX),
            split(
                (u64::MAX as u128 * 5_000 / 10_000) as u64,
                (u64::MAX as u128 * 5_000 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 12.34% commission (1,234 bps)
        assert_eq!(commission_split(1_234, 1), split(0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(1_234, 10), split(1, 8, true)); // 1-lamport truncation
        assert_eq!(commission_split(1_234, 1_000), split(123, 876, true)); // 1-lamport truncation
        assert_eq!(commission_split(1_234, 10_000), split(1_234, 8_766, true));
        assert_eq!(
            commission_split(1_234, u64::MAX),
            split(
                (u64::MAX as u128 * 1_234 / 10_000) as u64,
                (u64::MAX as u128 * 8_766 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 33.33% commission (3,333 bps)
        assert_eq!(commission_split(3_333, 1), split(0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(3_333, 10), split(3, 6, true)); // 1-lamport truncation
        assert_eq!(commission_split(3_333, 1_000), split(333, 666, true)); // 1-lamport truncation
        assert_eq!(commission_split(3_333, 10_000), split(3_333, 6_667, true));
        assert_eq!(
            commission_split(3_333, u64::MAX),
            split(
                (u64::MAX as u128 * 3_333 / 10_000) as u64,
                (u64::MAX as u128 * 6_667 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation
    }

    #[test]
    fn test_commission_split_preserve_lamports_bps() {
        // 0% commission
        assert_eq!(commission_split_preserve_lamports(0, 1), split(0, 1, false));
        assert_eq!(
            commission_split_preserve_lamports(0, 10),
            split(0, 10, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(0, 100),
            split(0, 100, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(0, u64::MAX),
            split(0, u64::MAX, false)
        );

        // 100% commission (10,000 bps)
        assert_eq!(
            commission_split_preserve_lamports(10_000, 1),
            split(1, 0, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(10_000, 10),
            split(10, 0, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(10_000, 100),
            split(100, 0, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(10_000, u64::MAX),
            split(u64::MAX, 0, false)
        );

        // Values > 10,000 bps are capped at 100%
        assert_eq!(
            commission_split_preserve_lamports(u16::MAX, 1),
            split(1, 0, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(u16::MAX, u64::MAX),
            split(u64::MAX, 0, false)
        );

        // Remainder lamports go to the voter.
        assert_eq!(
            commission_split_preserve_lamports(9_900, 1),
            split(1, 0, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(9_900, 10),
            split(10, 0, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(9_900, 100),
            split(99, 1, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(9_900, 1_000),
            split(990, 10, true)
        );

        assert_eq!(
            commission_split_preserve_lamports(100, 1),
            split(1, 0, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(100, 10),
            split(1, 9, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(100, 100),
            split(1, 99, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(100, 1_000),
            split(10, 990, true)
        );

        assert_eq!(
            commission_split_preserve_lamports(5_000, 1),
            split(1, 0, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(5_000, 10),
            split(5, 5, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(5_000, 100),
            split(50, 50, true)
        );

        assert_eq!(
            commission_split_preserve_lamports(1_234, 1),
            split(1, 0, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(1_234, 10),
            split(2, 8, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(1_234, 1_000),
            split(124, 876, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(1_234, 10_000),
            split(1_234, 8_766, true)
        );

        assert_eq!(
            commission_split_preserve_lamports(3_333, 1),
            split(1, 0, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(3_333, 10),
            split(4, 6, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(3_333, 1_000),
            split(334, 666, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(3_333, 10_000),
            split(3_333, 6_667, true)
        );
    }

    proptest! {
        #[test]
        fn test_commission_split_properties(
            commission_bps in 0..=u16::MAX,
            rewards in 0..=u64::MAX,
        ) {
            let CommissionSplit { voter, staker, was_split } =
                commission_split(commission_bps, rewards);

            // Invariant 1: No overflow — voter + staker never exceeds rewards.
            prop_assert!(voter + staker <= rewards);

            // Invariant 2: At most 1 lamport lost to truncation.
            prop_assert!(rewards - voter - staker <= 1);

            // Invariant 3: was_split is false only at the 0% and 100% boundaries.
            let effective_bps = commission_bps.min(10_000);
            if effective_bps == 0 || effective_bps == 10_000 {
                prop_assert!(!was_split);
            } else {
                prop_assert!(was_split);
            }

            // Invariant 4: Boundary — 0% commission gives everything to staker.
            if effective_bps == 0 {
                prop_assert_eq!(voter, 0);
                prop_assert_eq!(staker, rewards);
            }

            // Invariant 5: Boundary — 100% commission gives everything to voter.
            if effective_bps == 10_000 {
                prop_assert_eq!(voter, rewards);
                prop_assert_eq!(staker, 0);
            }

            // Invariant 6: Clamping — values above 10,000 bps behave as 10,000.
            if commission_bps > 10_000 {
                let CommissionSplit {
                    voter: clamped_voter,
                    staker: clamped_staker,
                    was_split: clamped_ws,
                } = commission_split(10_000, rewards);
                prop_assert_eq!(voter, clamped_voter);
                prop_assert_eq!(staker, clamped_staker);
                prop_assert_eq!(was_split, clamped_ws);
            }

            // Invariant 7: Monotonicity — higher commission means voter >= what
            // they'd get with a lower commission (for the same rewards).
            if commission_bps > 0 {
                let lower_bps = commission_bps - 1;
                let lower_voter = commission_split(lower_bps, rewards).voter;
                prop_assert!(voter >= lower_voter);
            }

            // Invariant 8: Exact split when bps divides evenly.
            // voter == rewards * effective_bps / 10_000 (using u128 math).
            let expected_voter =
                (u128::from(rewards) * u128::from(effective_bps) / 10_000) as u64;
            let expected_staker =
                (u128::from(rewards) * u128::from(10_000 - effective_bps) / 10_000) as u64;
            prop_assert_eq!(voter, expected_voter);
            prop_assert_eq!(staker, expected_staker);
        }

        #[test]
        fn test_commission_split_preserve_lamports_properties(
            commission_bps in 0..=u16::MAX,
            rewards in 0..=u64::MAX,
        ) {
            let CommissionSplit { voter, staker, was_split } =
                commission_split_preserve_lamports(commission_bps, rewards);

            // Invariant 1: The full reward amount is assigned.
            prop_assert_eq!(voter + staker, rewards);

            // Invariant 2: was_split is false only at the 0% and 100% boundaries.
            let effective_bps = commission_bps.min(10_000);
            if effective_bps == 0 || effective_bps == 10_000 {
                prop_assert!(!was_split);
            } else {
                prop_assert!(was_split);
            }

            // Invariant 3: Boundary - 0% commission gives everything to staker.
            if effective_bps == 0 {
                prop_assert_eq!(voter, 0);
                prop_assert_eq!(staker, rewards);
            }

            // Invariant 4: Boundary - 100% commission gives everything to voter.
            if effective_bps == 10_000 {
                prop_assert_eq!(voter, rewards);
                prop_assert_eq!(staker, 0);
            }

            // Invariant 5: Clamping - values above 10,000 bps behave as 10,000.
            if commission_bps > 10_000 {
                let CommissionSplit {
                    voter: clamped_voter,
                    staker: clamped_staker,
                    was_split: clamped_ws,
                } = commission_split_preserve_lamports(10_000, rewards);
                prop_assert_eq!(voter, clamped_voter);
                prop_assert_eq!(staker, clamped_staker);
                prop_assert_eq!(was_split, clamped_ws);
            }

            // Invariant 6: Higher commission does not decrease the voter amount.
            if commission_bps > 0 {
                let lower_bps = commission_bps - 1;
                let lower_voter =
                    commission_split_preserve_lamports(lower_bps, rewards).voter;
                prop_assert!(voter >= lower_voter);
            }

            // Invariant 7: The staker side is floored, and the voter gets the remainder.
            let staker_bps = 10_000 - effective_bps;
            let expected_staker =
                (u128::from(rewards) * u128::from(staker_bps) / 10_000) as u64;
            let expected_voter = rewards - expected_staker;
            prop_assert_eq!(voter, expected_voter);
            prop_assert_eq!(staker, expected_staker);
        }
    }
}
