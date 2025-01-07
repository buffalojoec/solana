//! An account with an address: `(Pubkey, Account)`.

use {
    super::seed_address::SeedAddress,
    crate::{error::FixtureError, proto::AcctState as ProtoAccount},
    solana_account::Account,
    solana_pubkey::Pubkey,
};

impl TryFrom<ProtoAccount> for (Pubkey, Account, Option<SeedAddress>) {
    type Error = FixtureError;

    fn try_from(value: ProtoAccount) -> Result<Self, Self::Error> {
        let ProtoAccount {
            address,
            owner,
            lamports,
            data,
            executable,
            rent_epoch,
            seed_addr,
        } = value;

        let pubkey = Pubkey::try_from(address).map_err(FixtureError::InvalidPubkeyBytes)?;
        let owner = Pubkey::try_from(owner).map_err(FixtureError::InvalidPubkeyBytes)?;

        Ok((
            pubkey,
            Account {
                data,
                executable,
                lamports,
                owner,
                rent_epoch,
            },
            seed_addr.map(Into::into),
        ))
    }
}

impl TryFrom<ProtoAccount> for (Pubkey, Account) {
    type Error = FixtureError;

    fn try_from(value: ProtoAccount) -> Result<Self, Self::Error> {
        let ProtoAccount {
            address,
            owner,
            lamports,
            data,
            executable,
            rent_epoch,
            ..
        } = value;

        let pubkey = Pubkey::try_from(address).map_err(FixtureError::InvalidPubkeyBytes)?;
        let owner = Pubkey::try_from(owner).map_err(FixtureError::InvalidPubkeyBytes)?;

        Ok((
            pubkey,
            Account {
                data,
                executable,
                lamports,
                owner,
                rent_epoch,
            },
        ))
    }
}

impl From<(Pubkey, Account, Option<SeedAddress>)> for ProtoAccount {
    fn from(value: (Pubkey, Account, Option<SeedAddress>)) -> Self {
        let Account {
            lamports,
            data,
            owner,
            executable,
            rent_epoch,
        } = value.1;

        ProtoAccount {
            address: value.0.to_bytes().to_vec(),
            owner: owner.to_bytes().to_vec(),
            lamports,
            data,
            executable,
            rent_epoch,
            seed_addr: value.2.map(Into::into),
        }
    }
}

impl From<(Pubkey, Account)> for ProtoAccount {
    fn from(value: (Pubkey, Account)) -> Self {
        let Account {
            lamports,
            data,
            owner,
            executable,
            rent_epoch,
        } = value.1;

        ProtoAccount {
            address: value.0.to_bytes().to_vec(),
            owner: owner.to_bytes().to_vec(),
            lamports,
            data,
            executable,
            rent_epoch,
            seed_addr: None,
        }
    }
}

#[cfg(feature = "serde")]
pub(crate) fn hash_proto_accounts(
    hasher: &mut solana_sdk::keccak::Hasher,
    accounts: &[ProtoAccount],
) {
    for account in accounts {
        hasher.hash(&account.address);
        hasher.hash(&account.owner);
        hasher.hash(&account.lamports.to_le_bytes());
        hasher.hash(&account.data);
        hasher.hash(&[account.executable as u8]);
        hasher.hash(&account.rent_epoch.to_le_bytes());
        if let Some(seed_addr) = &account.seed_addr {
            hasher.hash(&seed_addr.base);
            hasher.hash(&seed_addr.seed);
            hasher.hash(&seed_addr.owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_to_from_acct_state(acct_state: (Pubkey, Account, Option<SeedAddress>)) {
        let proto = ProtoAccount::from(acct_state.clone());
        let accs = <(Pubkey, Account, Option<SeedAddress>)>::try_from(proto).unwrap();
        assert_eq!(acct_state, accs);
    }

    #[test]
    fn test_conversion_acct_state() {
        let pubkey = Pubkey::new_unique();
        let account = Account::new(0, 0, &Pubkey::new_unique());
        let seed_addr = None;
        test_to_from_acct_state((pubkey, account, seed_addr));

        let pubkey = Pubkey::new_from_array([3; 32]);
        let account = Account::new(100_000, 45, &Pubkey::new_from_array([6; 32]));
        let seed_addr = Some(SeedAddress {
            base: vec![1, 2, 3, 4],
            seed: vec![5, 6, 7, 8],
            owner: vec![9, 10, 11, 12],
        });
        test_to_from_acct_state((pubkey, account, seed_addr));

        let pubkey = Pubkey::new_from_array([3; 32]);
        let account = Account {
            data: vec![125, 126, 127, 128],
            executable: true,
            lamports: 400_000_000,
            owner: Pubkey::new_from_array([9; 32]),
            rent_epoch: u64::MAX,
        };
        let seed_addr = Some(SeedAddress {
            base: vec![9, 10, 11, 12],
            seed: vec![5, 6, 7, 8],
            owner: vec![1, 2, 3, 4],
        });
        test_to_from_acct_state((pubkey, account, seed_addr));
    }
}
