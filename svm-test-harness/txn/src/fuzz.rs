//! FFI bindings for transaction execution fuzzing.

#![allow(clippy::missing_safety_doc)]

use {
    crate::{
        execute_txn,
        fixture::{
            proto::{TxnContext as ProtoTxnContext, TxnResult as ProtoTxnResult},
            txn_context::TxnContext,
            txn_result::txn_result_to_proto,
        },
    },
    agave_feature_set::{increase_cpi_account_info_limit, raise_cpi_nesting_limit_to_8},
    prost::Message,
    solana_compute_budget::compute_budget::ComputeBudget,
    std::{env, ffi::c_int},
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sol_compat_init(_log_level: i32) {
    unsafe {
        env::set_var("SOLANA_RAYON_THREADS", "1");
        env::set_var("RAYON_NUM_THREADS", "1");
    }
    if env::var("ENABLE_SOLANA_LOGGER").is_ok() {
        agave_logger::setup();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sol_compat_fini() {}

/// Execute a transaction from protobuf input and return protobuf output.
pub fn execute_txn_proto(input: ProtoTxnContext) -> Option<ProtoTxnResult> {
    let context = TxnContext::try_from(input).ok()?;

    // Clone the transaction before passing context to execute_txn,
    // since we need it for txn_result_to_proto.
    let transaction = context.transaction.clone();

    // Set up compute budget based on active features.
    let simd_0268_active = context
        .feature_set
        .is_active(&raise_cpi_nesting_limit_to_8::id());
    let simd_0339_active = context
        .feature_set
        .is_active(&increase_cpi_account_info_limit::id());
    let compute_budget = ComputeBudget::new_with_defaults(simd_0268_active, simd_0339_active);

    let result = execute_txn(context, &compute_budget, None)?;

    Some(txn_result_to_proto(result, &transaction))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sol_compat_txn_execute_v1(
    out_ptr: *mut u8,
    out_psz: *mut u64,
    in_ptr: *mut u8,
    in_sz: u64,
) -> c_int {
    if in_ptr.is_null() || in_sz == 0 {
        return 0;
    }

    let in_slice = unsafe { std::slice::from_raw_parts(in_ptr, in_sz as usize) };
    let Ok(input) = ProtoTxnContext::decode(in_slice) else {
        return 0;
    };

    let Some(output) = execute_txn_proto(input) else {
        return 0;
    };

    let out_slice = unsafe { std::slice::from_raw_parts_mut(out_ptr, (*out_psz) as usize) };
    let out_vec = output.encode_to_vec();
    if out_vec.len() > out_slice.len() {
        return 0;
    }

    out_slice[..out_vec.len()].copy_from_slice(&out_vec);
    unsafe {
        *out_psz = out_vec.len() as u64;
    }
    1
}
