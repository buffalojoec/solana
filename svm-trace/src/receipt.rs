//! SVM transaction receipt.

use solana_sdk::{
    fee::FeeDetails, keccak::Hasher, transaction, transaction_context::TransactionReturnData,
};

/// An SVM transaction receipt. Captures the runtime result of a processed
/// transaction.
pub struct SVMTransactionReceipt<'a> {
    pub compute_units_consumed: &'a u64,
    pub fee_details: &'a FeeDetails,
    pub log_messages: Option<&'a Vec<String>>,
    pub return_data: Option<&'a TransactionReturnData>,
    pub status: &'a transaction::Result<()>,
}

pub fn hash_receipt(hasher: &mut Hasher, receipt: &SVMTransactionReceipt) {
    // `compute_units_consumed`
    hasher.hash(&receipt.compute_units_consumed.to_le_bytes());

    // `fee_details`
    hasher.hashv(&[
        &receipt.fee_details.transaction_fee().to_le_bytes(),
        &receipt.fee_details.prioritization_fee().to_le_bytes(),
        // TODO: `remove_rounding_in_fee_calculation` omitted.
    ]);

    // `log_messages`
    if let Some(messages) = receipt.log_messages {
        for m in messages {
            hasher.hash(m.as_bytes());
        }
    }

    // `return_data`
    if let Some(data) = receipt.return_data {
        hasher.hashv(&[data.program_id.as_ref(), &data.data]);
    }

    // `status`
    hasher.hash(&[match receipt.status {
        Ok(()) => 0,
        Err(_) => 1, // TODO: Error codes. Just need to do some integer conversions.
    }]);
}
