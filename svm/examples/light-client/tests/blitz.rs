use {
    solana_sdk::{
        account::{AccountSharedData, WritableAccount},
        feature_set::FeatureSet,
        fee::{FeeDetails, FeeStructure},
        keccak::Hasher,
        native_loader,
        pubkey::Pubkey,
        rent_collector::RentCollector,
        signature::Keypair,
        signer::Signer,
        system_instruction, system_program,
        transaction::{SanitizedTransaction, Transaction},
    },
    solana_svm_example_light_client::{blitz::Blitz, light_client::BlitzLightClient},
    solana_svm_trace::{receipt::SVMTransactionReceipt, stf::STFEnvironment},
};

const ALICE_LAMPORTS: u64 = 100_000_000_000_000_000;

#[test]
fn blitz() {
    let alice = Keypair::new();
    let bob = Pubkey::new_unique();
    let carol = Pubkey::new_unique();

    // Set up the L2 instance.
    let mut blitz = Blitz::default();
    blitz.add_accounts(&[(
        alice.pubkey(),
        AccountSharedData::new(ALICE_LAMPORTS, 0, &system_program::id()),
    )]);

    // All transactions are system transfers here, but it doesn't matter.
    // We just need to work with the proofs.
    let transactions = (0..40)
        .map(|i| {
            SanitizedTransaction::from_transaction_for_tests(Transaction::new_signed_with_payer(
                &[system_instruction::transfer(
                    &alice.pubkey(),
                    if i % 2 == 0 { &bob } else { &carol },
                    ALICE_LAMPORTS / 100_000_000 * (i as u64),
                )],
                Some(&alice.pubkey()),
                &[&alice],
                solana_sdk::hash::Hash::default(),
            ))
        })
        .collect::<Vec<_>>();

    // Process the transactions with the Blitz L2, creating a new block every
    // time the max transactions per block threshold is reached.
    blitz.process_transactions(&transactions);

    let mut hasher = Hasher::default();
    let mut light_client = BlitzLightClient::new(&blitz, &mut hasher);

    // For checks.
    let system_account_with_lamports = |lamports| {
        let mut account = AccountSharedData::new(lamports, 0, &system_program::id());
        account.set_rent_epoch(u64::MAX);
        account
    };
    let system_program_account = || {
        let mut account = AccountSharedData::new(0, 0, &native_loader::id());
        account.set_executable(true);
        account
    };

    // Select a transaction to evaluate proofs for.
    // Blitz can support 10 transactions per block, so we know the slot.
    let slot = 0;
    let transaction = &transactions[8];
    assert!(light_client.prove_transaction_inclusion(&slot, transaction));
    assert!(light_client.prove_transaction_receipt(
        &slot,
        transaction,
        &SVMTransactionReceipt {
            compute_units_consumed: &150,
            fee_details: &FeeDetails::new(5000, 0, true),
            log_messages: None,
            return_data: None,
            status: &Ok(()),
        }
    ));
    assert!(light_client.prove_transaction_stf(
        &slot,
        transaction,
        &STFEnvironment {
            feature_set: &FeatureSet::all_enabled(),
            fee_structure: Some(&FeeStructure::default()),
            lamports_per_signature: &FeeStructure::default().lamports_per_signature,
            rent_collector: Some(&RentCollector::default())
        },
        &[
            (
                alice.pubkey(),
                system_account_with_lamports(99999971999955000),
            ),
            (bob, system_account_with_lamports(12000000000)),
            (system_program::id(), system_program_account()),
        ],
        &[
            (
                alice.pubkey(),
                system_account_with_lamports(99999963999955000),
            ),
            (bob, system_account_with_lamports(20000000000)),
            (system_program::id(), system_program_account()),
        ]
    ));

    // Select another.
    let slot = 2;
    let transaction = &transactions[29];
    assert!(light_client.prove_transaction_inclusion(&slot, transaction));
    assert!(light_client.prove_transaction_receipt(
        &slot,
        transaction,
        &SVMTransactionReceipt {
            compute_units_consumed: &150,
            fee_details: &FeeDetails::new(5000, 0, true),
            log_messages: None,
            return_data: None,
            status: &Ok(()),
        }
    ));
    assert!(light_client.prove_transaction_stf(
        &slot,
        transaction,
        &STFEnvironment {
            feature_set: &FeatureSet::all_enabled(),
            fee_structure: Some(&FeeStructure::default()),
            lamports_per_signature: &FeeStructure::default().lamports_per_signature,
            rent_collector: Some(&RentCollector::default())
        },
        &[
            (
                alice.pubkey(),
                system_account_with_lamports(99999593999850000),
            ),
            (carol, system_account_with_lamports(196000000000)),
            (system_program::id(), system_program_account()),
        ],
        &[
            (
                alice.pubkey(),
                system_account_with_lamports(99999564999850000),
            ),
            (carol, system_account_with_lamports(225000000000)),
            (system_program::id(), system_program_account()),
        ]
    ));
}
