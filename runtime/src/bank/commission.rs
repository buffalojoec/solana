//! Splitting a reward amount between a voter's commission and its stakers.

/// returns commission split as (voter_portion, staker_portion, was_split) tuple
///
///  if commission calculation is 100% one way or other,
///   indicate with false for was_split
pub(crate) fn commission_split(commission_bps: u16, on: u64) -> (u64, u64, bool) {
    const MAX_BPS: u16 = 10_000;
    const MAX_BPS_U128: u128 = MAX_BPS as u128;
    match commission_bps.min(MAX_BPS) {
        0 => (0, on, false),
        MAX_BPS => (on, 0, false),
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

            (mine as u64, theirs as u64, true)
        }
    }
}

/// returns commission split as (voter_portion, staker_portion, was_split) tuple,
/// assigning any fractional-lamport remainder to the voter so no lamports are lost.
///
/// This is used only for non-Tower epochs, where small unfair splits no longer defer redemption.
pub(crate) fn commission_split_preserve_lamports(commission_bps: u16, on: u64) -> (u64, u64, bool) {
    const MAX_BPS: u16 = 10_000;
    const MAX_BPS_U128: u128 = MAX_BPS as u128;
    match commission_bps.min(MAX_BPS) {
        0 => (0, on, false),
        MAX_BPS => (on, 0, false),
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

            (voter_rewards, staker_rewards, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use {super::*, proptest::prelude::*};

    #[test]
    fn test_commission_split_bps() {
        // 0% commission
        assert_eq!(commission_split(0, 1), (0, 1, false));
        assert_eq!(commission_split(0, 10), (0, 10, false));
        assert_eq!(commission_split(0, 100), (0, 100, false));
        assert_eq!(commission_split(0, 1_000), (0, 1_000, false));
        assert_eq!(commission_split(0, u64::MAX), (0, u64::MAX, false));

        // 100% commission (10,000 bps)
        assert_eq!(commission_split(10_000, 1), (1, 0, false));
        assert_eq!(commission_split(10_000, 10), (10, 0, false));
        assert_eq!(commission_split(10_000, 100), (100, 0, false));
        assert_eq!(commission_split(10_000, 1_000), (1_000, 0, false));
        assert_eq!(commission_split(10_000, u64::MAX), (u64::MAX, 0, false));

        // Values > 10,000 bps are capped at 100%
        assert_eq!(commission_split(u16::MAX, 1), (1, 0, false));
        assert_eq!(commission_split(u16::MAX, 10), (10, 0, false));
        assert_eq!(commission_split(u16::MAX, 100), (100, 0, false));
        assert_eq!(commission_split(u16::MAX, 1_000), (1_000, 0, false));
        assert_eq!(commission_split(u16::MAX, u64::MAX), (u64::MAX, 0, false));

        // 99% commission (9,900 bps)
        assert_eq!(commission_split(9_900, 1), (0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_900, 10), (9, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_900, 100), (99, 1, true));
        assert_eq!(commission_split(9_900, 1_000), (990, 10, true));
        assert_eq!(
            commission_split(9_900, u64::MAX),
            (
                (u64::MAX as u128 * 9_900 / 10_000) as u64,
                (u64::MAX as u128 * 100 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 99.99% commission (9,999 bps)
        assert_eq!(commission_split(9_999, 1), (0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_999, 10), (9, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_999, 100), (99, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(9_999, 1_000), (999, 0, true)); // 1-lamport truncation
        assert_eq!(
            commission_split(9_999, u64::MAX),
            (
                (u64::MAX as u128 * 9_999 / 10_000) as u64,
                (u64::MAX as u128 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 1% commission (100 bps)
        assert_eq!(commission_split(100, 1), (0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(100, 10), (0, 9, true)); // 1-lamport truncation
        assert_eq!(commission_split(100, 100), (1, 99, true));
        assert_eq!(commission_split(100, 1_000), (10, 990, true));
        assert_eq!(
            commission_split(100, u64::MAX),
            (
                (u64::MAX as u128 * 100 / 10_000) as u64,
                (u64::MAX as u128 * 9_900 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 50% commission (5,000 bps)
        assert_eq!(commission_split(5_000, 1), (0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(5_000, 10), (5, 5, true));
        assert_eq!(commission_split(5_000, 100), (50, 50, true));
        assert_eq!(commission_split(5_000, 1_000), (500, 500, true));
        assert_eq!(
            commission_split(5_000, u64::MAX),
            (
                (u64::MAX as u128 * 5_000 / 10_000) as u64,
                (u64::MAX as u128 * 5_000 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 12.34% commission (1,234 bps)
        assert_eq!(commission_split(1_234, 1), (0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(1_234, 10), (1, 8, true)); // 1-lamport truncation
        assert_eq!(commission_split(1_234, 1_000), (123, 876, true)); // 1-lamport truncation
        assert_eq!(commission_split(1_234, 10_000), (1_234, 8_766, true));
        assert_eq!(
            commission_split(1_234, u64::MAX),
            (
                (u64::MAX as u128 * 1_234 / 10_000) as u64,
                (u64::MAX as u128 * 8_766 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation

        // 33.33% commission (3,333 bps)
        assert_eq!(commission_split(3_333, 1), (0, 0, true)); // 1-lamport truncation
        assert_eq!(commission_split(3_333, 10), (3, 6, true)); // 1-lamport truncation
        assert_eq!(commission_split(3_333, 1_000), (333, 666, true)); // 1-lamport truncation
        assert_eq!(commission_split(3_333, 10_000), (3_333, 6_667, true));
        assert_eq!(
            commission_split(3_333, u64::MAX),
            (
                (u64::MAX as u128 * 3_333 / 10_000) as u64,
                (u64::MAX as u128 * 6_667 / 10_000) as u64,
                true
            )
        ); // 1-lamport truncation
    }

    #[test]
    fn test_commission_split_preserve_lamports_bps() {
        // 0% commission
        assert_eq!(commission_split_preserve_lamports(0, 1), (0, 1, false));
        assert_eq!(commission_split_preserve_lamports(0, 10), (0, 10, false));
        assert_eq!(commission_split_preserve_lamports(0, 100), (0, 100, false));
        assert_eq!(
            commission_split_preserve_lamports(0, u64::MAX),
            (0, u64::MAX, false)
        );

        // 100% commission (10,000 bps)
        assert_eq!(commission_split_preserve_lamports(10_000, 1), (1, 0, false));
        assert_eq!(
            commission_split_preserve_lamports(10_000, 10),
            (10, 0, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(10_000, 100),
            (100, 0, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(10_000, u64::MAX),
            (u64::MAX, 0, false)
        );

        // Values > 10,000 bps are capped at 100%
        assert_eq!(
            commission_split_preserve_lamports(u16::MAX, 1),
            (1, 0, false)
        );
        assert_eq!(
            commission_split_preserve_lamports(u16::MAX, u64::MAX),
            (u64::MAX, 0, false)
        );

        // Remainder lamports go to the voter.
        assert_eq!(commission_split_preserve_lamports(9_900, 1), (1, 0, true));
        assert_eq!(commission_split_preserve_lamports(9_900, 10), (10, 0, true));
        assert_eq!(
            commission_split_preserve_lamports(9_900, 100),
            (99, 1, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(9_900, 1_000),
            (990, 10, true)
        );

        assert_eq!(commission_split_preserve_lamports(100, 1), (1, 0, true));
        assert_eq!(commission_split_preserve_lamports(100, 10), (1, 9, true));
        assert_eq!(commission_split_preserve_lamports(100, 100), (1, 99, true));
        assert_eq!(
            commission_split_preserve_lamports(100, 1_000),
            (10, 990, true)
        );

        assert_eq!(commission_split_preserve_lamports(5_000, 1), (1, 0, true));
        assert_eq!(commission_split_preserve_lamports(5_000, 10), (5, 5, true));
        assert_eq!(
            commission_split_preserve_lamports(5_000, 100),
            (50, 50, true)
        );

        assert_eq!(commission_split_preserve_lamports(1_234, 1), (1, 0, true));
        assert_eq!(commission_split_preserve_lamports(1_234, 10), (2, 8, true));
        assert_eq!(
            commission_split_preserve_lamports(1_234, 1_000),
            (124, 876, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(1_234, 10_000),
            (1_234, 8_766, true)
        );

        assert_eq!(commission_split_preserve_lamports(3_333, 1), (1, 0, true));
        assert_eq!(commission_split_preserve_lamports(3_333, 10), (4, 6, true));
        assert_eq!(
            commission_split_preserve_lamports(3_333, 1_000),
            (334, 666, true)
        );
        assert_eq!(
            commission_split_preserve_lamports(3_333, 10_000),
            (3_333, 6_667, true)
        );
    }

    proptest! {
        #[test]
        fn test_commission_split_properties(
            commission_bps in 0..=u16::MAX,
            rewards in 0..=u64::MAX,
        ) {
            let (voter, staker, was_split) = commission_split(commission_bps, rewards);

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
                let (clamped_voter, clamped_staker, clamped_ws) =
                    commission_split(10_000, rewards);
                prop_assert_eq!(voter, clamped_voter);
                prop_assert_eq!(staker, clamped_staker);
                prop_assert_eq!(was_split, clamped_ws);
            }

            // Invariant 7: Monotonicity — higher commission means voter >= what
            // they'd get with a lower commission (for the same rewards).
            if commission_bps > 0 {
                let lower_bps = commission_bps - 1;
                let (lower_voter, _, _) = commission_split(lower_bps, rewards);
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
            let (voter, staker, was_split) =
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
                let (clamped_voter, clamped_staker, clamped_ws) =
                    commission_split_preserve_lamports(10_000, rewards);
                prop_assert_eq!(voter, clamped_voter);
                prop_assert_eq!(staker, clamped_staker);
                prop_assert_eq!(was_split, clamped_ws);
            }

            // Invariant 6: Higher commission does not decrease the voter amount.
            if commission_bps > 0 {
                let lower_bps = commission_bps - 1;
                let (lower_voter, _, _) = commission_split_preserve_lamports(lower_bps, rewards);
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
