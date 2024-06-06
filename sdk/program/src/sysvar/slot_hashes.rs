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
use {
    crate::{
        account_info::AccountInfo,
        clock::Slot,
        hash::Hash,
        program_error::ProgramError,
        sysvar::{get_sysvar, Sysvar, SysvarId},
    },
    bytemuck::{Pod, Zeroable},
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

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C)]
struct PodSlotHash {
    slot: Slot,
    hash: Hash,
}

/// Trait for querying the `SlotHashes` sysvar.
pub trait SlotHashesSysvar {
    /// Get a value from the sysvar entries by its key.
    /// Returns `None` if the key is not found.
    fn get(slot: &Slot) -> Result<Option<Hash>, ProgramError> {
        let data_len = SlotHashes::size_of();
        let mut data = vec![0u8; data_len];
        get_sysvar(&mut data, &SlotHashes::id(), 0, data_len as u64)?;
        let pod_hashes: &[PodSlotHash] =
            bytemuck::try_cast_slice(&data[8..]).map_err(|_| ProgramError::InvalidAccountData)?;

        Ok(pod_hashes
            .binary_search_by(|PodSlotHash { slot: this, .. }| slot.cmp(this))
            .map(|idx| pod_hashes[idx].hash)
            .ok())
    }

    /// Get the position of an entry in the sysvar by its key.
    /// Returns `None` if the key is not found.
    fn position(slot: &Slot) -> Result<Option<usize>, ProgramError> {
        let data_len = SlotHashes::size_of();
        let mut data = vec![0u8; data_len];
        get_sysvar(&mut data, &SlotHashes::id(), 0, data_len as u64)?;
        let pod_hashes: &[PodSlotHash] =
            bytemuck::try_cast_slice(&data[8..]).map_err(|_| ProgramError::InvalidAccountData)?;

        Ok(pod_hashes
            .binary_search_by(|PodSlotHash { slot: this, .. }| slot.cmp(this))
            .ok())
    }
}

impl SlotHashesSysvar for SlotHashes {}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            clock::Slot,
            entrypoint::SUCCESS,
            hash::{hash, Hash},
            program_stubs::{set_syscall_stubs, SyscallStubs},
            slot_hashes::{SlotHash, MAX_ENTRIES},
        },
    };

    struct MockSlotHashesSyscall {
        slot_hashes: SlotHashes,
    }

    impl SyscallStubs for MockSlotHashesSyscall {
        #[allow(clippy::arithmetic_side_effects)]
        fn sol_get_sysvar(
            &self,
            _sysvar_id_addr: *const u8,
            var_addr: *mut u8,
            offset: u64,
            length: u64,
        ) -> u64 {
            // The syscall tests for `sol_get_sysvar` should ensure the following:
            //
            // - The provided `sysvar_id_addr` can be translated into a valid
            //   sysvar ID for a sysvar contained in the sysvar cache, of which
            //   `SlotHashes` is one.
            // - Length and memory checks on `offset` and `length`.
            //
            // Therefore this mockup can simply just unsafely use the provided
            // `offset` and `length` to copy the serialized `SlotHashes` into
            // the provided `var_addr`.
            let data = bincode::serialize(&self.slot_hashes).unwrap();
            let slice = unsafe { std::slice::from_raw_parts_mut(var_addr, length as usize) };
            slice.copy_from_slice(&data[offset as usize..(offset + length) as usize]);
            SUCCESS
        }
    }

    fn mock_get_sysvar_syscall(slot_hashes: &[SlotHash]) {
        set_syscall_stubs(Box::new(MockSlotHashesSyscall {
            slot_hashes: SlotHashes::new(slot_hashes),
        }));
    }

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

    #[test]
    fn test_slot_hashes_sysvar() {
        let mut slot_hashes = vec![];
        for i in 0..MAX_ENTRIES {
            slot_hashes.push((
                i as u64,
                hash(&[(i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8, i as u8]),
            ));
        }

        mock_get_sysvar_syscall(&slot_hashes);

        let check_slot_hashes = SlotHashes::new(&slot_hashes);

        // `get`:
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::get(&0).unwrap().as_ref(),
            check_slot_hashes.get(&0),
        );
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::get(&256)
                .unwrap()
                .as_ref(),
            check_slot_hashes.get(&256),
        );
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::get(&511)
                .unwrap()
                .as_ref(),
            check_slot_hashes.get(&511),
        );
        // `None`.
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::get(&600)
                .unwrap()
                .as_ref(),
            check_slot_hashes.get(&600),
        );

        // `position`:
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::position(&0).unwrap(),
            check_slot_hashes.position(&0),
        );
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::position(&256).unwrap(),
            check_slot_hashes.position(&256),
        );
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::position(&511).unwrap(),
            check_slot_hashes.position(&511),
        );
        // `None`.
        assert_eq!(
            <SlotHashes as SlotHashesSysvar>::position(&600).unwrap(),
            check_slot_hashes.position(&600),
        );
    }
}
