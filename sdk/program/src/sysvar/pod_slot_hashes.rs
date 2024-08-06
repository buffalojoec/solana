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

pub use crate::slot_hashes::PodSlotHashes;
use crate::{
    program_error::ProgramError,
    sysvar::{get_sysvar, Sysvar, SysvarId},
};

crate::declare_sysvar_id!("SysvarS1otHashes111111111111111111111111111", PodSlotHashes);

impl Sysvar for PodSlotHashes {
    fn size_of() -> usize {
        // Kept in sync with `SlotHashes`.
        crate::slot_hashes::SlotHashes::size_of()
    }

    fn get() -> Result<Self, ProgramError> {
        let sysvar_len = Self::size_of();
        let mut data = vec![0; sysvar_len];
        get_sysvar(
            &mut data,
            &Self::id(),
            /* offset */ 0,
            /* length */ sysvar_len as u64,
        )?;
        PodSlotHashes::new(data)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            clock::Slot,
            hash::{hash, Hash},
            slot_hashes::{SlotHashes, MAX_ENTRIES},
            sysvar::tests::mock_get_sysvar_syscall,
        },
        serial_test::serial,
        test_case::test_case,
    };

    #[test]
    fn test_size_of() {
        assert_eq!(
            PodSlotHashes::size_of(),
            bincode::serialized_size(
                &(0..MAX_ENTRIES)
                    .map(|slot| (slot as Slot, Hash::default()))
                    .collect::<SlotHashes>()
            )
            .unwrap() as usize
        );
    }

    #[test_case(0)]
    #[test_case(1)]
    #[test_case(2)]
    #[test_case(5)]
    #[test_case(10)]
    #[test_case(64)]
    #[test_case(128)]
    #[test_case(192)]
    #[test_case(256)]
    #[test_case(384)]
    #[test_case(MAX_ENTRIES)]
    #[serial]
    fn test_pod_slot_hashes_sysvar(num_entries: usize) {
        let mut slot_hashes = vec![];
        for i in 0..num_entries {
            slot_hashes.push((
                i as u64,
                hash(&[(i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8, i as u8]),
            ));
        }

        let check_slot_hashes = SlotHashes::new(&slot_hashes);
        mock_get_sysvar_syscall(&bincode::serialize(&check_slot_hashes).unwrap());

        let pod_slot_hashes = <PodSlotHashes as Sysvar>::get().unwrap();
        let pod_slot_hashes_slice = pod_slot_hashes.as_slice().unwrap();

        assert_eq!(pod_slot_hashes_slice.len(), num_entries);

        for (slot_hash, pod_slot_hash) in check_slot_hashes.iter().zip(pod_slot_hashes_slice) {
            assert_eq!(slot_hash.0, pod_slot_hash.slot);
            assert_eq!(slot_hash.1, pod_slot_hash.hash);
        }
    }
}
