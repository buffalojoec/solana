//! Shared constants for the `squads` proxy authority program: the address it's
//! deployed at, and the PDA of it that holds another program's upgrade
//! authority.

use solana_pubkey::{Pubkey, pubkey};

pub const ID: Pubkey = pubkey!("SquadsProxy11111111111111111111111111111111");
pub const AUTHORITY_SEED: &[u8] = b"squads";
pub const AUTHORITY: Pubkey = pubkey!("7SvUfGML7Zin2R6itP42purz6wSRQ9JmcXE2bJCYtaib");
pub const AUTHORITY_BUMP: u8 = 254;
