//! The most recent hashes of a slot's parent banks.
//!
//! The _slot hashes sysvar_ provides access to the [`SlotHashes`] type.
//!
//! The [`Sysvar::from_account_info`] and [`Sysvar::get`] methods always return
//! [`ProgramError::UnsupportedSysvar`] because this sysvar account is too large
//! to process on-chain. Thus this sysvar cannot be accessed on chain, though
//! one can still use the [`SysvarId::id`], [`SysvarId::check_id`] and
//! [`Sysvar::size_of`] methods in an on-chain program, and it can be accessed
//! off-chain through RPC.
//!
//! [`SysvarId::id`]: crate::sysvar::SysvarId::id
//! [`SysvarId::check_id`]: crate::sysvar::SysvarId::check_id
//!
//! # Examples
//!
//! Calling via the RPC client:
//!
//! ```
//! # use solana_program::example_mocks::solana_sdk;
//! # use solana_program::example_mocks::solana_rpc_client;
//! # use solana_sdk::account::Account;
//! # use solana_rpc_client::rpc_client::RpcClient;
//! # use solana_sdk::sysvar::slot_hashes::{self, SlotHashes};
//! # use anyhow::Result;
//! #
//! fn print_sysvar_slot_hashes(client: &RpcClient) -> Result<()> {
//! #   client.set_get_account_response(slot_hashes::ID, Account {
//! #       lamports: 1009200,
//! #       data: vec![1, 0, 0, 0, 0, 0, 0, 0, 86, 190, 235, 7, 0, 0, 0, 0, 133, 242, 94, 158, 223, 253, 207, 184, 227, 194, 235, 27, 176, 98, 73, 3, 175, 201, 224, 111, 21, 65, 73, 27, 137, 73, 229, 19, 255, 192, 193, 126],
//! #       owner: solana_sdk::system_program::ID,
//! #       executable: false,
//! #       rent_epoch: 307,
//! # });
//! #
//!     let slot_hashes = client.get_account(&slot_hashes::ID)?;
//!     let data: SlotHashes = bincode::deserialize(&slot_hashes.data)?;
//!
//!     Ok(())
//! }
//! #
//! # let client = RpcClient::new(String::new());
//! # print_sysvar_slot_hashes(&client)?;
//! #
//! # Ok::<(), anyhow::Error>(())
//! ```

pub use crate::slot_hashes::SlotHashes;
use crate::{
    account_info::AccountInfo,
    clock::Slot,
    hash::{Hash, HASH_BYTES},
    program_error::ProgramError,
    slot_hashes::{SlotHash, MAX_ENTRIES},
    sysvar::{get_sysvar, Sysvar, SysvarId},
};

crate::declare_sysvar_id!("SysvarS1otHashes111111111111111111111111111", SlotHashes);

impl Sysvar for SlotHashes {
    // override
    fn size_of() -> usize {
        // hard-coded so that we don't have to construct an empty
        20_488 // golden, update if MAX_ENTRIES changes
    }
    fn from_account_info(_account_info: &AccountInfo) -> Result<Self, ProgramError> {
        // This sysvar is too large to bincode::deserialize in-program
        Err(ProgramError::UnsupportedSysvar)
    }
}

/// Trait for querying the `SlotHashes` sysvar.
pub trait SlotHashesSysvar {
    /// Get a value from the sysvar entries by its key.
    /// Returns `None` if the key is not found.
    fn get(key: &Slot) -> Result<Option<Hash>, ProgramError> {
        get_slot_hash_bytes_with_position(key)
            .map(|result| result.map(|(_, bytes)| Hash::new(&bytes[8..8 + HASH_BYTES])))
    }

    /// Get the position of an entry in the sysvar by its key.
    /// Returns `None` if the key is not found.
    fn position(key: &Slot) -> Result<Option<usize>, ProgramError> {
        get_slot_hash_bytes_with_position(key).map(|result| result.map(|(position, _)| position))
    }
}

impl SlotHashesSysvar for SlotHashes {}

fn get_slot_hash_bytes_with_position<'a>(
    slot: &Slot,
) -> Result<Option<(usize, &'a [u8])>, ProgramError> {
    // Slot hashes is sorted largest -> smallest slot, so we can leverage
    // this to perform the search.
    //
    // `SlotHashes` can have skipped slots, so we'll have to implement a
    // binary search over data from `sol_get_sysvar`.
    let key = slot.to_le_bytes();

    // Rust's `serde::Serialize` will serialize a `usize` as a `u64` on 64-bit
    // systems for vector length prefixes.
    let start_offset = std::mem::size_of::<u64>();
    let length = SlotHashes::size_of().saturating_sub(start_offset);
    let entry_size = std::mem::size_of::<SlotHash>();

    let data = get_sysvar(&SlotHashes::id(), start_offset as u64, length as u64)?;

    let mut low: usize = 0;
    let mut high: usize = MAX_ENTRIES.saturating_sub(1);
    while low <= high {
        let mid = low.saturating_add(high).div_euclid(2);
        let offset = mid.saturating_mul(entry_size);
        let end = offset.saturating_add(entry_size);

        let entry_data = &data[offset..end];
        let key_data = &entry_data[..8];

        match key_data.cmp(&key) {
            std::cmp::Ordering::Equal => {
                return Ok(Some((mid, entry_data)));
            }
            std::cmp::Ordering::Greater => {
                low = mid.saturating_add(1);
            }
            std::cmp::Ordering::Less => {
                high = mid;
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{clock::Slot, hash::Hash, slot_hashes::MAX_ENTRIES},
    };

    #[test]
    fn test_size_of() {
        assert_eq!(
            SlotHashes::size_of(),
            bincode::serialized_size(
                &(0..MAX_ENTRIES)
                    .map(|slot| (slot as Slot, Hash::default()))
                    .collect::<SlotHashes>()
            )
            .unwrap() as usize
        );
    }
}
