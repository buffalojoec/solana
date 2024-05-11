use {
    solana_program_test::ProgramTest,
    solana_sdk::{
        bpf_loader_upgradeable, feature, instruction::Instruction, signature::Signer,
        transaction::Transaction,
    },
};

#[tokio::test]
async fn core_bpf_program() {
    let program_id = solana_sdk::config::program::id();
    let feature_id = solana_runtime::bank::builtins::test_only::config_program::feature::id();
    let buffer_address =
        solana_runtime::bank::builtins::test_only::config_program::source_buffer::id();

    std::env::set_var("SBF_OUT_DIR", "../programs/bpf_loader/test_elfs/out");

    let mut program_test = ProgramTest::default();
    program_test.prefer_bpf(true);
    program_test.add_program("noop_aligned", program_id, None);

    let mut context = program_test.start_with_context().await;

    // Assert the feature is active.
    let feature_account = context
        .banks_client
        .get_account(feature_id)
        .await
        .unwrap()
        .unwrap();
    let feature_account_state = feature::from_account(&feature_account).unwrap();
    assert_eq!(
        feature_account_state,
        feature::Feature {
            activated_at: Some(0)
        }
    );

    // Assert the source buffer does not exist.
    let buffer_account = context
        .banks_client
        .get_account(buffer_address)
        .await
        .unwrap();
    assert!(buffer_account.is_none());

    // Assert the program is an upgradeable BPF program.
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
