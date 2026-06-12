//! Chunked winner finalization state for daily lottery rounds.
//!
//! The ledger lets FinalizeWinners process participant accounts over many
//! transactions while preserving deterministic weighted draws without
//! replacement.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const FINALIZATION_PHASE_AGGREGATING: u8 = 0;
pub const FINALIZATION_PHASE_SELECTING: u8 = 1;
pub const FINALIZATION_PHASE_COMPLETED: u8 = 2;

#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SelectedWinner {
    pub wallet: Pubkey,
    pub tickets: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone)]
pub struct FinalizationLedger {
    /// Lottery this ledger belongs to.
    pub lottery: Pubkey,
    /// Maximum winner-count vote tracked by this ledger.
    pub max_winners: u64,
    /// Final number of winners selected after aggregation.
    pub target_winners: u64,
    /// Aggregation pass participant count.
    pub processed_count: u64,
    /// Reveal-included participants eligible for winner selection.
    pub eligible_count: u64,
    /// Sum of tickets held by eligible participants.
    pub total_eligible_tickets: u64,
    /// Chunkable commitment over eligible participants in wallet order.
    pub participants_commitment: [u8; 32],
    /// Final seed used for deterministic weighted draws.
    pub seed: [u8; 32],
    /// Current finalization phase.
    pub phase: u8,
    /// Current draw round during winner selection.
    pub current_round: u64,
    /// Participants scanned in the current draw round.
    pub round_processed_count: u64,
    /// Remaining-ticket cumulative cursor for the current draw round.
    pub round_remaining_tickets_seen: u64,
    /// Draw index for the current round.
    pub round_draw_index: u64,
    /// Winner found during the current round.
    pub round_winner_wallet: Pubkey,
    /// Ticket weight of the winner found during the current round.
    pub round_winner_tickets: u64,
    /// Whether the current round has found its winner.
    pub round_winner_found: bool,
    /// Whether `last_processed_wallet` has been initialized for this pass.
    pub has_last_wallet: bool,
    /// Last wallet processed in the current sorted pass.
    pub last_processed_wallet: Pubkey,
    /// Winners already selected, in draw order.
    pub winners: Vec<SelectedWinner>,
    /// Weighted vote totals by winner count, indexed 1..=max_winners.
    pub vote_weights: Vec<u128>,
    /// Earliest attestation timestamp by winner count, indexed 1..=max_winners.
    pub vote_first_seen: Vec<i64>,
    pub started_at_unix: i64,
    pub completed_at_unix: i64,
}

impl FinalizationLedger {
    pub const SELECTED_WINNER_SIZE: usize = 32 + 8;

    pub fn size_for(max_winners: usize) -> usize {
        let vote_len = max_winners.saturating_add(1);
        8 + // discriminator
            32 + // lottery
            8 + // max_winners
            8 + // target_winners
            8 + // processed_count
            8 + // eligible_count
            8 + // total_eligible_tickets
            32 + // participants_commitment
            32 + // seed
            1 + // phase
            8 + // current_round
            8 + // round_processed_count
            8 + // round_remaining_tickets_seen
            8 + // round_draw_index
            32 + // round_winner_wallet
            8 + // round_winner_tickets
            1 + // round_winner_found
            1 + // has_last_wallet
            32 + // last_processed_wallet
            4 + (max_winners * Self::SELECTED_WINNER_SIZE) + // winners vec
            4 + (vote_len * 16) + // vote_weights
            4 + (vote_len * 8) + // vote_first_seen
            8 + // started_at_unix
            8 // completed_at_unix
    }

    pub fn new(lottery: Pubkey, max_winners: u64, started_at_unix: i64) -> Self {
        let vote_len = (max_winners as usize).saturating_add(1);
        Self {
            lottery,
            max_winners,
            target_winners: 0,
            processed_count: 0,
            eligible_count: 0,
            total_eligible_tickets: 0,
            participants_commitment: [0; 32],
            seed: [0; 32],
            phase: FINALIZATION_PHASE_AGGREGATING,
            current_round: 0,
            round_processed_count: 0,
            round_remaining_tickets_seen: 0,
            round_draw_index: 0,
            round_winner_wallet: Pubkey::default(),
            round_winner_tickets: 0,
            round_winner_found: false,
            has_last_wallet: false,
            last_processed_wallet: Pubkey::default(),
            winners: Vec::new(),
            vote_weights: vec![0u128; vote_len],
            vote_first_seen: vec![i64::MAX; vote_len],
            started_at_unix,
            completed_at_unix: 0,
        }
    }

    pub fn reset_pass_cursor(&mut self) {
        self.has_last_wallet = false;
        self.last_processed_wallet = Pubkey::default();
    }

    pub fn begin_selection_round(&mut self, draw_index: u64) {
        self.phase = FINALIZATION_PHASE_SELECTING;
        self.round_processed_count = 0;
        self.round_remaining_tickets_seen = 0;
        self.round_draw_index = draw_index;
        self.round_winner_wallet = Pubkey::default();
        self.round_winner_tickets = 0;
        self.round_winner_found = false;
        self.reset_pass_cursor();
    }

    pub fn add_vote(&mut self, count: u64, weight: u128, attested_at: i64) {
        if count == 0 || count > self.max_winners {
            return;
        }
        let idx = count as usize;
        if idx >= self.vote_weights.len() {
            return;
        }
        self.vote_weights[idx] = self.vote_weights[idx].saturating_add(weight);
        if attested_at < self.vote_first_seen[idx] {
            self.vote_first_seen[idx] = attested_at;
        }
    }

    pub fn selected_winner_count(&self, participants_count: u64) -> u64 {
        if participants_count <= 1 {
            return 1;
        }

        let max_count = self.max_winners.min(participants_count.saturating_sub(1));
        let mut best_count = 1u64;
        let mut best_weight = 0u128;
        let mut best_time = i64::MAX;

        for count in 1..=max_count {
            let idx = count as usize;
            if idx >= self.vote_weights.len() {
                break;
            }
            let weight = self.vote_weights[idx];
            if weight == 0 {
                continue;
            }
            let time = self.vote_first_seen.get(idx).copied().unwrap_or(i64::MAX);
            if weight > best_weight
                || (weight == best_weight && time < best_time)
                || (weight == best_weight && time == best_time && count < best_count)
            {
                best_weight = weight;
                best_time = time;
                best_count = count;
            }
        }

        if best_weight == 0 {
            1
        } else {
            best_count
        }
    }

    pub fn has_selected(&self, wallet: &Pubkey) -> bool {
        self.winners.iter().any(|winner| &winner.wallet == wallet)
    }

    pub fn selected_tickets_total(&self) -> u64 {
        self.winners
            .iter()
            .fold(0u64, |total, winner| total.saturating_add(winner.tickets))
    }

    pub fn remaining_tickets(&self) -> Result<u64, crate::error::Error> {
        self.total_eligible_tickets
            .checked_sub(self.selected_tickets_total())
            .ok_or(crate::error::Error::MathOverflow)
    }

    pub fn push_winner(&mut self, wallet: Pubkey, tickets: u64) -> Result<(), crate::error::Error> {
        if self.has_selected(&wallet) || self.winners.len() >= self.target_winners as usize {
            return Err(crate::error::Error::InvalidInstruction);
        }
        self.winners.push(SelectedWinner { wallet, tickets });
        Ok(())
    }

    pub fn complete(&mut self, timestamp: i64) {
        self.phase = FINALIZATION_PHASE_COMPLETED;
        self.completed_at_unix = timestamp;
        self.reset_pass_cursor();
    }
}
