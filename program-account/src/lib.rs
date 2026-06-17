//! Program account ELF extraction.
//!
//! A [`ProgramAccount`] wraps the account that holds a program's ELF and vends
//! the ELF bytes, hiding the per-loader layout. It centralizes the offset and
//! owner logic that would otherwise be duplicated everywhere a program is
//! loaded from chain.

use {
    solana_account::{AccountSharedData, ReadableAccount, state_traits::StateMut},
    solana_loader_v3_interface::state::UpgradeableLoaderState,
    solana_loader_v4_interface::state::{LoaderV4State, LoaderV4Status},
    solana_pubkey::Pubkey,
    solana_sdk_ids::{
        bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable, loader_v4, native_loader,
    },
};

/// The loader that owns a program account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramOwner {
    NativeLoader,
    LoaderV1,
    LoaderV2,
    LoaderV3,
    LoaderV4,
}

/// The role of a loader-V3 account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V3ProgramType {
    /// A program account: a pointer to the programdata account that holds the
    /// ELF.
    Program { programdata_address: Pubkey },
    /// A programdata account: holds the ELF after its metadata header.
    ProgramData,
}

/// An on-chain account whose data holds a program's ELF, paired with the loader
/// that owns it.
///
/// For loaders V1, V2, and V4 the ELF lives directly in the program account.
/// For loader V3 the ELF lives in the *programdata* account, not the program
/// account that points to it - so callers must construct a [`ProgramAccount`]
/// from the programdata account in that case.
pub struct ProgramAccount<'a> {
    account: &'a AccountSharedData,
    owner: ProgramOwner,
}

impl<'a> ProgramAccount<'a> {
    /// Classify `account` by its owning loader, returning `None` if it is not
    /// owned by a known loader or its data is too small to hold the loader's
    /// header.
    ///
    /// For loader V3, `account` must be the programdata account (which contains
    /// the ELF), not the program account that points to it.
    pub fn try_new(account: &'a AccountSharedData) -> Option<Self> {
        let owner = if native_loader::check_id(account.owner()) {
            ProgramOwner::NativeLoader
        } else if bpf_loader_deprecated::check_id(account.owner()) {
            ProgramOwner::LoaderV1
        } else if bpf_loader::check_id(account.owner()) {
            ProgramOwner::LoaderV2
        } else if bpf_loader_upgradeable::check_id(account.owner()) {
            ProgramOwner::LoaderV3
        } else if loader_v4::check_id(account.owner()) {
            ProgramOwner::LoaderV4
        } else {
            return None;
        };

        // Verify the data is large enough to slice the ELF from, so
        // `get_elf_bytes` can never fail.
        let valid = match owner {
            // Built-ins are registered, not compiled, so there is no ELF to
            // validate.
            ProgramOwner::NativeLoader => true,
            ProgramOwner::LoaderV1 | ProgramOwner::LoaderV2 => true,
            ProgramOwner::LoaderV3 => {
                matches!(
                    account.state(),
                    Ok(UpgradeableLoaderState::ProgramData { .. })
                )
            }
            ProgramOwner::LoaderV4 => has_valid_loader_v4_state(account.data()),
        };

        valid.then_some(Self { account, owner })
    }

    /// Classify a loader-V3 account by role, returning `None` if it is not owned
    /// by loader V3. A program account points to the programdata account that
    /// holds the ELF; the programdata account holds the ELF itself.
    pub fn get_loader_v3_type(account: &AccountSharedData) -> Option<V3ProgramType> {
        if !bpf_loader_upgradeable::check_id(account.owner()) {
            return None;
        }
        match account.state() {
            Ok(UpgradeableLoaderState::Program {
                programdata_address,
            }) => Some(V3ProgramType::Program {
                programdata_address,
            }),
            Ok(UpgradeableLoaderState::ProgramData { .. }) => Some(V3ProgramType::ProgramData),
            _ => None,
        }
    }

    /// The loader that owns this program account.
    pub fn owner(&self) -> ProgramOwner {
        self.owner
    }

    /// The program's ELF bytes, sliced past any loader header. Built-ins carry
    /// no ELF, so they yield an empty slice.
    pub fn get_elf_bytes(&self) -> &'a [u8] {
        let offset = match self.owner {
            // No ELF to slice; a built-in is registered, not compiled.
            ProgramOwner::NativeLoader => return &[],
            ProgramOwner::LoaderV1 | ProgramOwner::LoaderV2 => 0,
            ProgramOwner::LoaderV3 => UpgradeableLoaderState::size_of_programdata_metadata(),
            ProgramOwner::LoaderV4 => LoaderV4State::program_data_offset(),
        };
        // `try_new` verified the data is long enough for this offset. The slice
        // is tied to the borrowed account, not to `self`, so it outlives a
        // temporary `ProgramAccount`.
        self.account.data().get(offset..).unwrap_or_default()
    }
}

// Loader V4 has no leading discriminator; instead its header ends with an
// 8-byte `status` field (`#[repr(u64)]`). Treat the metadata as valid when the
// data spans a full header and that status holds a known variant.
fn has_valid_loader_v4_state(data: &[u8]) -> bool {
    const STATUS_OFFSET: usize =
        LoaderV4State::program_data_offset().saturating_sub(std::mem::size_of::<u64>());
    const RETRACTED: u64 = LoaderV4Status::Retracted as u64;
    const FINALIZED: u64 = LoaderV4Status::Finalized as u64;
    data.get(STATUS_OFFSET..LoaderV4State::program_data_offset())
        .and_then(|status| <[u8; 8]>::try_from(status).ok())
        .map(u64::from_le_bytes)
        .is_some_and(|status| matches!(status, RETRACTED..=FINALIZED))
}

#[cfg(test)]
mod tests {
    use {super::*, solana_account::WritableAccount, solana_pubkey::Pubkey, std::mem::size_of};

    const ELF: &[u8] = b"\x7fELF the program bytes";

    fn account(owner: &Pubkey, data: &[u8]) -> AccountSharedData {
        let mut account = AccountSharedData::new(0, data.len(), owner);
        account.data_as_mut_slice().copy_from_slice(data);
        account
    }

    #[test]
    fn unknown_owner_is_rejected() {
        let account = account(&Pubkey::new_unique(), ELF);
        assert!(ProgramAccount::try_new(&account).is_none());
    }

    #[test]
    fn native_loader_is_a_builtin_with_no_elf() {
        let account = account(&native_loader::id(), b"my_builtin_program");
        let program = ProgramAccount::try_new(&account).unwrap();
        assert_eq!(program.owner(), ProgramOwner::NativeLoader);
        assert!(program.get_elf_bytes().is_empty());
    }

    #[test]
    fn loader_v1_and_v2_vend_data_directly() {
        for owner in [bpf_loader_deprecated::id(), bpf_loader::id()] {
            let account = account(&owner, ELF);
            let program = ProgramAccount::try_new(&account).unwrap();
            assert_eq!(program.get_elf_bytes(), ELF);
        }
    }

    #[test]
    fn loader_v3_vends_data_after_programdata_header() {
        let mut data = bincode::serialize(&UpgradeableLoaderState::ProgramData {
            slot: 0,
            upgrade_authority_address: Some(Pubkey::new_unique()),
        })
        .unwrap();
        assert_eq!(
            data.len(),
            UpgradeableLoaderState::size_of_programdata_metadata()
        );
        data.extend_from_slice(ELF);

        let account = account(&bpf_loader_upgradeable::id(), &data);
        let program = ProgramAccount::try_new(&account).unwrap();
        assert_eq!(program.owner(), ProgramOwner::LoaderV3);
        assert_eq!(program.get_elf_bytes(), ELF);
    }

    #[test]
    fn loader_v3_program_pointer_is_rejected() {
        // The program account that merely points at the programdata account has
        // the loader's owner but is not a valid ELF holder.
        let data = bincode::serialize(&UpgradeableLoaderState::Program {
            programdata_address: Pubkey::new_unique(),
        })
        .unwrap();
        let account = account(&bpf_loader_upgradeable::id(), &data);
        assert!(ProgramAccount::try_new(&account).is_none());
    }

    #[test]
    fn loader_v3_type_classifies_program_and_programdata() {
        let programdata_address = Pubkey::new_unique();
        let program = account(
            &bpf_loader_upgradeable::id(),
            &bincode::serialize(&UpgradeableLoaderState::Program {
                programdata_address,
            })
            .unwrap(),
        );
        assert_eq!(
            ProgramAccount::get_loader_v3_type(&program),
            Some(V3ProgramType::Program {
                programdata_address
            })
        );

        let programdata = account(
            &bpf_loader_upgradeable::id(),
            &bincode::serialize(&UpgradeableLoaderState::ProgramData {
                slot: 0,
                upgrade_authority_address: None,
            })
            .unwrap(),
        );
        assert_eq!(
            ProgramAccount::get_loader_v3_type(&programdata),
            Some(V3ProgramType::ProgramData)
        );
    }

    #[test]
    fn loader_v3_type_is_none_for_other_loaders() {
        let account = account(&loader_v4::id(), ELF);
        assert_eq!(ProgramAccount::get_loader_v3_type(&account), None);
    }

    // Build a V4 program account: a header carrying `status` followed by the ELF.
    fn loader_v4_account(status: u64) -> AccountSharedData {
        let mut data = vec![0u8; LoaderV4State::program_data_offset()];
        let offset = LoaderV4State::program_data_offset().saturating_sub(size_of::<u64>());
        data[offset..].copy_from_slice(&status.to_le_bytes());
        data.extend_from_slice(ELF);
        account(&loader_v4::id(), &data)
    }

    #[test]
    fn loader_v4_vends_data_after_header() {
        let account = loader_v4_account(LoaderV4Status::Deployed as u64);
        let program = ProgramAccount::try_new(&account).unwrap();
        assert_eq!(program.owner(), ProgramOwner::LoaderV4);
        assert_eq!(program.get_elf_bytes(), ELF);
    }

    #[test]
    fn loader_v4_invalid_status_is_rejected() {
        let account = loader_v4_account(42);
        assert!(ProgramAccount::try_new(&account).is_none());
    }

    #[test]
    fn loader_v4_too_small_is_rejected() {
        let account = account(&loader_v4::id(), &[0u8; 4]);
        assert!(ProgramAccount::try_new(&account).is_none());
    }
}
