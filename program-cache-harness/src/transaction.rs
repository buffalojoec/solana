//! Transaction helpers.

use {
    crate::consts::{DEFAULT_ENTRY_OWNER, NOOP_ELF, PAYER_LAMPORTS},
    solana_account::AccountSharedData,
    solana_instruction::Instruction,
    solana_keypair::Keypair,
    solana_loader_v3_interface::{
        get_program_data_address,
        instruction::{close_any, deploy_with_max_program_len, upgrade},
        state::UpgradeableLoaderState,
    },
    solana_message::Message,
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_runtime::bank::Bank,
    solana_sdk_ids::system_program,
    solana_signer::Signer,
    solana_transaction::{Transaction, versioned::VersionedTransaction},
};

/// Craft a transaction invoking `target`.
pub(crate) fn invoke(bank: &Bank, target: &Pubkey) -> VersionedTransaction {
    let payer = store_payer(bank);
    let instruction = Instruction::new_with_bytes(*target, &[], Vec::new());
    versioned_transaction(bank, &payer, &[], &[instruction])
}

/// Craft a transaction deploying `target`.
pub(crate) fn deploy(bank: &Bank, target: &Pubkey, authority: &Keypair) -> VersionedTransaction {
    let payer = store_payer(bank);
    let buffer = store_buffer(bank, authority);
    let deployed = bank
        .get_account(&get_program_data_address(target))
        .is_some();

    let instructions = if deployed {
        vec![upgrade(
            target,
            &buffer,
            &authority.pubkey(),
            &payer.pubkey(),
        )]
    } else {
        store_uninitialized_program(bank, target);
        deploy_with_max_program_len(
            &payer.pubkey(),
            target,
            &buffer,
            &authority.pubkey(),
            0,
            NOOP_ELF.len(),
        )
        .unwrap()
        .split_off(1)
    };
    versioned_transaction(bank, &payer, &[authority], &instructions)
}

/// Craft a transaction closing `target`.
pub(crate) fn close(bank: &Bank, target: &Pubkey, authority: &Keypair) -> VersionedTransaction {
    let payer = store_payer(bank);
    let instruction = close_any(
        &get_program_data_address(target),
        &payer.pubkey(),
        Some(&authority.pubkey()),
        Some(target),
    );
    versioned_transaction(bank, &payer, &[authority], &[instruction])
}

fn store_payer(bank: &Bank) -> Keypair {
    let payer = Keypair::new();
    bank.store_account(
        &payer.pubkey(),
        &AccountSharedData::new(PAYER_LAMPORTS, 0, &system_program::id()),
    );
    payer
}

fn store_buffer(bank: &Bank, authority: &Keypair) -> Pubkey {
    let buffer = Pubkey::new_unique();
    let mut data = bincode::serialize(&UpgradeableLoaderState::Buffer {
        authority_address: Some(authority.pubkey()),
    })
    .unwrap();
    data.extend_from_slice(NOOP_ELF);
    bank.store_account(&buffer, &loader_account(data));
    buffer
}

fn store_uninitialized_program(bank: &Bank, target: &Pubkey) {
    let data = vec![0; UpgradeableLoaderState::size_of_program()];
    bank.store_account(target, &loader_account(data));
}

fn loader_account(data: Vec<u8>) -> AccountSharedData {
    AccountSharedData::from(solana_account::Account {
        lamports: Rent::default().minimum_balance(data.len()).max(1),
        data,
        owner: DEFAULT_ENTRY_OWNER.into(),
        executable: false,
        rent_epoch: 0,
    })
}

fn versioned_transaction(
    bank: &Bank,
    payer: &Keypair,
    extra_signers: &[&Keypair],
    instructions: &[Instruction],
) -> VersionedTransaction {
    let message = Message::new(instructions, Some(&payer.pubkey()));
    let signers = [&[payer], extra_signers].concat();
    VersionedTransaction::from(Transaction::new(&signers, message, bank.last_blockhash()))
}
