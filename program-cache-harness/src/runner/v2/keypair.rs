//! Deterministic keypair.

use solana_keypair::Keypair;

pub fn keypair(seed: u8) -> Keypair {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    bytes[31] = 1;
    Keypair::new_from_array(bytes)
}
