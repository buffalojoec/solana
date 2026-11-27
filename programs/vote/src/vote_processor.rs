//! Vote program processor

use {
    crate::vote_state::{self, handler::VoteStateTargetVersion},
    log::*,
    solana_instruction_error::InstructionError,
    solana_packet::PACKET_DATA_WINCODE_CONFIG,
    solana_program_runtime::{
        declare_process_instruction, invoke_context::InvokeContext,
        sysvar_cache::get_sysvar_with_account_check,
    },
    solana_pubkey::Pubkey,
    solana_transaction_context::{
        instruction::InstructionContext, instruction_accounts::BorrowedInstructionAccount,
    },
    solana_vote_interface::{instruction::VoteInstruction, program::id, state::VoteAuthorize},
    std::collections::HashSet,
};

#[allow(clippy::too_many_arguments)]
fn process_authorize_with_seed_instruction<F>(
    invoke_context: &InvokeContext,
    instruction_context: &InstructionContext,
    vote_account: &mut BorrowedInstructionAccount,
    target_version: VoteStateTargetVersion,
    new_authority: &Pubkey,
    authorization_type: VoteAuthorize,
    current_authority_derived_key_owner: &Pubkey,
    current_authority_derived_key_seed: &str,
    consume_pop_compute_units: F,
) -> Result<(), InstructionError>
where
    F: FnOnce() -> Result<(), InstructionError>,
{
    let clock = get_sysvar_with_account_check::clock(invoke_context, instruction_context, 1)?;
    let mut expected_authority_keys: HashSet<Pubkey> = HashSet::default();
    if instruction_context.is_instruction_account_signer(2)? {
        let base_pubkey = instruction_context.get_key_of_instruction_account(2)?;
        // The conversion from `PubkeyError` to `InstructionError` through
        // num-traits is incorrect, but it's the existing behavior.
        expected_authority_keys.insert(
            Pubkey::create_with_seed(
                base_pubkey,
                current_authority_derived_key_seed,
                current_authority_derived_key_owner,
            )
            .map_err(|e| e as u64)?,
        );
    };
    vote_state::authorize(
        vote_account,
        target_version,
        new_authority,
        authorization_type,
        &expected_authority_keys,
        &clock,
        consume_pop_compute_units,
    )
}

fn is_init_account_v2_enabled(invoke_context: &InvokeContext) -> bool {
    let feature_set = invoke_context.get_feature_set();
    feature_set.bls_pubkey_management_in_vote_account
        && feature_set.block_revenue_sharing
        && feature_set.vote_account_initialize_v2
}

// Citing `runtime/src/block_cost_limit.rs`, vote has statically defined 2100
// units; can consume based on instructions in the future like `bpf_loader` does.
pub const DEFAULT_COMPUTE_UNITS: u64 = 2_100;

/// Cost in compute units for BLS proof-of-possession verification (SIMD-0387).
pub const BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS: u64 = 34_500;

declare_process_instruction!(Entrypoint, DEFAULT_COMPUTE_UNITS, |invoke_context| {
    let transaction_context = &invoke_context.transaction_context;
    let instruction_context = transaction_context.get_current_instruction_context()?;
    let data = instruction_context.get_instruction_data();

    trace!("process_instruction: {data:?}");

    let mut me = instruction_context.try_borrow_instruction_account(0)?;
    if *me.get_owner() != id() {
        return Err(InstructionError::InvalidAccountOwner);
    }

    // Determine the target vote state version to use for all operations.
    let target_version = VoteStateTargetVersion::V4;

    let signers = instruction_context.get_signers()?;
    let is_init_account_v2_enabled = is_init_account_v2_enabled(invoke_context);
    let consume_pop_compute_units = || {
        invoke_context
            .consume_checked(BLS_PROOF_OF_POSSESSION_VERIFICATION_COMPUTE_UNITS)
            .map_err(|_| InstructionError::ComputationalBudgetExceeded)
    };
    let instruction: VoteInstruction =
        wincode::config::deserialize(data, PACKET_DATA_WINCODE_CONFIG)
            .map_err(|_| InstructionError::InvalidInstructionData)?;
    match instruction {
        VoteInstruction::InitializeAccount(vote_init) => {
            let rent =
                get_sysvar_with_account_check::rent(invoke_context, &instruction_context, 1)?;
            if !rent.is_exempt(me.get_lamports(), me.get_data().len()) {
                return Err(InstructionError::InsufficientFunds);
            }
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, &instruction_context, 2)?;
            vote_state::initialize_account(&mut me, target_version, &vote_init, &signers, &clock)
        }
        VoteInstruction::Authorize(voter_pubkey, vote_authorize) => {
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, &instruction_context, 1)?;
            vote_state::authorize(
                &mut me,
                target_version,
                &voter_pubkey,
                vote_authorize,
                &signers,
                &clock,
                consume_pop_compute_units,
            )
        }
        VoteInstruction::AuthorizeWithSeed(args) => {
            instruction_context.check_number_of_instruction_accounts(3)?;
            process_authorize_with_seed_instruction(
                invoke_context,
                &instruction_context,
                &mut me,
                target_version,
                &args.new_authority,
                args.authorization_type,
                &args.current_authority_derived_key_owner,
                args.current_authority_derived_key_seed.as_str(),
                consume_pop_compute_units,
            )
        }
        VoteInstruction::AuthorizeCheckedWithSeed(args) => {
            instruction_context.check_number_of_instruction_accounts(4)?;
            let new_authority = instruction_context.get_key_of_instruction_account(3)?;
            if !instruction_context.is_instruction_account_signer(3)? {
                return Err(InstructionError::MissingRequiredSignature);
            }
            process_authorize_with_seed_instruction(
                invoke_context,
                &instruction_context,
                &mut me,
                target_version,
                new_authority,
                args.authorization_type,
                &args.current_authority_derived_key_owner,
                args.current_authority_derived_key_seed.as_str(),
                consume_pop_compute_units,
            )
        }
        VoteInstruction::UpdateValidatorIdentity => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let node_pubkey = instruction_context.get_key_of_instruction_account(1)?;
            vote_state::update_validator_identity(&mut me, target_version, node_pubkey, &signers)
        }
        VoteInstruction::UpdateCommission(commission) => {
            vote_state::update_commission(&mut me, target_version, commission, &signers)
        }
        VoteInstruction::Vote(_)
        | VoteInstruction::VoteSwitch(_, _)
        | VoteInstruction::UpdateVoteState(_)
        | VoteInstruction::UpdateVoteStateSwitch(_, _)
        | VoteInstruction::CompactUpdateVoteState(_)
        | VoteInstruction::CompactUpdateVoteStateSwitch(_, _)
        | VoteInstruction::TowerSync(_)
        | VoteInstruction::TowerSyncSwitch(_, _) => Err(InstructionError::InvalidInstructionData),
        VoteInstruction::Withdraw(lamports) => {
            instruction_context.check_number_of_instruction_accounts(2)?;
            let rent_sysvar = invoke_context
                .environment_config
                .sysvar_cache()
                .get_rent()?;
            let clock_sysvar = invoke_context
                .environment_config
                .sysvar_cache()
                .get_clock()?;

            drop(me);
            vote_state::withdraw(
                &instruction_context,
                0,
                target_version,
                lamports,
                1,
                &signers,
                &rent_sysvar,
                &clock_sysvar,
            )
        }
        VoteInstruction::AuthorizeChecked(vote_authorize) => {
            instruction_context.check_number_of_instruction_accounts(4)?;
            let voter_pubkey = instruction_context.get_key_of_instruction_account(3)?;
            if !instruction_context.is_instruction_account_signer(3)? {
                return Err(InstructionError::MissingRequiredSignature);
            }
            let clock =
                get_sysvar_with_account_check::clock(invoke_context, &instruction_context, 1)?;
            vote_state::authorize(
                &mut me,
                target_version,
                voter_pubkey,
                vote_authorize,
                &signers,
                &clock,
                consume_pop_compute_units,
            )
        }
        VoteInstruction::InitializeAccountV2(vote_init_v2) => {
            if !is_init_account_v2_enabled {
                return Err(InstructionError::InvalidInstructionData);
            }

            instruction_context.check_number_of_instruction_accounts(4)?;

            let sysvar_cache = invoke_context.environment_config.sysvar_cache();
            let clock = sysvar_cache.get_clock()?;
            let rent = sysvar_cache.get_rent()?;

            drop(me);
            vote_state::initialize_account_v2(
                &instruction_context,
                /* vote_account_index */ 0,
                target_version,
                &vote_init_v2,
                /* inflation_rewards_collector_index */ 2,
                /* block_revenue_collector_index */ 3,
                &signers,
                &clock,
                &rent,
                consume_pop_compute_units,
            )
        }
        VoteInstruction::UpdateCommissionBps {
            commission_bps,
            kind,
        } => vote_state::update_commission_bps(
            &mut me,
            target_version,
            commission_bps,
            kind,
            &signers,
            invoke_context.get_feature_set().block_revenue_sharing,
        ),
        VoteInstruction::UpdateCommissionCollector(kind) => {
            instruction_context.check_number_of_instruction_accounts(3)?;

            let rent = invoke_context
                .environment_config
                .sysvar_cache()
                .get_rent()?;

            drop(me);
            vote_state::update_commission_collector(
                &instruction_context,
                /* vote_account_index */ 0,
                target_version,
                /* new_collector_index */ 1,
                kind,
                &signers,
                &rent,
            )
        }
        VoteInstruction::DepositDelegatorRewards { .. } => {
            return Err(InstructionError::InvalidInstructionData);
        }
    }
});
