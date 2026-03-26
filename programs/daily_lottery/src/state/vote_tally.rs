//! Vote tally account for UploadReveals batching.
//!
//! Tracks weighted votes and earliest attestation timestamps so the selected
//! winner count is deterministic and batch-order independent.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone)]
pub struct VoteTally {
    /// Lottery this tally belongs to
    pub lottery: Pubkey,
    /// Maximum winners count tracked by this tally
    pub max_winners: u64,
    /// Total attested participants expected
    pub total_attested: u64,
    /// Number of participants processed so far
    pub processed_count: u64,
    /// Weighted vote totals by count index (1..=max_winners)
    pub weights: Vec<u128>,
    /// Earliest attestation time per count (1..=max_winners)
    pub first_seen: Vec<i64>,
}

impl VoteTally {
    /// Computes required account size for a given max winners cap.
    pub fn account_size_for(max_winners: usize) -> usize {
        let len = max_winners.saturating_add(1);
        // discriminator 8 + lottery[32] + max_winners[8] + total_attested[8] + processed_count[8]
        // + weights vec header[4] + len*16 + first_seen vec header[4] + len*8
        8 + 32 + 8 + 8 + 8 + 4 + (len * 16) + 4 + (len * 8)
    }

    /// Initialize a new tally with zeroed weights and max timestamps.
    pub fn new(lottery: Pubkey, max_winners: u64, total_attested: u64) -> Self {
        let len = (max_winners as usize).saturating_add(1);
        Self {
            lottery,
            max_winners,
            total_attested,
            processed_count: 0,
            weights: vec![0u128; len],
            first_seen: vec![i64::MAX; len],
        }
    }

    /// Add a weighted vote and update earliest attestation timestamp.
    pub fn add_vote(&mut self, count: u64, weight: u128, attested_at: i64) {
        if count == 0 || count > self.max_winners {
            return;
        }
        let idx = count as usize;
        if idx >= self.weights.len() {
            return;
        }
        self.weights[idx] = self.weights[idx].saturating_add(weight);
        if attested_at < self.first_seen[idx] {
            self.first_seen[idx] = attested_at;
        }
    }

    /// Compute selected winners count from the current tally.
    pub fn selected_winners(&self, participants_count: u64) -> u64 {
        if participants_count <= 1 {
            return 1;
        }

        let max_count = self.max_winners.min(participants_count.saturating_sub(1));

        let mut best_count = 1u64;
        let mut best_weight = 0u128;
        let mut best_time = i64::MAX;

        for count in 1..=max_count {
            let idx = count as usize;
            if idx >= self.weights.len() {
                break;
            }
            let weight = self.weights[idx];
            if weight == 0 {
                continue;
            }
            let time = self.first_seen.get(idx).copied().unwrap_or(i64::MAX);
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
}
