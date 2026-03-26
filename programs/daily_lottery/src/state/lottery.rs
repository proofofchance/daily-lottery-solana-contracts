//! # Lottery Account State
//!
//! The Lottery account represents an individual daily lottery instance with all its
//! state data including timing, participants, funds, and settlement status.
//!
//! ## Lifecycle Phases
//! 1. **Buy**: Accepting ticket purchases (window)
//! 2. **Upload**: Participants upload PoC to provider and vote-and-attest on-chain (window)
//! 3. **Settlement**: Provider uploads proofs in batches and settlement executes (no deadline)
//!
//! ## Timing Model
//! - `created_at_unix`: When lottery was created
//! - `buy_start_unix` and `buy_deadline_unix`: Buy window
//! - `upload_start_unix` and `upload_deadline_unix`: Upload window (attestation occurs inside this window)
//! - `settlement_start_unix`: When settlement began (0 if not started)

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Status of a lottery instance (kept minimal; phases derived from timers)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[borsh(use_discriminant = true)]
pub enum LotteryStatus {
    /// Lottery is active (not settled). Phase is derived from timers.
    #[default]
    Active = 0,
    /// Lottery is closed and settled
    Settled = 1,
}

impl From<u8> for LotteryStatus {
    fn from(value: u8) -> Self {
        match value {
            0 => LotteryStatus::Active,
            1 => LotteryStatus::Settled,
            _ => LotteryStatus::Active,
        }
    }
}

impl From<LotteryStatus> for u8 {
    fn from(status: LotteryStatus) -> Self {
        status as u8
    }
}

/// Individual lottery instance state
///
/// Each lottery is a unique instance with its own participants, funds, and settlement.
/// The lottery follows a strict lifecycle from creation through settlement.
///
/// ## PDA Seeds
/// `["lottery", config_pubkey, lottery_id_le_bytes]`
///
/// ## Process
/// 1. Participants buy tickets during buy window
/// 2. Upload window opens; participants upload PoC to provider and vote-and-attest on-chain
/// 3. Settlement begins after upload window (or when all have attested); provider uploads PoCs in batches
/// 4. Lottery is settled using deterministic reveal-plaintext draw from reveal-included commitments
#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone)]
pub struct Lottery {
    /// Unique lottery ID (sequential, starting from 1)
    pub id: u64,

    /// Reference to the Config account that created this lottery
    pub config: Pubkey,

    /// Authority that can manage this lottery (copied from config for efficiency)
    pub authority: Pubkey,

    /// Unix timestamp when this lottery was created
    pub created_at_unix: i64,

    /// Buy window
    pub buy_start_unix: i64,
    pub buy_deadline_unix: i64,

    /// Upload window (attestation occurs within this window)
    pub upload_start_unix: i64,
    pub upload_deadline_unix: i64,

    /// When settlement began (0 if not started)
    pub settlement_start_unix: i64,

    /// Current status of the lottery
    pub status: u8, // Stored as u8 for Borsh compatibility

    /// Total number of tickets sold across all participants
    pub total_tickets: u64,

    /// Total funds collected from ticket sales (in lamports)
    pub total_funds: u64,

    /// Count of provider-uploaded proofs during settlement
    pub provider_uploaded_count: u64,

    /// Aggregate hash of all uploaded proofs (used for entropy)
    pub poc_aggregate_hash: [u8; 32],

    /// Whether all uploads required for settlement are complete
    pub uploads_complete: bool,

    /// Whether the lottery has been settled (winner paid, funds distributed)
    pub settled: bool,

    /// Vault account that holds the lottery funds
    pub vault: Pubkey,

    /// Bump seed for the vault PDA
    pub vault_bump: u8,

    /// Number of participants who have attested their reveal uploads
    pub attested_count: u64,

    /// Total number of unique participants (wallets) who bought at least one ticket
    pub participants_count: u64,

    /// The selected number of winners for this lottery, computed after reveals upload
    /// Defaults to 1 and must satisfy 1 <= selected_number_of_winners <= participants_count - 1
    pub selected_number_of_winners: u64,

    // === NEW SETTLEMENT FIELDS ===
    /// Merkle root of all winners (recipient, amount) pairs
    pub winners_merkle_root: [u8; 32],

    /// Total number of winners determined after settlement
    pub winners_count: u64,

    /// Total payout amount to all winners (in lamports)
    pub total_payout: u64,

    /// Packed bitmap tracking which winners have been paid (bit index = winner index)
    pub paid_winners_bitmap: Vec<u8>,

    /// Number of settlement batches completed
    pub settlement_batches_completed: u32,

    /// Whether all winners have been paid and settlement is complete
    pub settlement_complete: bool,
}

impl Lottery {
    /// Creates a new lottery with the given parameters
    pub fn new(
        id: u64,
        config: Pubkey,
        authority: Pubkey,
        created_at_unix: i64,
        vault: Pubkey,
        vault_bump: u8,
    ) -> Self {
        let (buy_start_unix, buy_deadline_unix, upload_start_unix, upload_deadline_unix) =
            Self::calculate_windows(created_at_unix);

        Self {
            id,
            config,
            authority,
            created_at_unix,
            buy_start_unix,
            buy_deadline_unix,
            upload_start_unix,
            upload_deadline_unix,
            settlement_start_unix: 0,
            status: LotteryStatus::Active.into(),
            total_tickets: 0,
            total_funds: 0,
            provider_uploaded_count: 0,
            poc_aggregate_hash: [0; 32],
            uploads_complete: false,
            settled: false,
            vault,
            vault_bump,
            attested_count: 0,
            participants_count: 0,
            selected_number_of_winners: 1,

            // Initialize new settlement fields
            winners_merkle_root: [0; 32],
            winners_count: 0,
            total_payout: 0,
            // Pre-allocate bitmap to maximum size to keep serialized size constant
            paid_winners_bitmap: vec![0u8; crate::state::sizes::MAX_WINNERS.div_ceil(8)],
            settlement_batches_completed: 0,
            settlement_complete: false,
        }
    }

    /// Calculates buy and upload window timing based on creation time
    fn calculate_windows(created_at_unix: i64) -> (i64, i64, i64, i64) {
        // Production mode defaults:
        // - Buy window: [created_at, created_at + 24h)
        // - Upload window: [created_at + 24h, created_at + 48h)
        // BeginUploadPhase can fast-track by setting upload_start_unix = now
        let buy_start = created_at_unix;
        let buy_deadline = created_at_unix + 24 * 60 * 60; // 24 hours buy window
        let upload_start = created_at_unix + 24 * 60 * 60; // default: start after buy ends
        let upload_deadline = created_at_unix + 48 * 60 * 60; // default: end 24h after upload begins
        (buy_start, buy_deadline, upload_start, upload_deadline)
    }

    /// Gets the current lottery status as enum
    pub fn get_status(&self) -> LotteryStatus {
        LotteryStatus::from(self.status)
    }

    /// Sets the lottery status
    pub fn set_status(&mut self, status: LotteryStatus) {
        self.status = status.into();
    }

    /// Checks if the lottery is currently active (not settled)
    pub fn is_active(&self) -> bool {
        self.get_status() == LotteryStatus::Active && !self.settled
    }

    /// Checks if we're currently in the buy window
    pub fn is_in_buy_window(&self, current_time: i64) -> bool {
        if self.upload_start_unix > 0 && current_time >= self.upload_start_unix {
            return false;
        }
        self.buy_start_unix > 0
            && current_time >= self.buy_start_unix
            && current_time < self.buy_deadline_unix
    }

    /// Checks if we're currently in the upload window (attestation period)
    pub fn is_in_upload_window(&self, current_time: i64) -> bool {
        self.upload_start_unix > 0
            && current_time >= self.upload_start_unix
            && current_time <= self.upload_deadline_unix
    }

    /// Computes the current phase of the lottery statelessly based on timestamps and state
    ///
    /// Phases:
    /// - "buy": current_time <= buy_deadline_unix
    /// - "upload": buy_deadline_unix < current_time <= upload_deadline_unix (and upload_start_unix > 0)
    /// - "settlement": settlement_start_unix > 0 OR upload_deadline_unix < current_time (and not settled)
    /// - "settled": settled flag is true OR single-participant auto-settle rule applies
    ///
    /// Single-participant auto-settle rule:
    /// If participants_count == 1 and current_time > upload_deadline_unix, treat as settled
    /// even if the on-chain settled bit hasn't been explicitly set yet.
    pub fn phase(&self, current_time: i64) -> &'static str {
        // Check explicit settled flag first
        if self.settled {
            return "settled";
        }

        // Single-participant auto-settle rule
        if self.participants_count == 1
            && self.upload_deadline_unix > 0
            && current_time > self.upload_deadline_unix
        {
            return "settled";
        }

        // If settlement has explicitly begun, treat as settlement even if upload window is open
        if self.settlement_start_unix > 0 {
            return "settlement";
        }

        // Standard phase derivation
        if self.upload_start_unix > 0
            && current_time >= self.upload_start_unix
            && current_time <= self.upload_deadline_unix
        {
            return "upload";
        }

        if current_time <= self.buy_deadline_unix {
            return "buy";
        }

        "settlement"
    }

    /// Checks if the lottery is effectively settled (either explicitly or via stateless rules)
    pub fn is_effectively_settled(&self, current_time: i64) -> bool {
        self.phase(current_time) == "settled"
    }

    /// Adds tickets to the lottery (called during ticket purchase)
    pub fn add_tickets(&mut self, count: u64, lamports: u64) -> Result<(), crate::error::Error> {
        self.total_tickets = self
            .total_tickets
            .checked_add(count)
            .ok_or(crate::error::Error::MathOverflow)?;

        self.total_funds = self
            .total_funds
            .checked_add(lamports)
            .ok_or(crate::error::Error::MathOverflow)?;

        Ok(())
    }

    /// Increments the attestation count
    pub fn add_attestation(&mut self) -> Result<(), crate::error::Error> {
        self.attested_count = self
            .attested_count
            .checked_add(1)
            .ok_or(crate::error::Error::MathOverflow)?;
        Ok(())
    }

    /// Marks uploads as complete with aggregate PoC hash
    pub fn mark_uploads_complete(&mut self, aggregate_hash: [u8; 32]) {
        self.uploads_complete = true;
        self.poc_aggregate_hash = aggregate_hash;
    }

    /// Marks the lottery as settled
    pub fn settle(&mut self) {
        self.settled = true;
        self.set_status(LotteryStatus::Settled);
    }

    /// Adjusts the upload window (emergency use by authority)
    pub fn adjust_upload_window(
        &mut self,
        new_start: i64,
        new_deadline: i64,
    ) -> Result<(), crate::error::Error> {
        if new_start == 0 || new_deadline <= new_start {
            return Err(crate::error::Error::InvalidInstruction);
        }

        self.upload_start_unix = new_start;
        self.upload_deadline_unix = new_deadline;
        Ok(())
    }

    /// Increments the participant count (called when a new participant account is created)
    pub fn add_participant(&mut self) -> Result<(), crate::error::Error> {
        self.participants_count = self
            .participants_count
            .checked_add(1)
            .ok_or(crate::error::Error::MathOverflow)?;
        Ok(())
    }

    /// Sets the selected number of winners after reveals processing
    pub fn set_selected_winners(&mut self, count: u64) -> Result<(), crate::error::Error> {
        // Basic validation: at least 1 winner and at most participants - 1
        if count == 0 {
            return Err(crate::error::Error::InvalidInstruction);
        }
        if self.participants_count <= 1 {
            return Err(crate::error::Error::InvalidInstruction);
        }
        if count > self.participants_count.saturating_sub(1) {
            return Err(crate::error::Error::InvalidInstruction);
        }
        self.selected_number_of_winners = count;
        Ok(())
    }

    // === NEW SETTLEMENT METHODS ===

    /// Initializes settlement with winners merkle root and metadata
    pub fn initialize_settlement(
        &mut self,
        winners_merkle_root: [u8; 32],
        winners_count: u64,
        total_payout: u64,
    ) -> Result<(), crate::error::Error> {
        if self.settlement_complete {
            return Err(crate::error::Error::LotteryAlreadySettled);
        }

        // Validate winners count doesn't exceed maximum
        if winners_count > crate::state::sizes::MAX_WINNERS as u64 {
            return Err(crate::error::Error::InvalidInstruction);
        }

        self.winners_merkle_root = winners_merkle_root;
        self.winners_count = winners_count;
        self.total_payout = total_payout;

        // Bitmap is already pre-allocated to maximum size, just clear the used portion
        let bytes_needed = winners_count.div_ceil(8) as usize;
        for i in 0..bytes_needed {
            self.paid_winners_bitmap[i] = 0;
        }

        Ok(())
    }

    /// Checks if a winner has been paid
    pub fn is_winner_paid(&self, winner_index: u64) -> bool {
        if winner_index >= self.winners_count {
            return false;
        }

        let byte_index = (winner_index / 8) as usize;
        let bit_index = winner_index % 8;

        if byte_index >= self.paid_winners_bitmap.len() {
            return false;
        }

        (self.paid_winners_bitmap[byte_index] >> bit_index) & 1 == 1
    }

    /// Marks a winner as paid
    pub fn mark_winner_paid(&mut self, winner_index: u64) -> Result<(), crate::error::Error> {
        if winner_index >= self.winners_count {
            return Err(crate::error::Error::InvalidInstruction);
        }

        let byte_index = (winner_index / 8) as usize;
        let bit_index = winner_index % 8;

        // Extend bitmap if needed (shouldn't happen if initialized correctly)
        while byte_index >= self.paid_winners_bitmap.len() {
            self.paid_winners_bitmap.push(0);
        }

        self.paid_winners_bitmap[byte_index] |= 1 << bit_index;
        Ok(())
    }

    /// Increments the settlement batches completed counter
    pub fn increment_settlement_batch(&mut self) {
        self.settlement_batches_completed = self.settlement_batches_completed.saturating_add(1);
    }

    /// Checks if all winners have been paid
    pub fn all_winners_paid(&self) -> bool {
        if self.winners_count == 0 {
            return true;
        }

        // Check if all bits are set for the number of winners
        for i in 0..self.winners_count {
            if !self.is_winner_paid(i) {
                return false;
            }
        }
        true
    }

    /// Completes the settlement process
    pub fn complete_settlement(&mut self) -> Result<(), crate::error::Error> {
        if !self.all_winners_paid() {
            return Err(crate::error::Error::InvalidInstruction);
        }

        self.settlement_complete = true;
        self.settle(); // Mark as settled using existing method
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn test_lottery_creation() {
        let config = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let vault = Pubkey::new_unique();
        let created_at = 1000;

        let lottery = Lottery::new(1, config, authority, created_at, vault, 255);

        assert_eq!(lottery.id, 1);
        assert_eq!(lottery.config, config);
        assert_eq!(lottery.authority, authority);
        assert_eq!(lottery.created_at_unix, created_at);
        assert_eq!(lottery.vault, vault);
        assert_eq!(lottery.vault_bump, 255);
        assert!(lottery.is_active());
        assert!(!lottery.settled);
    }

    #[test]
    fn test_ticket_addition() {
        let mut lottery = Lottery::default();

        lottery.add_tickets(5, 5000).unwrap();
        assert_eq!(lottery.total_tickets, 5);
        assert_eq!(lottery.total_funds, 5000);

        lottery.add_tickets(3, 3000).unwrap();
        assert_eq!(lottery.total_tickets, 8);
        assert_eq!(lottery.total_funds, 8000);
    }

    #[test]
    fn test_windows() {
        let current_time = 1000;

        let mut lottery = Lottery::new(
            1,
            Pubkey::default(),
            Pubkey::default(),
            current_time,
            Pubkey::default(),
            0,
        );
        assert!(lottery.is_in_buy_window(current_time)); // Buy starts immediately
        assert!(lottery.is_in_buy_window(current_time + 12 * 60 * 60)); // within 24h buy window
        assert!(!lottery.is_in_buy_window(lottery.buy_deadline_unix)); // buy deadline is exclusive
        assert!(!lottery.is_in_buy_window(current_time + 25 * 60 * 60)); // after buy window

        // Upload start hard-stops buys even if buy deadline has not elapsed.
        lottery.upload_start_unix = current_time + 60;
        assert!(!lottery.is_in_buy_window(current_time + 60));
    }

    #[test]
    fn test_stateless_phase_computation() {
        let mut lottery = Lottery::new(
            1,
            Pubkey::default(),
            Pubkey::default(),
            1000,
            Pubkey::default(),
            0,
        );

        // Set up windows for testing
        lottery.buy_start_unix = 1000;
        lottery.buy_deadline_unix = 2000;
        lottery.upload_start_unix = 2000;
        lottery.upload_deadline_unix = 3000;

        // Buy phase
        assert_eq!(lottery.phase(1500), "buy");
        assert_eq!(lottery.phase(2000), "upload"); // boundary belongs to upload

        // Upload phase
        assert_eq!(lottery.phase(2500), "upload");
        assert_eq!(lottery.phase(3000), "upload"); // at deadline

        // Early settlement when settlement_start_unix is set
        lottery.settlement_start_unix = 2400;
        assert_eq!(lottery.phase(2500), "settlement");
        assert_eq!(lottery.phase(3000), "settlement");
        lottery.settlement_start_unix = 0;

        // Settlement phase
        assert_eq!(lottery.phase(3500), "settlement");

        // Explicit settled
        lottery.settle();
        assert_eq!(lottery.phase(3500), "settled");
    }

    #[test]
    fn test_single_participant_auto_settle() {
        let mut lottery = Lottery::new(
            1,
            Pubkey::default(),
            Pubkey::default(),
            1000,
            Pubkey::default(),
            0,
        );

        // Set up windows and single participant
        lottery.buy_start_unix = 1000;
        lottery.buy_deadline_unix = 2000;
        lottery.upload_start_unix = 2000;
        lottery.upload_deadline_unix = 3000;
        lottery.participants_count = 1;

        // Before upload deadline - normal phases
        assert_eq!(lottery.phase(1500), "buy");
        assert_eq!(lottery.phase(2500), "upload");

        // After upload deadline with single participant - auto-settle
        assert_eq!(lottery.phase(3500), "settled");
        assert!(lottery.is_effectively_settled(3500));

        // Multiple participants should not auto-settle
        lottery.participants_count = 2;
        assert_eq!(lottery.phase(3500), "settlement");
        assert!(!lottery.is_effectively_settled(3500));
    }
}
