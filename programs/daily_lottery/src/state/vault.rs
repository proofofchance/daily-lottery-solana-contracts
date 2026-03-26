//! # Vault Account State
//!
//! The Vault account serves as a custody account for lottery funds. Each lottery
//! has its own dedicated vault that holds all ticket sale proceeds until settlement.
//!
//! ## Security Model
//! - Vault is a PDA owned by the program
//! - Only the program can transfer funds out during settlement
//! - Funds are distributed to winner and authority based on service charge

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Vault state for lottery fund custody
///
/// Each lottery has a dedicated vault account that holds all the lamports
/// collected from ticket sales. The vault is a PDA that can only be controlled
/// by the program itself, ensuring secure custody of funds.
///
/// ## PDA Seeds
/// `["vault", lottery_pubkey]`
///
/// ## Fund Flow
/// 1. Participants transfer lamports to vault during ticket purchases
/// 2. Funds accumulate in vault throughout lottery lifecycle
/// 3. During settlement, program transfers funds out:
///    - Service fee to authority wallet
///    - Remaining amount to winner wallet
#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone)]
pub struct Vault {
    /// The lottery that owns this vault
    pub lottery: Pubkey,

    /// PDA bump seed for this vault account
    /// Stored for efficient signing during fund transfers
    pub bump: u8,
}

impl Vault {
    /// Creates a new vault for the specified lottery
    pub fn new(lottery: Pubkey, bump: u8) -> Self {
        Self { lottery, bump }
    }

    /// Gets the PDA seeds for this vault (used for signing)
    /// Returns a vector of seed slices that can be used for signing
    pub fn get_seeds(&self) -> Vec<&[u8]> {
        vec![
            b"vault",
            self.lottery.as_ref(),
            std::slice::from_ref(&self.bump),
        ]
    }

    /// Validates that this vault belongs to the specified lottery
    pub fn validate_lottery(&self, expected_lottery: &Pubkey) -> bool {
        self.lottery == *expected_lottery
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn test_vault_creation() {
        let lottery = Pubkey::new_unique();
        let bump = 255;

        let vault = Vault::new(lottery, bump);

        assert_eq!(vault.lottery, lottery);
        assert_eq!(vault.bump, bump);
    }

    #[test]
    fn test_vault_validation() {
        let lottery1 = Pubkey::new_unique();
        let lottery2 = Pubkey::new_unique();
        let vault = Vault::new(lottery1, 255);

        assert!(vault.validate_lottery(&lottery1));
        assert!(!vault.validate_lottery(&lottery2));
    }

    #[test]
    fn test_seeds_generation() {
        let lottery = Pubkey::new_unique();
        let bump = 200;
        let vault = Vault::new(lottery, bump);

        let seeds = vault.get_seeds();
        assert_eq!(seeds[0], b"vault");
        assert_eq!(seeds[1], lottery.as_ref());
        assert_eq!(seeds[2], &[bump]);
    }
}
