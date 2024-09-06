use solana_sdk::{
    account::{AccountSharedData, ReadableAccount},
    keccak::Hasher,
    pubkey::Pubkey,
};

fn hash_account(hasher: &mut Hasher, pubkey: &Pubkey, account: &AccountSharedData) {
    hasher.hashv(&[
        pubkey.as_ref(),
        account.data(),
        &account.lamports().to_le_bytes(),
        account.owner().as_ref(),
        &[if account.executable() { 1 } else { 0 }],
        &account.rent_epoch().to_le_bytes(),
    ]);
}

pub(crate) fn hash_accounts(hasher: &mut Hasher, accounts: &[(Pubkey, AccountSharedData)]) {
    accounts.iter().for_each(|(pubkey, account)| {
        hash_account(hasher, pubkey, account);
    });
}
