//! Transaction helpers.

use {
    crate::consts::PAYER_LAMPORTS,
    solana_account::AccountSharedData,
    solana_instruction::Instruction,
    solana_keypair::Keypair,
    solana_message::Message,
    solana_pubkey::Pubkey,
    solana_runtime::bank::Bank,
    solana_sdk_ids::system_program,
    solana_signer::Signer,
    solana_transaction::{Transaction, versioned::VersionedTransaction},
};

/// Craft a transaction invoking `target`.
pub(crate) fn invoke(bank: &Bank, target: &Pubkey) -> VersionedTransaction {
    versioned_transaction(
        bank,
        &[Instruction::new_with_bytes(*target, &[], Vec::new())],
    )
}

/// Craft a transaction over `instructions`, paid for by a fresh payer (to
/// avoid account lock contention).
pub(crate) fn versioned_transaction(
    bank: &Bank,
    instructions: &[Instruction],
) -> VersionedTransaction {
    let payer = Keypair::new();
    bank.store_account(
        &payer.pubkey(),
        &AccountSharedData::new(PAYER_LAMPORTS, 0, &system_program::id()),
    );
    let message = Message::new(instructions, Some(&payer.pubkey()));
    VersionedTransaction::from(Transaction::new(&[&payer], message, bank.last_blockhash()))
}
