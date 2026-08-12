#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::missing_safety_doc)]

use {
    core::slice,
    solana_transaction_context::{
        instruction::InstructionFrame,
        instruction_accounts::InstructionAccount,
        transaction::TransactionFrame,
        transaction_accounts::AccountSharedFields,
        vm_addresses::{
            ACCOUNT_METADATA_AREA, GUEST_INSTRUCTION_ACCOUNT_BASE_ADDRESS,
            GUEST_INSTRUCTION_DATA_BASE_ADDRESS, GUEST_REGION_SIZE, TRANSACTION_FRAME_ADDRESS,
        },
    },
    std::{alloc::Layout, ptr::null_mut},
};

#[global_allocator]
static A: BumpAllocator =
    unsafe { BumpAllocator::with_fixed_address_range(0x300000000, 32 * 1024) };

pub struct BumpAllocator {
    start: usize,
    len: usize,
}

impl BumpAllocator {
    #[inline]
    #[allow(clippy::arithmetic_side_effects)]
    pub unsafe fn new(arena: &mut [u8]) -> Self {
        debug_assert!(
            arena.len() > size_of::<usize>(),
            "Arena should be larger than usize"
        );

        // create a pointer to the start of the arena
        // that will hold an address of the byte following free space
        let pos_ptr = arena.as_mut_ptr() as *mut usize;
        // initialize the data there
        *pos_ptr = pos_ptr as usize + arena.len();

        Self {
            start: pos_ptr as usize,
            len: arena.len(),
        }
    }

    pub const unsafe fn with_fixed_address_range(start: usize, len: usize) -> Self {
        Self { start, len }
    }
}

#[allow(clippy::arithmetic_side_effects)]
unsafe impl std::alloc::GlobalAlloc for BumpAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pos_ptr = self.start as *mut usize;
        let mut pos = *pos_ptr;
        if pos == 0 {
            // First time, set starting position
            pos = self.start + self.len;
        }
        pos = pos.saturating_sub(layout.size());
        pos &= !(layout.align().wrapping_sub(1));
        if pos < self.start + size_of::<*mut u8>() {
            return null_mut();
        }
        *pos_ptr = pos;
        pos as *mut u8
    }
    #[inline]
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {
        // I'm a bump allocator, I don't free
    }
}

fn sol_log(message: &[u8]) {
    unsafe {
        let syscall: extern "C" fn(*const u8, u64) = core::mem::transmute(544561597u64); // murmur32 hash of "sol_log_"
        syscall(message.as_ptr(), message.len() as u64)
    }
}

#[unsafe(no_mangle)]
extern "C" fn custom_panic(info: &core::panic::PanicInfo<'_>) {
    let formatted = format!("{info:?}");
    sol_log(formatted.as_bytes());
}

fn set_buffer_length(base_address: u64, new_length: u64) -> u64 {
    unsafe {
        let syscall: extern "C" fn(u64, u64, u64, u64, u64) -> u64 =
            core::mem::transmute(0x713026f5u64);
        syscall(base_address, new_length, 0, 0, 0)
    }
}

fn invoke_cpi(program_idx: u64, signer_seeds_ptr: u64, signer_seeds_len: u64) {
    unsafe {
        let syscall: extern "C" fn(u64, u64, u64) = core::mem::transmute(2722332484u64);
        syscall(program_idx, signer_seeds_ptr, signer_seeds_len);
    }
}

unsafe fn perform_checks_inside_cpi(
    tx_frame: &mut TransactionFrame,
    current_ix: &InstructionFrame,
) {
    let ix_accounts = current_ix.instruction_accounts.as_slice();

    assert_eq!(current_ix.nesting_level, 1);

    // All accounts are supposed to be readonly
    for account in ix_accounts.iter() {
        assert!(!account.is_writable());
    }

    assert_eq!(
        current_ix.instruction_data.as_slice().get(1..).unwrap(),
        b"Hello!"
    );

    assert_eq!(current_ix.nesting_level, 1);
    assert_eq!(current_ix.index_of_caller_instruction, 0);

    assert_eq!(
        tx_frame.cpi_data_scratchpad.ptr(),
        current_ix
            .instruction_data
            .ptr()
            .saturating_add(GUEST_REGION_SIZE)
    );
    assert_eq!(
        tx_frame.cpi_accounts_scratchpad.ptr(),
        current_ix
            .instruction_accounts
            .ptr()
            .saturating_add(GUEST_REGION_SIZE)
    );

    // Let's write something to return data
    let return_string = b"Hi!";
    set_buffer_length(
        tx_frame.return_data_scratchpad.ptr(),
        return_string.len() as u64,
    );
    assert_eq!(
        tx_frame.return_data_scratchpad.len(),
        return_string.len() as u64
    );
    let return_data_buffer_mut = tx_frame.return_data_scratchpad.as_slice_mut();
    return_data_buffer_mut.copy_from_slice(return_string);
}

unsafe fn basic_cpi_test(
    tx_frame: &mut TransactionFrame,
    tx_accounts_metadata: &[AccountSharedFields],
    ix_accounts: &[InstructionAccount],
) {
    // Prepare CPI data
    let cpi_data = b"Hello!";
    set_buffer_length(
        tx_frame.cpi_data_scratchpad.ptr(),
        cpi_data.len().saturating_add(1) as u64,
    );
    let cpi_data_scratchpad_mut = tx_frame.cpi_data_scratchpad.as_slice_mut();
    assert_eq!(
        cpi_data_scratchpad_mut.len(),
        cpi_data.len().saturating_add(1)
    );
    *cpi_data_scratchpad_mut.get_mut(0).unwrap() = 3;
    cpi_data_scratchpad_mut
        .get_mut(1..)
        .unwrap()
        .copy_from_slice(cpi_data);

    // Ensure all accounts are writable (except the first which is the program to be called),
    // so we can see if we restrict visibility in CPI
    for account in ix_accounts.iter().skip(1) {
        assert!(account.is_writable());
    }

    // Prepare CPI accounts
    set_buffer_length(
        tx_frame.cpi_accounts_scratchpad.ptr(),
        size_of::<InstructionAccount>().saturating_mul(2) as u64,
    );
    let cpi_accounts_scratchpad_mut = tx_frame.cpi_accounts_scratchpad.as_slice_mut();
    assert_eq!(cpi_accounts_scratchpad_mut.len(), 2);
    let acc_0 = cpi_accounts_scratchpad_mut.get_unchecked_mut(0);
    *acc_0 = *ix_accounts.get(1).unwrap();
    acc_0.set_is_writable(false);

    let acc_1 = cpi_accounts_scratchpad_mut.get_unchecked_mut(1);
    *acc_1 = *ix_accounts.get(2).unwrap();
    acc_1.set_is_writable(false);

    let callee_program = ix_accounts.get_unchecked(0).index_in_transaction;
    invoke_cpi(callee_program as u64, 0, 0);

    // Checks after CPI
    assert_eq!(tx_frame.return_data_scratchpad.as_slice(), b"Hi!");
    assert_eq!(
        tx_frame.return_data_pubkey,
        tx_accounts_metadata
            .get_unchecked(callee_program as usize)
            .key
    );
}

unsafe fn cpi_into_v1(
    tx_frame: &mut TransactionFrame,
    tx_accounts_metadata: &[AccountSharedFields],
    ix_accounts: &[InstructionAccount],
) {
    set_buffer_length(tx_frame.cpi_data_scratchpad.ptr(), 33);
    let cpi_data = tx_frame.cpi_data_scratchpad.as_slice_mut();

    let program_to_call = ix_accounts.first().unwrap().index_in_transaction;
    let program_id = tx_accounts_metadata
        .get(program_to_call as usize)
        .unwrap()
        .key;

    *cpi_data.get_unchecked_mut(0) = 0;
    cpi_data
        .get_unchecked_mut(1..33)
        .copy_from_slice(program_id.as_ref());

    let callee_owned_account = ix_accounts.get(1).unwrap().index_in_transaction;
    assert_eq!(
        tx_accounts_metadata
            .get(callee_owned_account as usize)
            .unwrap()
            .owner,
        program_id
    );
    assert_eq!(
        tx_accounts_metadata
            .get(callee_owned_account as usize)
            .unwrap()
            .payload
            .len(),
        0
    );

    set_buffer_length(
        tx_frame.cpi_accounts_scratchpad.ptr(),
        2u64.saturating_mul(size_of::<InstructionAccount>() as u64),
    );
    let cpi_accounts = tx_frame.cpi_accounts_scratchpad.as_slice_mut();

    *cpi_accounts.get_unchecked_mut(0) = *ix_accounts.get(1).unwrap();
    *cpi_accounts.get_unchecked_mut(1) = *ix_accounts.get(2).unwrap();

    invoke_cpi(program_to_call as u64, 0, 0);

    // Checks after CPI
    let written_account = tx_accounts_metadata
        .get(callee_owned_account as usize)
        .unwrap();

    assert_eq!(written_account.payload.as_slice(), b"Hello from ABIv1");

    assert_eq!(tx_frame.return_data_pubkey, program_id);

    let return_data = tx_frame.return_data_scratchpad.as_slice();
    assert_eq!(return_data, b"ABIv1 return");
}

unsafe fn cpi_into_v1_and_then_v2(
    tx_frame: &mut TransactionFrame,
    tx_accounts_metadata: &[AccountSharedFields],
    ix_accounts: &[InstructionAccount],
) {
    let program_to_call = ix_accounts.first().unwrap().index_in_transaction;

    set_buffer_length(tx_frame.cpi_data_scratchpad.ptr(), 1);
    *tx_frame
        .cpi_data_scratchpad
        .as_slice_mut()
        .get_unchecked_mut(0) = 1;

    set_buffer_length(
        tx_frame.cpi_accounts_scratchpad.ptr(),
        3u64.saturating_mul(size_of::<InstructionAccount>() as u64),
    );

    let accounts_for_cpi = ix_accounts.get(1..4).unwrap();
    tx_frame
        .cpi_accounts_scratchpad
        .as_slice_mut()
        .copy_from_slice(accounts_for_cpi);

    invoke_cpi(program_to_call as u64, 0, 0);

    // Checks after CPI
    assert_eq!(
        tx_frame.cpi_data_scratchpad.ptr(),
        GUEST_INSTRUCTION_DATA_BASE_ADDRESS.saturating_add(
            GUEST_REGION_SIZE.saturating_mul(tx_frame.total_number_of_instructions_in_trace as u64)
        )
    );
    assert_eq!(
        tx_frame.cpi_accounts_scratchpad.ptr(),
        GUEST_INSTRUCTION_ACCOUNT_BASE_ADDRESS.saturating_add(
            GUEST_REGION_SIZE.saturating_mul(tx_frame.total_number_of_instructions_in_trace as u64)
        )
    );

    let account_to_check = ix_accounts.get(3).unwrap().index_in_transaction;
    let tx_account_to_check = tx_accounts_metadata.get(account_to_check as usize).unwrap();
    assert_eq!(
        tx_account_to_check.payload.as_slice(),
        b"Hello from ABIv2 - level 2"
    );
}

unsafe fn second_level_cpi(
    tx_frame: &mut TransactionFrame,
    current_ix: &InstructionFrame,
    ix_accounts: &[InstructionAccount],
    ix_payload: &[u8],
    tx_accounts_metadata: &mut [AccountSharedFields],
) {
    assert_eq!(current_ix.nesting_level, 2);
    assert_eq!(ix_payload.get(1..).unwrap(), b"Hello from the other side");

    let second_account_idx = ix_accounts.get(1).unwrap().index_in_transaction;
    let tx_account = tx_accounts_metadata
        .get_mut(second_account_idx as usize)
        .unwrap();

    let data_to_write = b"Hello from ABIv2 - level 2";
    set_buffer_length(tx_account.payload.ptr(), data_to_write.len() as u64);
    tx_account
        .payload
        .as_slice_mut()
        .copy_from_slice(data_to_write);

    let return_to_write = b"ABIv2 return";
    set_buffer_length(
        tx_frame.return_data_scratchpad.ptr(),
        return_to_write.len() as u64,
    );
    tx_frame
        .return_data_scratchpad
        .as_slice_mut()
        .copy_from_slice(return_to_write);
}

#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn entrypoint(
    ix_metadata: u64,
    ix_accounts_ptr: u64,
    ix_accounts_len: u64,
    ix_payload_ptr: u64,
    ix_payload_len: u64,
) -> u64 {
    // Transaction frame
    let tx_frame_ptr = TRANSACTION_FRAME_ADDRESS as *mut TransactionFrame;
    let tx_frame = &mut *tx_frame_ptr;

    let current_ix = &*(ix_metadata as *const InstructionFrame);
    let ix_accounts = slice::from_raw_parts(
        ix_accounts_ptr as *const InstructionAccount,
        ix_accounts_len as usize,
    );
    let ix_payload = slice::from_raw_parts(ix_payload_ptr as *const u8, ix_payload_len as usize);

    let tx_accounts_metadata = slice::from_raw_parts_mut(
        ACCOUNT_METADATA_AREA as *mut AccountSharedFields,
        tx_frame.number_of_transaction_accounts as usize,
    );

    match *current_ix.instruction_data.as_slice().first().unwrap() {
        0 => basic_cpi_test(tx_frame, tx_accounts_metadata, ix_accounts),
        1 => cpi_into_v1(tx_frame, tx_accounts_metadata, ix_accounts),
        2 => second_level_cpi(
            tx_frame,
            current_ix,
            ix_accounts,
            ix_payload,
            tx_accounts_metadata,
        ),
        3 => perform_checks_inside_cpi(tx_frame, current_ix),
        4 => cpi_into_v1_and_then_v2(tx_frame, tx_accounts_metadata, ix_accounts),
        _ => panic!("Unexpected case"),
    }

    0
}
