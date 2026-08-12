use {
    solana_account_info::AccountInfo,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::{get_return_data, invoke, set_return_data},
    },
    solana_program_entrypoint::ProgramResult,
    solana_pubkey::Pubkey,
};

solana_program_entrypoint::entrypoint_no_alloc!(entry);

fn return_to_abi_v2(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) {
    let passed_id = Pubkey::try_from(&data[1..33]).unwrap();
    assert_eq!(program_id, &passed_id);

    // Write to an account the program owns and see if it reflects in the caller
    let my_account = accounts.first().unwrap();
    assert_eq!(
        my_account.owner, program_id,
        "Program does not own the account"
    );
    let message = b"Hello from ABIv1";
    my_account.resize(message.len()).unwrap();
    my_account.data.borrow_mut().copy_from_slice(message);

    // Set return data
    let return_data = b"ABIv1 return";
    set_return_data(return_data);
}

fn cpi_into_v2(accounts: &[AccountInfo]) {
    // Prepare CPI
    let cpi_accounts = vec![
        accounts.get(1).unwrap().clone(),
        accounts.get(2).unwrap().clone(),
    ];

    let program_id = accounts.first().unwrap().key;
    let data = b"Hello from the other side";
    let mut cpi_data = vec![2u8; data.len().saturating_add(1)];
    cpi_data.get_mut(1..).unwrap().copy_from_slice(data);
    let metas = vec![
        AccountMeta::new_readonly(*accounts.get(1).unwrap().key, false),
        AccountMeta::new(*accounts.get(2).unwrap().key, false),
    ];
    let instruction = Instruction::new_with_bytes(*program_id, &cpi_data, metas);
    invoke(&instruction, &cpi_accounts).unwrap();

    // Checks after CPI
    let (caller_id, return_data) = get_return_data().unwrap();
    assert_eq!(return_data, b"ABIv2 return");
    assert_eq!(&caller_id, program_id);

    assert_eq!(
        *accounts.get(2).unwrap().data.borrow(),
        b"Hello from ABIv2 - level 2"
    );
}

// This function is called from an ABIv2 program
pub fn entry(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    // Check if my program ID is right
    let case = data[0];
    if case == 0 {
        return_to_abi_v2(program_id, accounts, data);
    } else if case == 1 {
        cpi_into_v2(accounts);
    } else {
        panic!("Unexpected case");
    }

    Ok(())
}
