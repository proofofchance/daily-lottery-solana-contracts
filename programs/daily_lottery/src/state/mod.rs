//! # ProofOfChance Daily Lottery - State Module
//!
//! This module contains all the account state definitions for the daily lottery program.
//! These structures represent the on-chain data that persists across transactions.
//!
//! ## Account Types
//!
//! - [`Config`]: Global lottery configuration and authority management
//! - [`Lottery`]: Individual lottery instance state and metadata  
//! - [`Participant`]: Participant data including tickets and proof-of-chance
//! - [`VoteTally`]: Vote weights for winner count selection across reveal batches
//! - [`Vault`]: Custody account for lottery funds
//!
//! ## PDA Seed Patterns
//!
//! All accounts use Program Derived Addresses (PDAs) with the following seed patterns:
//! - Config: `["config"]`
//! - Lottery: `["lottery", config_pubkey, lottery_id_le_bytes]`
//! - Participant: `["participant", lottery_pubkey, wallet_pubkey]`
//! - Vault: `["vault", lottery_pubkey]`

pub mod config;
pub mod lottery;
pub mod participant;
pub mod vault;
pub mod vote_tally;
pub mod winners_ledger;

pub use config::*;
pub use lottery::*;
pub use participant::*;
pub use vault::*;
pub use vote_tally::*;
pub use winners_ledger::*;

/// Size constants for account allocation
/// These sizes match the Borsh serialization format (packed, no padding)
pub mod sizes {
    /// Maximum number of winners supported (affects lottery account size)
    pub const MAX_WINNERS: usize = 256;
    /// Size of Config account in bytes (discriminator + data)
    /// authority[32] + ticket_price_lamports[8] + service_charge_bps[2] + lottery_count[8] + buy_window_secs[4] + upload_window_secs[4] + max_winners_cap[4]
    pub const CONFIG_SIZE: usize = 8 + 32 + 8 + 2 + 8 + 4 + 4 + 4;

    /// Size of Lottery account in bytes (discriminator + data)  
    /// id[8] + config[32] + authority[32] + created_at_unix[8]
    /// + buy_start_unix[8] + buy_deadline_unix[8]
    /// + upload_start_unix[8] + upload_deadline_unix[8]
    /// + settlement_start_unix[8]
    /// + status[1] + total_tickets[8] + total_funds[8]
    /// + provider_uploaded_count[8] + poc_aggregate_hash[32] + uploads_complete[1]
    /// + settled[1] + vault[32] + vault_bump[1]
    /// + attested_count[8] + participants_count[8] + selected_number_of_winners[8]
    /// + winners_merkle_root[32] + winners_count[8] + total_payout[8]
    /// + paid_winners_bitmap[4+32] + settlement_batches_completed[4] + settlement_complete[1]
    pub const LOTTERY_SIZE: usize = 8
        + 8
        + 32
        + 32
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 1
        + 8
        + 8
        + 8
        + 32
        + 1
        + 1
        + 32
        + 1
        + 8
        + 8
        + 8
        + 32
        + 8
        + 8
        + (4 + MAX_WINNERS.div_ceil(8))
        + 4
        + 1;

    /// Size of Participant account in bytes (discriminator + data)
    /// lottery[32] + wallet[32] + reveal_hash[32] + tickets[8] + attested[1]
    /// + attested_at_unix[8] + voted_number_of_winners[8] + reveal_score[8]
    pub const PARTICIPANT_SIZE: usize = 8 + 32 + 32 + 32 + 8 + 1 + 8 + 8 + 8;

    /// Size of Vault account in bytes (discriminator + data)
    /// lottery[32] + bump[1]
    pub const VAULT_SIZE: usize = 8 + 32 + 1;
}
