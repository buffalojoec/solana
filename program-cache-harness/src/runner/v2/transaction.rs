//! Transaction builders.

use {
    solana_hash::Hash,
    solana_instruction::{AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_loader_v3_interface::{instruction as loader_v3, state::UpgradeableLoaderState},
    solana_pubkey::Pubkey,
    solana_rent::Rent,
    solana_sdk_ids::bpf_loader_upgradeable,
    solana_signer::Signer,
    solana_transaction::Transaction,
};

pub fn deploy(
    payer: &Keypair,
    program: &Keypair,
    buffer: &Pubkey,
    authority: &Keypair,
    elf_len: usize,
    blockhash: Hash,
) -> Transaction {
    let max_data_len = elf_len.max(1);
    let lamports = Rent::default()
        .minimum_balance(UpgradeableLoaderState::size_of_program())
        .max(1);
    let instructions = loader_v3::deploy_with_max_program_len(
        &payer.pubkey(),
        &program.pubkey(),
        buffer,
        &authority.pubkey(),
        lamports,
        max_data_len,
    )
    .expect("build deploy instructions");
    transaction(
        &instructions,
        payer,
        &[payer, program, authority],
        blockhash,
    )
}

pub fn upgrade(
    payer: &Keypair,
    program: &Pubkey,
    buffer: &Pubkey,
    authority: &Keypair,
    blockhash: Hash,
) -> Transaction {
    let instruction = loader_v3::upgrade(program, buffer, &authority.pubkey(), &payer.pubkey());
    transaction(&[instruction], payer, &[payer, authority], blockhash)
}

pub fn close(
    payer: &Keypair,
    program: &Pubkey,
    authority: &Keypair,
    blockhash: Hash,
) -> Transaction {
    let programdata =
        Pubkey::find_program_address(&[program.as_ref()], &bpf_loader_upgradeable::id()).0;
    let instruction = loader_v3::close_any(
        &programdata,
        &payer.pubkey(),
        Some(&authority.pubkey()),
        Some(program),
    );
    transaction(&[instruction], payer, &[payer, authority], blockhash)
}

pub fn invoke(payer: &Keypair, programs: &[Pubkey], blockhash: Hash) -> Transaction {
    let instructions: Vec<Instruction> = programs
        .iter()
        .map(|program| {
            Instruction::new_with_bytes(*program, &[], vec![AccountMeta::new(payer.pubkey(), true)])
        })
        .collect();
    transaction(&instructions, payer, &[payer], blockhash)
}

fn transaction(
    instructions: &[Instruction],
    payer: &Keypair,
    signers: &[&Keypair],
    blockhash: Hash,
) -> Transaction {
    Transaction::new_signed_with_payer(instructions, Some(&payer.pubkey()), signers, blockhash)
}

#[cfg(test)]
mod tests {
    use {
        super::{
            super::{account, forks::Forks, keypair::keypair},
            *,
        },
        crate::{elf::NOOP_OK, scenario::tree},
        solana_program_runtime::program_cache_entry::ProgramCacheEntryType,
        solana_runtime::bank::Bank,
        solana_svm::{
            transaction_error_metrics::TransactionErrorMetrics,
            transaction_processor::TransactionProcessingConfig,
        },
        solana_svm_timings::ExecuteTimings,
    };

    fn deploy_at(forks: &Forks, bank: &Bank, program: &Keypair) {
        let payer = forks.payer().insecure_clone();
        let buffer = keypair(200);
        bank.store_account(&buffer.pubkey(), &account::buffer(&payer.pubkey(), NOOP_OK));
        let transaction = deploy(
            &payer,
            program,
            &buffer.pubkey(),
            &payer,
            NOOP_OK.len(),
            bank.last_blockhash(),
        );
        bank.process_transaction(&transaction)
            .expect("the deploy lands");
    }

    #[test]
    fn a_real_deployment_fills_the_cache() {
        let forks = Forks::new(&tree(&[&[1, 2]]));
        let program = keypair(7);
        let deploying = forks.bank(1).expect("a bank at slot 1");
        deploy_at(&forks, &deploying, &program);

        // The next slot is past the delay visibility window.
        let running = forks.bank(2).expect("a bank at slot 2");
        let payer = forks.payer().insecure_clone();
        let transaction = invoke(&payer, &[program.pubkey()], running.last_blockhash());
        let batch = running.prepare_batch_for_tests(vec![transaction]);
        let output = running.load_and_execute_transactions(
            &batch,
            usize::MAX,
            &mut ExecuteTimings::default(),
            &mut TransactionErrorMetrics::default(),
            TransactionProcessingConfig::default(),
        );

        let extracted = output.program_cache_for_tx_batch;
        let entry = extracted
            .find(&program.pubkey())
            .expect("the batch was served the program");
        assert!(
            matches!(entry.program, ProgramCacheEntryType::Loaded(_)),
            "and it is executable"
        );
        assert_eq!(
            extracted.loaded_keys,
            vec![program.pubkey()],
            "the first use loads it"
        );
    }
}
