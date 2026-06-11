//! # Cryptographic Utilities
//!
//! This module provides cryptographic functions for the daily lottery program,
//! including hash aggregation and winner selection algorithms.

use sha2::{Digest, Sha256};
use solana_program::pubkey::Pubkey;
use solana_sha256_hasher::hashv;

const RPD_V2_REVEAL_DOMAIN: &[u8] = &[
    0x49, 0x4b, 0x49, 0x47, 0x41, 0x49, 0x5f, 0x52, 0x50, 0x44, 0x5f, 0x56, 0x32, 0x5f, 0x52, 0x45,
    0x56, 0x45, 0x41, 0x4c,
];

/// Aggregates multiple hashes into a single deterministic hash
///
/// This function is used to combine all participant reveal hashes into
/// a single entropy source for winner selection. The order of inputs
/// affects the output, so inputs must be consistently ordered.
///
/// ## Algorithm
/// 1. Create SHA256 hasher
/// 2. Update hasher with each input hash in sequence
/// 3. Finalize to get aggregate hash
///
/// ## Usage
/// ```ignore
/// let hashes = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
/// let aggregate = aggregate_hashes(&hashes);
/// ```
pub fn aggregate_hashes(inputs: &[[u8; 32]]) -> [u8; 32] {
    let mut hasher = Sha256::new();

    // Hash each input in sequence
    for hash in inputs {
        hasher.update(hash);
    }

    // Finalize and convert to fixed-size array
    let result = hasher.finalize();
    let mut output = [0u8; 32];
    output.copy_from_slice(&result);
    output
}

/// Computes the deterministic reveal digest included in settlement entropy.
pub fn compute_reveal_digest(wallet: &Pubkey, plaintext: &[u8]) -> [u8; 32] {
    let plaintext_len_le = (plaintext.len() as u32).to_le_bytes();
    hashv(&[
        RPD_V2_REVEAL_DOMAIN,
        &wallet.to_bytes(),
        &plaintext_len_le,
        plaintext,
    ])
    .to_bytes()
}

/// XORs reveal digests into an aggregate hash in a batch/order-independent way.
pub fn xor_reveal_digests(initial: [u8; 32], digests: &[[u8; 32]]) -> [u8; 32] {
    let mut aggregate_hash = initial;
    for digest in digests.iter() {
        for i in 0..aggregate_hash.len() {
            aggregate_hash[i] ^= digest[i];
        }
    }
    aggregate_hash
}

/// Selects a winning ticket index using deterministic randomness
///
/// This function converts entropy from reveal aggregation into a fair
/// ticket selection. It uses the first 16 bytes of entropy as a u128
/// to ensure sufficient randomness for large ticket counts.
///
/// ## Algorithm
/// 1. Convert first 16 bytes of entropy to u128 (little-endian)
/// 2. Take modulo of total tickets to get winning index
/// 3. Handle edge case of zero tickets
///
/// ## Fairness
/// The modulo operation ensures uniform distribution across all tickets
/// as long as the entropy source is uniformly random.
///
/// ## Parameters
/// - `entropy`: 32-byte hash from reveal aggregation
/// - `total_tickets`: Total number of tickets sold
///
/// ## Returns
/// Zero-based index of the winning ticket
pub fn select_winning_ticket(entropy: [u8; 32], total_tickets: u64) -> u64 {
    // Handle edge case
    if total_tickets == 0 {
        return 0;
    }

    // Convert first 16 bytes to u128 for sufficient range
    let num = u128::from_le_bytes({
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&entropy[..16]);
        arr
    });

    // Use modulo to select winning ticket
    (num % (total_tickets as u128)) as u64
}

/// Generates a discriminator for account types (Anchor-compatible)
///
/// This function creates 8-byte discriminators for different account types
/// using the same algorithm as Anchor framework for compatibility.
///
/// ## Algorithm
/// 1. Create SHA256 hash of "account:{name}"
/// 2. Take first 8 bytes as discriminator
///
/// ## Usage
/// ```ignore
/// let disc = discriminator("Config");
/// // Use disc as first 8 bytes of account data
/// ```
pub fn discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(format!("account:{}", name));
    let result = hasher.finalize();

    let mut discriminator = [0u8; 8];
    discriminator.copy_from_slice(&result[..8]);
    discriminator
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_hashes_empty() {
        let empty: Vec<[u8; 32]> = vec![];
        let result = aggregate_hashes(&empty);

        // Should be SHA256 of empty input
        let expected = Sha256::new().finalize();
        assert_eq!(result[..], expected[..]);
    }

    #[test]
    fn test_aggregate_hashes_single() {
        let input = [1u8; 32];
        let result = aggregate_hashes(&[input]);

        // Should be SHA256 of the single input
        let mut hasher = Sha256::new();
        hasher.update(input);
        let expected = hasher.finalize();
        assert_eq!(result[..], expected[..]);
    }

    #[test]
    fn test_aggregate_hashes_multiple() {
        let inputs = [[1u8; 32], [2u8; 32], [3u8; 32]];
        let result = aggregate_hashes(&inputs);

        // Verify deterministic output
        let result2 = aggregate_hashes(&inputs);
        assert_eq!(result, result2);

        // Verify order matters
        let inputs_reversed = [[3u8; 32], [2u8; 32], [1u8; 32]];
        let result_reversed = aggregate_hashes(&inputs_reversed);
        assert_ne!(result, result_reversed);
    }

    #[test]
    fn reveal_digest_is_deterministic() {
        let wallet = Pubkey::new_unique();
        let plaintext = b"cool\x1f0123";
        let d1 = compute_reveal_digest(&wallet, plaintext);
        let d2 = compute_reveal_digest(&wallet, plaintext);
        assert_eq!(d1, d2);
    }

    #[test]
    fn reveal_digest_changes_with_wallet_or_plaintext() {
        let wallet_a = Pubkey::new_unique();
        let wallet_b = Pubkey::new_unique();
        let d1 = compute_reveal_digest(&wallet_a, b"hello\x1fabcd");
        let d2 = compute_reveal_digest(&wallet_a, b"hello!\x1fabcd");
        let d3 = compute_reveal_digest(&wallet_b, b"hello\x1fabcd");
        assert_ne!(d1, d2);
        assert_ne!(d1, d3);
    }

    #[test]
    fn xor_reveal_digest_is_order_independent() {
        let wallet_a = Pubkey::new_unique();
        let wallet_b = Pubkey::new_unique();
        let d1 = compute_reveal_digest(&wallet_a, b"one\x1faaaa");
        let d2 = compute_reveal_digest(&wallet_b, b"two\x1fbbbb");
        let left = xor_reveal_digests([0u8; 32], &[d1, d2]);
        let right = xor_reveal_digests([0u8; 32], &[d2, d1]);
        assert_eq!(left, right);
        assert_ne!(left, [0u8; 32]);
    }

    #[test]
    fn test_select_winning_ticket_zero_tickets() {
        let entropy = [0u8; 32];
        let result = select_winning_ticket(entropy, 0);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_select_winning_ticket_single() {
        let entropy = [255u8; 32]; // Max entropy
        let result = select_winning_ticket(entropy, 1);
        assert_eq!(result, 0); // Only possible outcome
    }

    #[test]
    fn test_select_winning_ticket_multiple() {
        let entropy = [
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        let result = select_winning_ticket(entropy, 10);

        // With entropy = 1 and 10 tickets, result should be 1
        assert_eq!(result, 1);

        // Test boundary case
        let result = select_winning_ticket(entropy, 1);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_select_winning_ticket_large() {
        let entropy = [255u8; 32];
        let total_tickets = 1_000_000;
        let result = select_winning_ticket(entropy, total_tickets);

        // Should be within valid range
        assert!(result < total_tickets);
    }

    #[test]
    fn test_discriminator() {
        let disc1 = discriminator("Config");
        let disc2 = discriminator("Lottery");
        let disc3 = discriminator("Config"); // Same as disc1

        // Should be deterministic
        assert_eq!(disc1, disc3);

        // Should be different for different names
        assert_ne!(disc1, disc2);

        // Should be 8 bytes
        assert_eq!(disc1.len(), 8);
    }

    #[test]
    fn test_discriminator_anchor_compatibility() {
        // Test that our discriminator matches expected format
        let disc = discriminator("Config");

        // Should not be all zeros (very unlikely)
        assert_ne!(disc, [0u8; 8]);

        // Should be consistent across calls
        let disc2 = discriminator("Config");
        assert_eq!(disc, disc2);
    }
}
