//! A type to hold data for the [`SlotHashes` sysvar][sv].
//!
//! [sv]: https://docs.solanalabs.com/runtime/sysvars#slothashes
//!
//! The sysvar ID is declared in [`sysvar::slot_hashes`].
//!
//! [`sysvar::slot_hashes`]: crate::sysvar::slot_hashes

pub use crate::clock::Slot;
use {
    crate::{hash::Hash, program_error::ProgramError},
    bytemuck_derive::{Pod, Zeroable},
    serde::{Deserialize, Deserializer, Serialize, Serializer},
    std::{
        iter::FromIterator,
        ops::Deref,
        sync::atomic::{AtomicUsize, Ordering},
    },
};

const U64_SIZE: usize = std::mem::size_of::<u64>();

pub const MAX_ENTRIES: usize = 512; // about 2.5 minutes to get your vote in

// This is to allow tests with custom slot hash expiry to avoid having to generate
// 512 blocks for such tests.
static NUM_ENTRIES: AtomicUsize = AtomicUsize::new(MAX_ENTRIES);

pub fn get_entries() -> usize {
    NUM_ENTRIES.load(Ordering::Relaxed)
}

pub fn set_entries_for_tests_only(entries: usize) {
    NUM_ENTRIES.store(entries, Ordering::Relaxed);
}

pub type SlotHash = (Slot, Hash);

#[repr(C)]
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Default)]
pub struct SlotHashes(Vec<SlotHash>);

impl SlotHashes {
    pub fn add(&mut self, slot: Slot, hash: Hash) {
        match self.binary_search_by(|(probe, _)| slot.cmp(probe)) {
            Ok(index) => (self.0)[index] = (slot, hash),
            Err(index) => (self.0).insert(index, (slot, hash)),
        }
        (self.0).truncate(get_entries());
    }
    pub fn position(&self, slot: &Slot) -> Option<usize> {
        self.binary_search_by(|(probe, _)| slot.cmp(probe)).ok()
    }
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn get(&self, slot: &Slot) -> Option<&Hash> {
        self.binary_search_by(|(probe, _)| slot.cmp(probe))
            .ok()
            .map(|index| &self[index].1)
    }
    pub fn new(slot_hashes: &[SlotHash]) -> Self {
        let mut slot_hashes = slot_hashes.to_vec();
        slot_hashes.sort_by(|(a, _), (b, _)| b.cmp(a));
        Self(slot_hashes)
    }
    pub fn slot_hashes(&self) -> &[SlotHash] {
        &self.0
    }
}

impl FromIterator<(Slot, Hash)> for SlotHashes {
    fn from_iter<I: IntoIterator<Item = (Slot, Hash)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Deref for SlotHashes {
    type Target = Vec<SlotHash>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A bytemuck-compatible (plain old data) version of `SlotHash`.
#[derive(Copy, Clone, Default, Pod, Zeroable)]
#[repr(C)]
pub struct PodSlotHash {
    pub slot: Slot,
    pub hash: Hash,
}

/// API for querying of the `SlotHashes` sysvar by on-chain programs.
///
/// Hangs onto the allocated raw buffer from the account data, which can be
/// queried or accessed directly as a slice of `PodSlotHash`.
#[derive(Default)]
pub struct PodSlotHashes {
    data: Vec<u8>,
    slot_hashes_start: usize,
    slot_hashes_end: usize,
}

impl PodSlotHashes {
    pub(crate) fn new(data: Vec<u8>) -> Result<Self, ProgramError> {
        // Get the number of slot hashes present in the data by reading the
        // `u64` length at the beginning of the data, the use that count to
        // calculate the length of the slot hashes data.
        //
        // The rest of the buffer is uninitialized and should not be accessed.
        let length = data
            .get(..U64_SIZE)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
            .and_then(|length| length.checked_mul(std::mem::size_of::<PodSlotHash>() as u64))
            .ok_or(ProgramError::InvalidAccountData)?;

        let slot_hashes_start = U64_SIZE;
        let slot_hashes_end = slot_hashes_start.saturating_add(length as usize);

        Ok(Self {
            data,
            slot_hashes_start,
            slot_hashes_end,
        })
    }

    /// Return the slot hashes sysvar as a vector of `PodSlotHash`.
    pub fn as_slice(&self) -> Result<&[PodSlotHash], ProgramError> {
        self.data
            .get(self.slot_hashes_start..self.slot_hashes_end)
            .and_then(|data| bytemuck::try_cast_slice(data).ok())
            .ok_or(ProgramError::InvalidAccountData)
    }

    /// Get a value from the sysvar entries by its key.
    /// Returns `None` if the key is not found.
    pub fn get(&self, slot: &Slot) -> Result<Option<Hash>, ProgramError> {
        self.as_slice().map(|pod_hashes| {
            pod_hashes
                .binary_search_by(|PodSlotHash { slot: this, .. }| slot.cmp(this))
                .map(|idx| pod_hashes[idx].hash)
                .ok()
        })
    }

    /// Get the position of an entry in the sysvar by its key.
    /// Returns `None` if the key is not found.
    pub fn position(&self, slot: &Slot) -> Result<Option<usize>, ProgramError> {
        self.as_slice().map(|pod_hashes| {
            pod_hashes
                .binary_search_by(|PodSlotHash { slot: this, .. }| slot.cmp(this))
                .ok()
        })
    }
}

impl Serialize for PodSlotHashes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.data)
    }
}

impl<'de> Deserialize<'de> for PodSlotHashes {
    fn deserialize<D>(deserializer: D) -> Result<PodSlotHashes, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(PodSlotHashesVisitor)
    }
}

struct PodSlotHashesVisitor;

impl<'de> serde::de::Visitor<'de> for PodSlotHashesVisitor {
    type Value = PodSlotHashes;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a byte array")
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<PodSlotHashes, E>
    where
        E: serde::de::Error,
    {
        // Just read the raw data into a Vec<u8> without deserializing.
        PodSlotHashes::new(value.to_vec()).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::hash::hash, test_case::test_case};

    #[test]
    fn test_slot_hashes() {
        let mut slot_hashes = SlotHashes::new(&[(1, Hash::default()), (3, Hash::default())]);
        slot_hashes.add(2, Hash::default());
        assert_eq!(
            slot_hashes,
            SlotHashes(vec![
                (3, Hash::default()),
                (2, Hash::default()),
                (1, Hash::default()),
            ])
        );

        let mut slot_hashes = SlotHashes::new(&[]);
        for i in 0..MAX_ENTRIES + 1 {
            slot_hashes.add(
                i as u64,
                hash(&[(i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8, i as u8]),
            );
        }
        for i in 0..MAX_ENTRIES {
            assert_eq!(slot_hashes[i].0, (MAX_ENTRIES - i) as u64);
        }

        assert_eq!(slot_hashes.len(), MAX_ENTRIES);
    }

    #[allow(clippy::arithmetic_side_effects)]
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
    fn test_pod_slot_hashes(num_entries: usize) {
        let mut slot_hashes = vec![];
        for i in 0..num_entries {
            slot_hashes.push((
                i as u64,
                hash(&[(i >> 24) as u8, (i >> 16) as u8, (i >> 8) as u8, i as u8]),
            ));
        }

        let slot_hashes = SlotHashes::new(&slot_hashes);
        let data = bincode::serialize(&slot_hashes).unwrap();
        let pod_slot_hashes = PodSlotHashes::new(data).unwrap();

        // `get`:
        assert_eq!(
            slot_hashes.get(&0),
            pod_slot_hashes.get(&0).unwrap().as_ref(),
        );
        assert_eq!(
            slot_hashes.get(&256),
            pod_slot_hashes.get(&256).unwrap().as_ref(),
        );
        assert_eq!(
            slot_hashes.get(&511),
            pod_slot_hashes.get(&511).unwrap().as_ref(),
        );
        // `None`.
        assert_eq!(
            slot_hashes.get(&600),
            pod_slot_hashes.get(&600).unwrap().as_ref(),
        );

        // `position`:
        assert_eq!(
            slot_hashes.position(&0),
            pod_slot_hashes.position(&0).unwrap(),
        );
        assert_eq!(
            slot_hashes.position(&256),
            pod_slot_hashes.position(&256).unwrap(),
        );
        assert_eq!(
            slot_hashes.position(&511),
            pod_slot_hashes.position(&511).unwrap(),
        );
        // `None`.
        assert_eq!(
            slot_hashes.position(&600),
            pod_slot_hashes.position(&600).unwrap(),
        );
    }
}
