use {
    solana_program_test::{
        find_file, programs::bpf_loader_upgradeable_program_accounts, read_file, ProgramTest,
    },
    solana_sdk::{
        bpf_loader_upgradeable, instruction::Instruction, rent::Rent, signature::Signer,
        transaction::Transaction,
    },
};

#[tokio::test]
async fn test_add_bpf_program() {
    std::env::set_var("SBF_OUT_DIR", "../programs/bpf_loader/test_elfs/out");

    // Core BPF program: Address Lookup Lable.
    let program_id = solana_sdk::address_lookup_table::program::id();
    let program_name = "noop_aligned";

    let program_file = find_file(&format!("{program_name}.so")).unwrap();
    let elf = read_file(&program_file);

    let program_accounts =
        bpf_loader_upgradeable_program_accounts(&program_id, &elf, &Rent::default());

    let mut program_test = ProgramTest::default();
    program_test.prefer_bpf(true);

    for (address, account) in program_accounts {
        program_test.add_genesis_account(address, account);
    }

    let mut context = program_test.start_with_context().await;

    // Assert the program is a BPF Loader Upgradeable program.
    let program_account = context
        .banks_client
        .get_account(program_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(program_account.owner, bpf_loader_upgradeable::id());

    // Invoke the program.
    let instruction = Instruction::new_with_bytes(program_id, &[], Vec::new());
    let transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&context.payer.pubkey()),
        &[&context.payer],
        context.last_blockhash,
    );
    context
        .banks_client
        .process_transaction(transaction)
        .await
        .unwrap();
}
