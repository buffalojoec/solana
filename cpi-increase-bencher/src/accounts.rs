use {solana_account::Account, solana_pubkey::Pubkey};

pub const DEFAULT_ACCOUNT_DATA_LEN: usize = 1024;
const NUM_ACCOUNTS: usize = 2;

pub fn program_elf() -> Vec<u8> {
    let dir = std::env::var("SBF_OUT_DIR").expect(
        "SBF_OUT_DIR is unset: build the program first with\n  cargo-build-sbf --manifest-path \
         programs/sbf/rust/cpi_depth/Cargo.toml --sbf-out-dir <dir>",
    );
    let path = std::path::Path::new(&dir).join("solana_sbf_rust_cpi_depth.so");
    std::fs::read(&path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

pub(crate) fn accounts(
    program_id: &Pubkey,
    elf: &[u8],
    account_data_len: usize,
) -> Vec<(Pubkey, Account)> {
    let mut accounts = vec![(
        *program_id,
        Account {
            lamports: 1,
            data: elf.to_vec(),
            owner: solana_sdk_ids::bpf_loader::id(),
            executable: true,
            rent_epoch: 0,
        },
    )];
    accounts.extend((0..NUM_ACCOUNTS).map(|_| {
        (
            Pubkey::new_unique(),
            Account {
                lamports: 1,
                data: vec![0u8; account_data_len],
                owner: *program_id,
                executable: false,
                rent_epoch: 0,
            },
        )
    }));
    accounts
}
