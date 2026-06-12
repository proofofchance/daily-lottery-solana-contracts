//! # Config Account State
//!
//! The Config account stores global lottery system configuration and is the root authority
//! for all lottery operations. It uses PDA seeds `["config"]`.
//!
//! ## Key Features
//! - Authority management for lottery operations
//! - Ticket pricing configuration (immutable after init)
//! - Service charge settings (updatable by authority)
//! - Lottery counter for unique ID generation across concurrent lotteries

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

    /// Validates the configured winner-count capacity for per-lottery ledgers.
    pub fn validate_max_winners_cap(max_winners_cap: u32) -> bool {
        max_winners_cap > 0 && max_winners_cap <= crate::state::sizes::MAX_WINNERS as u32
    }

    /// Returns the maximum winner count this config can honor for a lottery.
    pub fn effective_max_winners(&self, participants_count: u64) -> u64 {
        if participants_count <= 1 {
            return 1;
        }

        let configured_cap = (self.max_winners_cap as u64)
            .min(crate::state::sizes::MAX_WINNERS as u64)
            .max(1);
        configured_cap.min(participants_count.saturating_sub(1))
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

    #[test]
    fn test_validate_max_winners_cap() {
        assert!(Config::validate_max_winners_cap(1));
        assert!(Config::validate_max_winners_cap(
            crate::state::sizes::MAX_WINNERS as u32
        ));
        assert!(!Config::validate_max_winners_cap(0));
        assert!(!Config::validate_max_winners_cap(
            crate::state::sizes::MAX_WINNERS as u32 + 1
        ));
    }

    #[test]
    fn test_effective_max_winners() {
        let mut config = Config {
            max_winners_cap: 32,
            ..Config::default()
        };
        assert_eq!(config.effective_max_winners(1), 1);
        assert_eq!(config.effective_max_winners(2), 1);
        assert_eq!(config.effective_max_winners(40), 32);

        config.max_winners_cap = 0;
        assert_eq!(config.effective_max_winners(10), 1);
    }

    // lifecycle constraints removed; multiple concurrent lotteries supported
}
