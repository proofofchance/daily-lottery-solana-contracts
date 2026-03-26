//! # Config Account State
//!
//! The Config account stores global lottery system configuration and is the root authority
//! for all lottery operations. It uses PDA seeds `["config"]`.
//!
//! ## Key Features
//! - Authority management for lottery operations
//! - Ticket pricing configuration (immutable after init)
//! - Service charge settings (updatable by authority)
//! - Active lottery tracking for single-lottery constraint
//! - Lottery counter for unique ID generation

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Global configuration for the daily lottery system
///
/// This account is created once during program initialization and stores
/// system-wide settings that govern all lottery operations.
///
/// ## PDA Seeds
/// `["config"]`
///
/// ## Authority Model
/// Only the `authority` pubkey can:
/// - Create new lotteries
/// - Update service charge rates
/// - Adjust reveal windows (emergency use)
/// - Settle lotteries
#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone)]
pub struct Config {
    /// The authority pubkey that can perform administrative operations
    /// Set during initialization and cannot be changed
    pub authority: Pubkey,

    /// Price per lottery ticket in lamports
    /// Set during initialization and cannot be changed to ensure fairness
    pub ticket_price_lamports: u64,

    /// Service charge in basis points (0-9999, where 10000 = 100%)
    /// Can be updated by authority to adjust platform fees
    pub service_charge_bps: u16,

    /// Total number of lotteries created (used for unique ID generation)
    /// Incremented each time a new lottery is created
    pub lottery_count: u64,

    /// Default buy window length in seconds (e.g., 24h)
    pub buy_window_secs: u32,

    /// Default upload window length in seconds (e.g., 24h)
    pub upload_window_secs: u32,

    /// Upper bound for winners count to size on-chain bitmap allocation
    /// Used to pre-allocate sufficient space in the Lottery account at creation time
    pub max_winners_cap: u32,
}

impl Config {
    /// Validates that the service charge is within acceptable bounds
    pub fn validate_service_charge(bps: u16) -> bool {
        bps < 10_000 // Must be less than 100%
    }

    /// Increments the lottery count and returns the new lottery ID
    pub fn next_lottery_id(&mut self) -> Result<u64, crate::error::Error> {
        self.lottery_count = self
            .lottery_count
            .checked_add(1)
            .ok_or(crate::error::Error::MathOverflow)?;
        Ok(self.lottery_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_service_charge() {
        assert!(Config::validate_service_charge(0));
        assert!(Config::validate_service_charge(500)); // 5%
        assert!(Config::validate_service_charge(9999)); // 99.99%
        assert!(!Config::validate_service_charge(10000)); // 100%
        assert!(!Config::validate_service_charge(15000)); // 150%
    }

    // lifecycle constraints removed; multiple concurrent lotteries supported
}
