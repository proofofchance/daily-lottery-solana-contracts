//! # Event Emission Module
//!
//! This module defines structured events that are emitted by the daily lottery program.
//! Events are emitted as JSON-formatted log messages that can be parsed by indexers
//! for real-time updates and historical tracking.
//!
//! ## Event Design Principles
//!
//! 1. **Deterministic**: Each instruction emits exactly one semantic event
//! 2. **Complete**: Events contain all data needed for state reconstruction
//! 3. **Parseable**: JSON format for easy parsing by off-chain indexers
//! 4. **Versioned**: Events include version for future compatibility

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use solana_program::msg;

/// Winner's lucky word entry for WinnersLuckyWords event
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct WinnerLuckyWord {
    pub wallet: String,
    pub lucky_words: String,
}

/// Version identifier for event schema
pub const EVENT_VERSION: &str = "1.0.0";

/// All possible events emitted by the daily lottery program
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(tag = "event_type", content = "data")]
pub enum LotteryEvent {
    /// Emitted when the lottery system is initialized
    SystemInitialized {
        authority: String,
        config: String,
        ticket_price_lamports: u64,
        service_charge_bps: u16,
        timestamp: i64,
    },

    /// Emitted when service charge is updated
    ServiceChargeUpdated {
        config: String,
        old_bps: u16,
        new_bps: u16,
        authority: String,
        timestamp: i64,
    },

    /// Emitted when default window durations are updated
    WindowsUpdated {
        config: String,
        old_buy_window_secs: u32,
        new_buy_window_secs: u32,
        old_upload_window_secs: u32,
        new_upload_window_secs: u32,
        authority: String,
        timestamp: i64,
    },

    /// Emitted when a new lottery is created
    LotteryCreated {
        lottery_id: u64,
        lottery: String,
        config: String,
        authority: String,
        vault: String,
        created_at_unix: i64,
        buy_start_unix: i64,
        buy_deadline_unix: i64,
        upload_start_unix: i64,
        upload_deadline_unix: i64,
    },

    /// Emitted when tickets are purchased
    TicketsPurchased {
        lottery_id: u64,
        lottery: String,
        participant: String,
        buyer: String,
        tickets_bought: u64,
        total_tickets_for_participant: u64,
        total_tickets_for_lottery: u64,
        amount_paid: u64,
        total_funds: u64,
        proof_of_chance_hash: Option<[u8; 32]>,
        timestamp: i64,
    },

    /// Emitted when the buy phase begins or is extended
    BuyPhaseBegan {
        lottery_id: u64,
        lottery: String,
        buy_start_unix: i64,
        buy_deadline_unix: i64,
        timestamp: i64,
    },

    /// Emitted when a participant attests to uploading their reveal
    AttestationSubmitted {
        lottery_id: u64,
        lottery: String,
        participant: String,
        wallet: String,
        voted_number_of_winners: u64,
        total_attested: u64,
        timestamp: i64,
    },

    /// Emitted when reveals are uploaded by the service provider
    RevealsUploaded {
        lottery_id: u64,
        lottery: String,
        authority: String,
        participants_count: u64,
        aggregate_hash: [u8; 32],
        selected_number_of_winners: u64,
        timestamp: i64,
    },

    /// Emitted when reveal window is adjusted
    RevealWindowAdjusted {
        lottery_id: u64,
        lottery: String,
        authority: String,
        old_start: i64,
        old_deadline: i64,
        new_start: i64,
        new_deadline: i64,
        timestamp: i64,
    },

    /// Emitted when upload window begins (participants can upload and vote)
    UploadPhaseBegan {
        lottery_id: u64,
        lottery: String,
        new_start: i64,
        new_deadline: i64,
        timestamp: i64,
    },

    /// Emitted when settlement phase begins (no deadline)
    SettlementPhaseBegan {
        lottery_id: u64,
        lottery: String,
        settlement_start_unix: i64,
        timestamp: i64,
    },

    /// Emitted when a lottery is settled
    LotterySettled {
        lottery_id: u64,
        lottery: String,
        vault: String,
        winner: String,
        winning_ticket_index: u64,
        total_tickets: u64,
        total_funds: u64,
        service_fee: u64,
        winner_payout: u64,
        selected_number_of_winners: u64,
        authority: String,
        timestamp: i64,
        // Multi-winner support fields
        winners: Vec<String>,
        per_winner_payout: u64,
    },

    /// Emitted once per winner within the final settlement transaction
    WinnerSettled {
        lottery_id: u64,
        lottery: String,
        /// Winner wallet address (base58)
        winner: String,
        /// Amount paid/credited to this winner (lamports)
        amount: u64,
        timestamp: i64,
    },

    /// Emitted when winners' lucky words are revealed
    WinnersLuckyWords {
        lottery_id: u64,
        lottery: String,
        winners: Vec<WinnerLuckyWord>,
        timestamp: i64,
    },

    /// Emitted when a lottery had no buyers and is concluded
    NoBuyersConcluded {
        lottery_id: u64,
        lottery: String,
        timestamp: i64,
    },

    /// Emitted when refunds are issued (e.g., no attesters)
    RefundsIssued {
        lottery_id: u64,
        lottery: String,
        recipient_count: u64,
        total_refunded_lamports: u64,
        reason: String,
        timestamp: i64,
    },

    /// Emitted when winners are finalized and merkle root is stored
    WinnersFinalized {
        lottery_id: u64,
        lottery: String,
        winners_count: u64,
        total_payout: u64,
        per_winner_payout: u64,
        winners_merkle_root: [u8; 32],
        /// List of winner recipients for merkle proof generation
        winners: Vec<String>,
        timestamp: i64,
    },

    /// Emitted when an individual winner is paid
    WinnerPaid {
        lottery_id: u64,
        lottery: String,
        winner: String,
        amount: u64,
        batch_index: u32,
        winner_index: u64,
        timestamp: i64,
    },

    /// Emitted when all winners have been paid and settlement is complete
    PayoutsComplete {
        lottery_id: u64,
        lottery: String,
        total_winners: u64,
        total_paid: u64,
        batches_completed: u32,
        timestamp: i64,
    },

    /// Emitted when service fee and remainder are paid to authority
    ServiceFeePaid {
        lottery_id: u64,
        lottery: String,
        authority: String,
        service_fee: u64,
        remainder: u64,
        vault_rent_reclaimed: u64,
        timestamp: i64,
    },

    /// Emitted when a reveal is reviewed for transparency
    RevealReviewed {
        lottery_id: u64,
        lottery: String,
        voter: String,
        reveal_index: u64,
        ok: bool,
        reason: Option<String>,
        timestamp: i64,
    },

    /// Emitted before winners are announced with algorithm summary
    WinnersAlgorithmInterlude {
        lottery_id: u64,
        lottery: String,
        report_id: String,
        seed: String,
        rule_version: String,
        total_uploaded: u64,
        total_reviewed: u64,
        eligible_count: u64,
        rejected_count: u64,
        counts_summary: Vec<(String, u64)>, // (lucky_word, count)
        preview: String,
        timestamp: i64,
    },

    /// Emitted when winners are computed with algorithm metadata
    WinnersComputed {
        lottery_id: u64,
        lottery: String,
        seed: String,
        rule_version: String,
        total_eligible: u64,
        winners: Vec<String>,
        timestamp: i64,
    },
}

impl LotteryEvent {
    /// Emit this event as a structured log message
    pub fn emit(&self) {
        let event_wrapper = EventWrapper {
            version: EVENT_VERSION.to_string(),
            program: "daily_lottery".to_string(),
            event: self.clone(),
        };

        // Emit as JSON log that indexers can parse
        if let Ok(json) = serde_json::to_string(&event_wrapper) {
            msg!("LOTTERY_EVENT: {}", json);
        }
    }

    /// Get the lottery ID from any event that has one
    pub fn lottery_id(&self) -> Option<u64> {
        match self {
            LotteryEvent::SystemInitialized { .. } => None,
            LotteryEvent::ServiceChargeUpdated { .. } => None,
            LotteryEvent::WindowsUpdated { .. } => None,
            LotteryEvent::LotteryCreated { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::TicketsPurchased { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::BuyPhaseBegan { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::AttestationSubmitted { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::RevealsUploaded { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::RevealWindowAdjusted { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::UploadPhaseBegan { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::SettlementPhaseBegan { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::LotterySettled { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::WinnerSettled { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::WinnersLuckyWords { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::NoBuyersConcluded { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::RefundsIssued { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::WinnersFinalized { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::WinnerPaid { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::PayoutsComplete { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::ServiceFeePaid { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::RevealReviewed { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::WinnersAlgorithmInterlude { lottery_id, .. } => Some(*lottery_id),
            LotteryEvent::WinnersComputed { lottery_id, .. } => Some(*lottery_id),
        }
    }

    /// Get the timestamp from any event
    pub fn timestamp(&self) -> i64 {
        match self {
            LotteryEvent::SystemInitialized { timestamp, .. } => *timestamp,
            LotteryEvent::ServiceChargeUpdated { timestamp, .. } => *timestamp,
            LotteryEvent::WindowsUpdated { timestamp, .. } => *timestamp,
            LotteryEvent::LotteryCreated {
                created_at_unix, ..
            } => *created_at_unix,
            LotteryEvent::TicketsPurchased { timestamp, .. } => *timestamp,
            LotteryEvent::BuyPhaseBegan { timestamp, .. } => *timestamp,
            LotteryEvent::AttestationSubmitted { timestamp, .. } => *timestamp,
            LotteryEvent::RevealsUploaded { timestamp, .. } => *timestamp,
            LotteryEvent::RevealWindowAdjusted { timestamp, .. } => *timestamp,
            LotteryEvent::UploadPhaseBegan { timestamp, .. } => *timestamp,
            LotteryEvent::SettlementPhaseBegan { timestamp, .. } => *timestamp,
            LotteryEvent::LotterySettled { timestamp, .. } => *timestamp,
            LotteryEvent::WinnerSettled { timestamp, .. } => *timestamp,
            LotteryEvent::WinnersLuckyWords { timestamp, .. } => *timestamp,
            LotteryEvent::NoBuyersConcluded { timestamp, .. } => *timestamp,
            LotteryEvent::RefundsIssued { timestamp, .. } => *timestamp,
            LotteryEvent::WinnersFinalized { timestamp, .. } => *timestamp,
            LotteryEvent::WinnerPaid { timestamp, .. } => *timestamp,
            LotteryEvent::PayoutsComplete { timestamp, .. } => *timestamp,
            LotteryEvent::ServiceFeePaid { timestamp, .. } => *timestamp,
            LotteryEvent::RevealReviewed { timestamp, .. } => *timestamp,
            LotteryEvent::WinnersAlgorithmInterlude { timestamp, .. } => *timestamp,
            LotteryEvent::WinnersComputed { timestamp, .. } => *timestamp,
        }
    }
}

/// Wrapper for events with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventWrapper {
    version: String,
    program: String,
    event: LotteryEvent,
}

/// Helper macro for emitting events with current timestamp
#[macro_export]
macro_rules! emit_event {
    ($event:expr) => {{
        use solana_program::{clock::Clock, sysvar::Sysvar};
        let clock = Clock::get().unwrap_or_default();
        $event.emit();
    }};
}

/// Helper function to create events with current timestamp
pub fn with_current_timestamp<F>(_f: F) -> i64
where
    F: FnOnce(i64) -> LotteryEvent,
{
    use solana_program::{clock::Clock, sysvar::Sysvar};
    let clock = Clock::get().unwrap_or_default();
    clock.unix_timestamp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_serialization() {
        let event = LotteryEvent::LotteryCreated {
            lottery_id: 1,
            lottery: "11111111111111111111111111111111".to_string(),
            config: "11111111111111111111111111111111".to_string(),
            authority: "11111111111111111111111111111111".to_string(),
            vault: "11111111111111111111111111111111".to_string(),
            created_at_unix: 1640995200,
            buy_start_unix: 1640995200,
            buy_deadline_unix: 1641081600,
            upload_start_unix: 1641081600,
            upload_deadline_unix: 1641168000,
        };

        let wrapper = EventWrapper {
            version: EVENT_VERSION.to_string(),
            program: "daily_lottery".to_string(),
            event,
        };

        let json = serde_json::to_string(&wrapper).unwrap();
        assert!(json.contains("LotteryCreated"));
        assert!(json.contains("daily_lottery"));
        assert!(json.contains(EVENT_VERSION));
    }

    #[test]
    fn test_lottery_id_extraction() {
        let event = LotteryEvent::TicketsPurchased {
            lottery_id: 42,
            lottery: "11111111111111111111111111111111".to_string(),
            participant: "11111111111111111111111111111111".to_string(),
            buyer: "11111111111111111111111111111111".to_string(),
            tickets_bought: 5,
            total_tickets_for_participant: 10,
            total_tickets_for_lottery: 100,
            amount_paid: 5000,
            total_funds: 50000,
            proof_of_chance_hash: None,
            timestamp: 1640995200,
        };

        assert_eq!(event.lottery_id(), Some(42));
    }
}
